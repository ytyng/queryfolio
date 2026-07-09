use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// アプリ設定。~/.config/queryfolio/settings.json に保存する。
///
/// GUI アプリを Finder / Dock から起動した場合は shell の環境変数を
/// 継承しないため、config YAML の取得方法をこの設定に保存できるようにする。
/// 環境変数 (QUERYFOLIO_CONFIG_YAML 等) はこの設定より優先される (開発用)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    /// 接続設定 YAML ファイルのパス。未設定なら ~/.config/queryfolio/config.yaml
    #[serde(default)]
    pub config_yaml_path: Option<String>,

    /// 接続設定 YAML を stdout に出力するコマンド (例: `op read "op://..."`)。
    /// 設定されている場合、config_yaml_path より優先される。
    #[serde(default)]
    pub config_yaml_getter_command: Option<String>,

    /// クエリファイルの保存ディレクトリ。
    /// 未設定なら ~/.config/queryfolio/sqlfiles
    #[serde(default)]
    pub sqlfiles_dir: Option<String>,
}

/// ~/.config/queryfolio ディレクトリを返す。
pub fn app_config_dir() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Config("ホームディレクトリを特定できません".into()))?;
    Ok(home.join(".config").join("queryfolio"))
}

fn settings_file_path() -> Result<PathBuf, AppError> {
    Ok(app_config_dir()?.join("settings.json"))
}

/// パス文字列の先頭の ~ をホームディレクトリに展開する。
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

impl AppSettings {
    pub fn load() -> Result<Self, AppError> {
        let path = settings_file_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)?;
        serde_json::from_str(&text).map_err(|e| {
            AppError::Config(format!("settings.json のパースに失敗: {e}"))
        })
    }

    pub fn save(&self) -> Result<(), AppError> {
        let path = settings_file_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| {
            AppError::Config(format!("settings.json のシリアライズに失敗: {e}"))
        })?;
        fs::write(&path, text)?;
        Ok(())
    }

    /// 接続設定 YAML ファイルのパスを解決する。
    pub fn resolve_config_yaml_path(&self) -> Result<PathBuf, AppError> {
        match &self.config_yaml_path {
            Some(path) if !path.trim().is_empty() => Ok(expand_tilde(path)),
            _ => Ok(app_config_dir()?.join("config.yaml")),
        }
    }

    /// クエリファイルの保存ディレクトリを解決する。
    pub fn resolve_sqlfiles_dir(&self) -> Result<PathBuf, AppError> {
        match &self.sqlfiles_dir {
            Some(dir) if !dir.trim().is_empty() => Ok(expand_tilde(dir)),
            _ => Ok(app_config_dir()?.join("sqlfiles")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~/foo/bar"), home.join("foo/bar"));
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_tilde("rel/path"), PathBuf::from("rel/path"));
    }
}
