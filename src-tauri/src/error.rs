use serde::{Serialize, Serializer};

/// アプリ全体のエラー型。Tauri コマンドの戻り値としてフロントエンドに
/// 文字列として渡す。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Config error: {0}")]
    Config(String),

    #[error("SSH tunnel error: {0}")]
    SshTunnel(String),

    #[error("Query file error: {0}")]
    QueryFile(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
