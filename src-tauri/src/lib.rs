mod ai;
mod config;
mod db;
mod history;
mod meta_commands;
mod error;
mod query_files;
mod schema_info;
mod tunnel;

use std::path::PathBuf;

use config::{AppConfig, ConfigInfo, ConnectionInfo, ServerConfig};
use db::{CancelRegistry, DbManager, DbPool, QueryResult, DEFAULT_MAX_ROWS};
use error::AppError;

/// アプリ全体の共有状態。
#[derive(Default)]
struct AppState {
    /// 接続設定のキャッシュ。get_connections で更新される。
    /// パスワード等の機密を含むためフロントエンドには渡さない。
    servers: tokio::sync::Mutex<Option<Vec<ServerConfig>>>,
    /// default_limit のセッションキャッシュ (Reload config でクリア)。
    default_limit: tokio::sync::Mutex<Option<u64>>,
    /// クエリファイル保存ディレクトリのセッションキャッシュ。
    /// config.yml は手編集されるため、開いているファイルの保存中に
    /// sqlfiles_dir が変わると未保存内容が新ディレクトリへ書かれてしまう。
    /// 再読込 (reset_connections) まで最初に解決した値を使い続けることで、
    /// dirty ファイルの保存先を読み込み時のディレクトリに固定する。
    sqlfiles_dir: tokio::sync::Mutex<Option<PathBuf>>,
    db: DbManager,
    /// 実行中クエリのキャンセルレジストリ (接続名単位)。
    query_cancels: CancelRegistry,
    /// クエリ実行履歴の記録 (接続ごとの行数キャッシュを保持)。
    history: history::HistoryManager,
    /// スキーマ情報 (テーブル・カラム) のキャッシュ。
    /// スキーマブラウザと SQL 補完 (get_schema_map) で共有する。
    schema_cache: schema_info::SchemaCache,
    /// AI 設定のセッションキャッシュ (reset_connections でクリア)。
    /// 外側の None は未解決を表す。api_key を含むためフロントには渡さず、
    /// get_ai_info で configured / model のみを返す。
    ai: tokio::sync::Mutex<Option<Option<ai::AiConfig>>>,
}

impl AppState {
    async fn resolve_default_limit(&self) -> Result<u64, AppError> {
        let mut cached = self.default_limit.lock().await;
        if let Some(limit) = *cached {
            return Ok(limit);
        }
        let limit = AppConfig::load()?.default_limit();
        *cached = Some(limit);
        Ok(limit)
    }

    async fn resolve_sqlfiles_dir(&self) -> Result<PathBuf, AppError> {
        let mut cached = self.sqlfiles_dir.lock().await;
        if let Some(dir) = cached.as_ref() {
            return Ok(dir.clone());
        }
        let dir = AppConfig::load()?.resolve_sqlfiles_dir()?;
        *cached = Some(dir.clone());
        Ok(dir)
    }

    async fn find_server(&self, connection: &str) -> Result<ServerConfig, AppError> {
        let mut servers = self.servers.lock().await;
        if servers.is_none() {
            *servers = Some(AppConfig::load()?.resolve_servers().await?.servers);
        }
        servers
            .as_ref()
            .unwrap()
            .iter()
            .find(|s| s.name == connection)
            .cloned()
            .ok_or_else(|| {
                AppError::Config(format!("Connection '{connection}' is not defined in the config"))
            })
    }

    /// スキーマキャッシュのキーになるアクティブスキーマ名を返す
    /// (オーバーライド > 設定のデフォルト > 空文字)。
    async fn active_schema_key(&self, server: &ServerConfig) -> String {
        match self.db.schema_override(&server.name).await {
            Some(schema) => schema,
            None => server.schema.clone().unwrap_or_default(),
        }
    }

    /// AI 設定を解決する (キャッシュあり)。未設定なら Ok(None)。
    /// ローカル config.yml と接続 YAML (ソース宣言で取得) の両方の
    /// トップレベル `ai:` を見て、接続 YAML 側を優先する。
    /// 解決エラー (不明 provider 等) はキャッシュせず毎回返す
    /// (設定修正 + リロードで直せるように)。
    async fn resolve_ai_config(&self) -> Result<Option<ai::AiConfig>, AppError> {
        let mut cached = self.ai.lock().await;
        if let Some(ai_config) = cached.as_ref() {
            return Ok(ai_config.clone());
        }
        let config = AppConfig::load()?;
        let resolved = config.resolve_servers().await?;
        let ai_config =
            ai::resolve_ai_config(config.local_ai().as_ref(), resolved.fetched_ai.as_ref())?;
        // 同じ取得結果からサーバー一覧も得られるのでキャッシュしておく
        *self.servers.lock().await = Some(resolved.servers);
        *cached = Some(ai_config.clone());
        Ok(ai_config)
    }

    /// テーブル → カラム名リストのマップを解決する (キャッシュあり)。
    /// SQL 補完 (get_schema_map) と AI の SQL 生成コンテキストで共有する。
    async fn resolve_schema_map(
        &self,
        server: &ServerConfig,
        schema_key: &str,
    ) -> Result<std::collections::BTreeMap<String, Vec<String>>, AppError> {
        if let Some(map) = self
            .schema_cache
            .get_schema_map(&server.name, schema_key)
            .await
        {
            return Ok(map);
        }
        let pool = self.db.get_pool(server).await?;
        let all = schema_info::fetch_all_columns(&pool).await?;
        let map = all
            .iter()
            .map(|(table, columns)| {
                (
                    table.clone(),
                    columns.iter().map(|c| c.name.clone()).collect(),
                )
            })
            .collect();
        self.schema_cache
            .put_all_columns(&server.name, schema_key, all)
            .await;
        Ok(map)
    }

    /// AI コマンド (SQL 生成 / エラー修正) 共通のコンテキストを解決する:
    /// AI 設定・接続設定・プロンプト用アクティブスキーマ名・スキーママップ。
    /// AI 未設定時は案内メッセージのエラーを返す。
    async fn resolve_ai_context(
        &self,
        connection: &str,
    ) -> Result<
        (
            ai::AiConfig,
            ServerConfig,
            Option<String>,
            std::collections::BTreeMap<String, Vec<String>>,
        ),
        AppError,
    > {
        let ai_config = self.resolve_ai_config().await?.ok_or_else(|| {
            AppError::Ai(
                "AI is not configured. Add an 'ai:' section (provider / api_key) \
                 to config.yml or the connection YAML"
                    .into(),
            )
        })?;
        let server = self.find_server(connection).await?;
        let schema_key = self.active_schema_key(&server).await;
        let schema_map = self.resolve_schema_map(&server, &schema_key).await?;
        // sqlite の schema はローカル DB ファイルパスなので、プロンプトには含めない
        let is_sqlite = matches!(
            server.engine.to_ascii_lowercase().as_str(),
            "sqlite" | "sqlite3"
        );
        let active_schema =
            (!is_sqlite && !schema_key.trim().is_empty()).then_some(schema_key);
        Ok((ai_config, server, active_schema, schema_map))
    }
}

#[tauri::command]
async fn get_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>, AppError> {
    let config = AppConfig::load()?;
    let resolved = config.resolve_servers().await?;
    let infos = resolved.servers.iter().map(ConnectionInfo::from).collect();
    // 同じ取得結果から AI 設定も解決してキャッシュする (取得コマンドの
    // 再実行を避けるため)。解決エラーはここでは接続一覧を壊さず、
    // get_ai_info / ai_generate_sql 側の再解決で返す。
    match ai::resolve_ai_config(config.local_ai().as_ref(), resolved.fetched_ai.as_ref()) {
        Ok(ai_config) => *state.ai.lock().await = Some(ai_config),
        Err(_) => *state.ai.lock().await = None,
    }
    *state.servers.lock().await = Some(resolved.servers);
    Ok(infos)
}

/// 接続設定のキャッシュ・プール・SSH トンネルを破棄する。
/// 設定を変更した後のリロード時に呼ぶ。
#[tauri::command]
async fn reset_connections(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    *state.servers.lock().await = None;
    *state.sqlfiles_dir.lock().await = None;
    *state.default_limit.lock().await = None;
    *state.ai.lock().await = None;
    state.db.reset().await;
    state.schema_cache.clear().await;
    Ok(())
}

#[tauri::command]
async fn run_query(
    state: tauri::State<'_, AppState>,
    connection: String,
    sql: String,
    max_rows: Option<usize>,
) -> Result<QueryResult, AppError> {
    let server = state.find_server(&connection).await?;
    // 履歴記録用に実行時点のアクティブスキーマを控えておく
    let schema = match state.db.schema_override(&connection).await {
        Some(schema) => Some(schema),
        None => server.schema.clone(),
    };
    let auto_limit = match state.resolve_default_limit().await? {
        0 => None,
        limit => Some(limit),
    };
    let started = std::time::Instant::now();
    let result = async {
        let pool: DbPool = state.db.get_pool(&server).await?;
        db::run_query_cancellable(
            &pool,
            &state.query_cancels,
            &connection,
            &sql,
            max_rows.unwrap_or(DEFAULT_MAX_ROWS),
            auto_limit,
            server.readonly,
        )
        .await
    }
    .await;

    // 成功・失敗にかかわらず実行履歴を記録する。
    // 記録の失敗でクエリ結果を損なわないよう、エラーはログに留める。
    // (追記は小さな同期 I/O なので async コンテキストのまま行う。
    //  ローテーション時のみ全読み・書き直しが走るが、上限 1 万行 =
    //  高々数 MB のため許容する)
    let entry = history::HistoryEntry {
        time: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        sql,
        schema,
        row_count: result.as_ref().ok().map(|r| match r.affected_rows {
            Some(affected) => affected,
            None => r.row_count as u64,
        }),
        elapsed_ms: started.elapsed().as_millis() as u64,
        success: result.is_ok(),
    };
    match history::default_history_dir() {
        Ok(dir) => {
            if let Err(e) = state.history.append(&dir, &connection, &entry) {
                eprintln!("[history] failed to record the query history: {e}");
            }
        }
        Err(e) => eprintln!("[history] {e}"),
    }

    result
}

/// 接続で実行中のクエリにキャンセルを要求する。
/// 実行中のクエリが無ければ何もせず false を返す。
/// キャンセルされた実行は run_query 側が AppError::Cancelled
/// ("Query cancelled") で返る。
#[tauri::command]
async fn cancel_query(
    state: tauri::State<'_, AppState>,
    connection: String,
) -> Result<bool, AppError> {
    state.query_cancels.cancel(&connection).await
}

/// 接続のクエリ実行履歴を新しい順に返す。
/// search を指定すると SQL の部分一致 (大文字小文字を区別しない) で絞り込む。
#[tauri::command]
fn list_query_history(
    connection: String,
    search: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<history::HistoryEntry>, AppError> {
    history::list_history(
        &history::default_history_dir()?,
        &connection,
        search.as_deref(),
        limit.unwrap_or(history::DEFAULT_LIST_LIMIT),
    )
}

#[tauri::command]
async fn list_query_files(
    state: tauri::State<'_, AppState>,
    connection: String,
) -> Result<Vec<String>, AppError> {
    query_files::list_query_files(&state.resolve_sqlfiles_dir().await?, &connection)
}

#[tauri::command]
async fn read_query_file(
    state: tauri::State<'_, AppState>,
    connection: String,
    file_name: String,
) -> Result<String, AppError> {
    query_files::read_query_file(
        &state.resolve_sqlfiles_dir().await?,
        &connection,
        &file_name,
    )
}

#[tauri::command]
async fn write_query_file(
    state: tauri::State<'_, AppState>,
    connection: String,
    file_name: String,
    content: String,
) -> Result<(), AppError> {
    query_files::write_query_file(
        &state.resolve_sqlfiles_dir().await?,
        &connection,
        &file_name,
        &content,
    )
}

#[tauri::command]
async fn create_query_file(
    state: tauri::State<'_, AppState>,
    connection: String,
    file_name: String,
) -> Result<String, AppError> {
    query_files::create_query_file(
        &state.resolve_sqlfiles_dir().await?,
        &connection,
        &file_name,
    )
}

#[tauri::command]
async fn delete_query_file(
    state: tauri::State<'_, AppState>,
    connection: String,
    file_name: String,
) -> Result<(), AppError> {
    query_files::delete_query_file(
        &state.resolve_sqlfiles_dir().await?,
        &connection,
        &file_name,
    )
}

/// 接続先サーバー上の database (スキーマ) 一覧を返す。
#[tauri::command]
async fn list_schemas(
    state: tauri::State<'_, AppState>,
    connection: String,
) -> Result<Vec<String>, AppError> {
    let server = state.find_server(&connection).await?;
    let pool = state.db.get_pool(&server).await?;
    db::list_schemas(&pool, &server).await
}

/// 接続のアクティブスキーマ (database) を切り替える。
/// プールが再構築され、次のクエリから新しい database に接続される。
#[tauri::command]
async fn set_active_schema(
    state: tauri::State<'_, AppState>,
    connection: String,
    schema: String,
) -> Result<(), AppError> {
    if schema.trim().is_empty() {
        return Err(AppError::Config("The schema name is empty".into()));
    }
    // 接続名の実在確認 (存在しない接続へのオーバーライド蓄積を防ぐ)
    state.find_server(&connection).await?;
    state.db.set_schema_override(&connection, schema).await;
    // 切替後に古いスキーマ情報を返さないよう、接続単位でキャッシュを破棄する
    state.schema_cache.invalidate_connection(&connection).await;
    Ok(())
}

/// 接続のアクティブスキーマを返す (オーバーライトが無ければ設定のデフォルト)。
#[tauri::command]
async fn get_active_schema(
    state: tauri::State<'_, AppState>,
    connection: String,
) -> Result<Option<String>, AppError> {
    if let Some(schema) = state.db.schema_override(&connection).await {
        return Ok(Some(schema));
    }
    let server = state.find_server(&connection).await?;
    Ok(server.schema)
}

/// 接続先のテーブル / ビューの一覧を返す (キャッシュあり)。
/// refresh = true でキャッシュを破棄して再取得する (リロードボタン用)。
#[tauri::command]
async fn list_tables(
    state: tauri::State<'_, AppState>,
    connection: String,
    refresh: Option<bool>,
) -> Result<Vec<schema_info::TableInfo>, AppError> {
    let server = state.find_server(&connection).await?;
    let schema_key = state.active_schema_key(&server).await;
    if refresh.unwrap_or(false) {
        // カラムのキャッシュも古い可能性があるため、スキーマ単位で丸ごと破棄する
        state
            .schema_cache
            .invalidate_schema(&connection, &schema_key)
            .await;
    } else if let Some(tables) = state.schema_cache.get_tables(&connection, &schema_key).await {
        return Ok(tables);
    }
    let pool = state.db.get_pool(&server).await?;
    let tables = schema_info::fetch_tables(&pool).await?;
    state
        .schema_cache
        .put_tables(&connection, &schema_key, &tables)
        .await;
    Ok(tables)
}

/// テーブルのカラム一覧を返す (キャッシュあり。ツリー展開時の遅延ロード用)。
/// table は list_tables が返す qualified_name を渡す。
#[tauri::command]
async fn list_columns(
    state: tauri::State<'_, AppState>,
    connection: String,
    table: String,
) -> Result<Vec<schema_info::ColumnInfo>, AppError> {
    let server = state.find_server(&connection).await?;
    let schema_key = state.active_schema_key(&server).await;
    if let Some(columns) = state
        .schema_cache
        .get_columns(&connection, &schema_key, &table)
        .await
    {
        return Ok(columns);
    }
    let pool = state.db.get_pool(&server).await?;
    let columns = schema_info::fetch_columns(&pool, &table).await?;
    state
        .schema_cache
        .put_columns(&connection, &schema_key, &table, &columns)
        .await;
    Ok(columns)
}

/// テーブル名 → カラム名リストのマップを返す (SQL 補完の強化用)。
/// キャッシュに全テーブル分のカラムが無ければ一括取得してキャッシュする。
#[tauri::command]
async fn get_schema_map(
    state: tauri::State<'_, AppState>,
    connection: String,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, AppError> {
    let server = state.find_server(&connection).await?;
    let schema_key = state.active_schema_key(&server).await;
    state.resolve_schema_map(&server, &schema_key).await
}

/// AI 設定の情報 (configured / model) を返す。api_key は含めない。
/// `ai:` セクションが無い場合はエラーではなく configured: false。
/// セクションはあるが不正 (不明 provider 等) な場合はエラーを返す。
#[tauri::command]
async fn get_ai_info(state: tauri::State<'_, AppState>) -> Result<ai::AiInfo, AppError> {
    Ok(match state.resolve_ai_config().await? {
        Some(config) => ai::AiInfo {
            configured: true,
            model: config.model().to_string(),
        },
        None => ai::AiInfo {
            configured: false,
            model: String::new(),
        },
    })
}

/// 自然言語の指示から SQL を生成して返す。実行はせず、エディタへの
/// 挿入もフロント側に任せる (ユーザーが確認してから実行する)。
/// LLM に送るのはスキーマ情報 (テーブル・カラム名)・エンジン方言・
/// アクティブスキーマ名・ユーザーの指示のみ。クエリの結果データや
/// 接続情報 (ホスト・認証情報) は送らない。
#[tauri::command]
async fn ai_generate_sql(
    state: tauri::State<'_, AppState>,
    connection: String,
    instruction: String,
) -> Result<String, AppError> {
    if instruction.trim().is_empty() {
        return Err(AppError::Ai("The instruction is empty".into()));
    }
    let (ai_config, server, active_schema, schema_map) =
        state.resolve_ai_context(&connection).await?;
    let system_prompt =
        ai::build_sql_system_prompt(&server.engine, active_schema.as_deref(), &schema_map);
    let response = ai::chat_complete(&ai_config, &system_prompt, &instruction).await?;
    Ok(ai::strip_sql_fences(&response))
}

/// 失敗した SQL と DB のエラーメッセージから修正案の SQL を生成して返す。
/// 実行はせず、エディタへの反映もユーザーの確認 (Apply) に任せる。
/// LLM に送るのは失敗した SQL・エラーメッセージ・スキーマ情報
/// (テーブル・カラム名)・エンジン方言・アクティブスキーマ名のみ。
/// クエリの結果データや接続情報 (ホスト・認証情報) は送らない。
/// 注意: DB のエラーメッセージ自体が値を含むことがある (例: 一意制約違反の
/// DETAIL に衝突したキー値が載る)。修正に必要な情報のため加工せず送る
/// 設計とし、フロントのボタン tooltip で送信内容を明示している。
#[tauri::command]
async fn ai_fix_sql(
    state: tauri::State<'_, AppState>,
    connection: String,
    sql: String,
    error_message: String,
) -> Result<String, AppError> {
    if sql.trim().is_empty() {
        return Err(AppError::Ai("The SQL statement is empty".into()));
    }
    if error_message.trim().is_empty() {
        return Err(AppError::Ai("The error message is empty".into()));
    }
    let (ai_config, server, active_schema, schema_map) =
        state.resolve_ai_context(&connection).await?;
    let system_prompt =
        ai::build_fix_sql_system_prompt(&server.engine, active_schema.as_deref(), &schema_map);
    let user_prompt = ai::build_fix_sql_user_prompt(&sql, &error_message);
    let response = ai::chat_complete(&ai_config, &system_prompt, &user_prompt).await?;
    Ok(ai::strip_sql_fences(&response))
}

/// エンジン別の EXPLAIN プレフィックスを付けた SQL を組み立てて返す。
/// 実行はしない (フロントが通常の run_query 経路で実行する)。
/// 対象は SELECT / WITH のみ (Postgres の EXPLAIN ANALYZE は対象文を
/// 実際に実行するため、DML への付与はエラーで拒否する)。
#[tauri::command]
async fn build_explain_sql(
    state: tauri::State<'_, AppState>,
    connection: String,
    sql: String,
) -> Result<String, AppError> {
    let server = state.find_server(&connection).await?;
    db::build_explain_sql(&server.engine, &sql)
}

/// EXPLAIN の実行計画を AI に解説させ、ボトルネックの特定・インデックス
/// 提案・書き直し案の Markdown を返す。LLM に送るのはスキーマ情報
/// (テーブル・カラム名)・エンジン方言・アクティブスキーマ名・SQL・
/// 実行計画テキストのみ (実行計画はクエリの結果データではなくプランナー
/// 出力なので許容する)。接続情報 (ホスト・認証情報) は送らない。
#[tauri::command]
async fn ai_explain_plan(
    state: tauri::State<'_, AppState>,
    connection: String,
    sql: String,
    plan_text: String,
) -> Result<String, AppError> {
    if sql.trim().is_empty() {
        return Err(AppError::Ai("The SQL statement is empty".into()));
    }
    if plan_text.trim().is_empty() {
        return Err(AppError::Ai("The execution plan is empty".into()));
    }
    let ai_config = state.resolve_ai_config().await?.ok_or_else(|| {
        AppError::Ai(
            "AI is not configured. Add an 'ai:' section (provider / api_key) \
             to config.yml or the connection YAML"
                .into(),
        )
    })?;
    let server = state.find_server(&connection).await?;
    let schema_key = state.active_schema_key(&server).await;
    let schema_map = state.resolve_schema_map(&server, &schema_key).await?;
    // sqlite の schema はローカル DB ファイルパスなので、プロンプトには含めない
    let is_sqlite = matches!(
        server.engine.to_ascii_lowercase().as_str(),
        "sqlite" | "sqlite3"
    );
    let active_schema =
        (!is_sqlite && !schema_key.trim().is_empty()).then_some(schema_key.as_str());
    let system_prompt =
        ai::build_explain_system_prompt(&server.engine, active_schema, &schema_map);
    let user_message = ai::build_explain_user_message(&sql, &plan_text);
    let response = ai::chat_complete(&ai_config, &system_prompt, &user_message).await?;
    Ok(response.trim().to_string())
}

/// 設定の解決結果を返す (情報表示用。機密を含まない)。
#[tauri::command]
fn get_config_info() -> ConfigInfo {
    config::config_info()
}

/// config.yml が無ければテンプレートを作成する。作成した場合はそのパスを返す。
#[tauri::command]
fn ensure_config_file() -> Result<Option<String>, AppError> {
    config::ensure_config_file()
}

/// config.yml (無ければ設定フォルダ) を Finder 等のファイルマネージャで表示する。
fn reveal_config_folder() -> Result<(), AppError> {
    let target = match config::existing_config_path()? {
        Some(path) => path,
        None => config::app_config_dir()?,
    };
    tauri_plugin_opener::reveal_item_in_dir(&target)
        .map_err(|e| AppError::Config(format!("Failed to reveal {}: {e}", target.display())))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::menu::{Menu, MenuItemBuilder, SubmenuBuilder};
    use tauri::Emitter;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // 終了時のウインドウサイズ・位置を保存し、起動時に復元する
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            // デフォルトメニューに Config サブメニューを追加する
            let menu = Menu::default(app.handle())?;
            let reload_item = MenuItemBuilder::with_id("reload_config_file", "Reload config file")
                .accelerator("CmdOrCtrl+R")
                .build(app)?;
            let reveal_item =
                MenuItemBuilder::with_id("reveal_config_folder", "Reveal config folder")
                    .build(app)?;
            let config_menu = SubmenuBuilder::new(app, "Config")
                .item(&reload_item)
                .item(&reveal_item)
                .build()?;
            menu.append(&config_menu)?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "reload_config_file" => {
                // 再読込はフロントの状態 (選択・未保存編集) と連動するため、
                // イベントで通知してフロント側の reloadConnections に任せる
                if let Err(e) = app.emit("menu-reload-config", ()) {
                    eprintln!("[menu] failed to emit reload event: {e}");
                }
            }
            "reveal_config_folder" => {
                if let Err(e) = reveal_config_folder() {
                    eprintln!("[menu] {e}");
                }
            }
            _ => {}
        })
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_connections,
            reset_connections,
            run_query,
            cancel_query,
            list_query_history,
            list_query_files,
            read_query_file,
            write_query_file,
            create_query_file,
            delete_query_file,
            list_schemas,
            set_active_schema,
            get_active_schema,
            list_tables,
            list_columns,
            get_schema_map,
            get_ai_info,
            ai_generate_sql,
            build_explain_sql,
            ai_explain_plan,
            ai_fix_sql,
            get_config_info,
            ensure_config_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
