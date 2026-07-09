use serde::{Serialize, Serializer};

/// アプリ全体のエラー型。Tauri コマンドの戻り値としてフロントエンドに
/// 文字列として渡す。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("設定エラー: {0}")]
    Config(String),

    #[error("SSH トンネルエラー: {0}")]
    SshTunnel(String),

    #[error("クエリファイルエラー: {0}")]
    QueryFile(String),

    #[error("IO エラー: {0}")]
    Io(#[from] std::io::Error),

    #[error("DB エラー: {0}")]
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
