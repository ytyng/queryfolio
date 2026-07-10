mod config;
mod db;
mod meta_commands;
mod error;
mod query_files;
mod tunnel;

use std::path::PathBuf;

use config::{AppConfig, ConfigInfo, ConnectionInfo, ServerConfig};
use db::{DbManager, DbPool, QueryResult, DEFAULT_MAX_ROWS};
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
            *servers = Some(AppConfig::load()?.resolve_servers().await?);
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
}

#[tauri::command]
async fn get_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>, AppError> {
    let servers = AppConfig::load()?.resolve_servers().await?;
    let infos = servers.iter().map(ConnectionInfo::from).collect();
    *state.servers.lock().await = Some(servers);
    Ok(infos)
}

/// 接続設定のキャッシュ・プール・SSH トンネルを破棄する。
/// 設定を変更した後のリロード時に呼ぶ。
#[tauri::command]
async fn reset_connections(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    *state.servers.lock().await = None;
    *state.sqlfiles_dir.lock().await = None;
    *state.default_limit.lock().await = None;
    state.db.reset().await;
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
    let pool: DbPool = state.db.get_pool(&server).await?;
    let auto_limit = match state.resolve_default_limit().await? {
        0 => None,
        limit => Some(limit),
    };
    db::run_query(
        &pool,
        &sql,
        max_rows.unwrap_or(DEFAULT_MAX_ROWS),
        auto_limit,
        server.readonly,
    )
    .await
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
            list_query_files,
            read_query_file,
            write_query_file,
            create_query_file,
            delete_query_file,
            list_schemas,
            set_active_schema,
            get_active_schema,
            get_config_info,
            ensure_config_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
