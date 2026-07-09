mod config;
mod db;
mod error;
mod query_files;
mod settings;
mod tunnel;

use config::{ConnectionInfo, ServerConfig};
use db::{DbManager, DbPool, QueryResult, DEFAULT_MAX_ROWS};
use error::AppError;
use settings::AppSettings;

/// アプリ全体の共有状態。
#[derive(Default)]
struct AppState {
    /// 接続設定のキャッシュ。get_connections で更新される。
    /// パスワード等の機密を含むためフロントエンドには渡さない。
    servers: tokio::sync::Mutex<Option<Vec<ServerConfig>>>,
    db: DbManager,
}

impl AppState {
    async fn find_server(&self, connection: &str) -> Result<ServerConfig, AppError> {
        let mut servers = self.servers.lock().await;
        if servers.is_none() {
            let app_settings = AppSettings::load()?;
            *servers = Some(config::load_servers(&app_settings).await?);
        }
        servers
            .as_ref()
            .unwrap()
            .iter()
            .find(|s| s.name == connection)
            .cloned()
            .ok_or_else(|| {
                AppError::Config(format!("接続 '{connection}' が設定にありません"))
            })
    }
}

#[tauri::command]
async fn get_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>, AppError> {
    let app_settings = AppSettings::load()?;
    let servers = config::load_servers(&app_settings).await?;
    let infos = servers.iter().map(ConnectionInfo::from).collect();
    *state.servers.lock().await = Some(servers);
    Ok(infos)
}

/// 接続設定のキャッシュ・プール・SSH トンネルを破棄する。
/// 設定を変更した後のリロード時に呼ぶ。
#[tauri::command]
async fn reset_connections(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    *state.servers.lock().await = None;
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
fn list_query_files(connection: String) -> Result<Vec<String>, AppError> {
    let app_settings = AppSettings::load()?;
    query_files::list_query_files(&app_settings, &connection)
}

#[tauri::command]
fn read_query_file(connection: String, file_name: String) -> Result<String, AppError> {
    let app_settings = AppSettings::load()?;
    query_files::read_query_file(&app_settings, &connection, &file_name)
}

#[tauri::command]
fn write_query_file(
    connection: String,
    file_name: String,
    content: String,
) -> Result<(), AppError> {
    let app_settings = AppSettings::load()?;
    query_files::write_query_file(&app_settings, &connection, &file_name, &content)
}

#[tauri::command]
fn create_query_file(connection: String, file_name: String) -> Result<String, AppError> {
    let app_settings = AppSettings::load()?;
    query_files::create_query_file(&app_settings, &connection, &file_name)
}

#[tauri::command]
fn delete_query_file(connection: String, file_name: String) -> Result<(), AppError> {
    let app_settings = AppSettings::load()?;
    query_files::delete_query_file(&app_settings, &connection, &file_name)
}

#[tauri::command]
fn get_settings() -> Result<AppSettings, AppError> {
    AppSettings::load()
}

#[tauri::command]
fn save_settings(settings: AppSettings) -> Result<(), AppError> {
    settings.save()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
            get_settings,
            save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
