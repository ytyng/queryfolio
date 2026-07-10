mod config;
mod db;
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
    /// クエリファイル保存ディレクトリのセッションキャッシュ。
    /// config.yml は手編集されるため、開いているファイルの保存中に
    /// sqlfiles_dir が変わると未保存内容が新ディレクトリへ書かれてしまう。
    /// 再読込 (reset_connections) まで最初に解決した値を使い続けることで、
    /// dirty ファイルの保存先を読み込み時のディレクトリに固定する。
    sqlfiles_dir: tokio::sync::Mutex<Option<PathBuf>>,
    db: DbManager,
}

impl AppState {
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
    db::run_query(&pool, &sql, max_rows.unwrap_or(DEFAULT_MAX_ROWS)).await
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // 終了時のウインドウサイズ・位置を保存し、起動時に復元する
        .plugin(tauri_plugin_window_state::Builder::default().build())
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
            get_config_info,
            ensure_config_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
