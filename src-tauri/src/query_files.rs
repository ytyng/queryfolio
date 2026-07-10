use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// パス要素として安全な名前かを検証して返す。
/// パストラバーサルや不可視ファイルを防ぐ。
fn validate_component(name: &str) -> Result<&str, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::QueryFile("名前が空です".into()));
    }
    if name.starts_with('.') {
        return Err(AppError::QueryFile(format!(
            "先頭が . の名前は使用できません: {name}"
        )));
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(AppError::QueryFile(format!(
            "名前に使用できない文字が含まれています: {name}"
        )));
    }
    Ok(name)
}

/// クエリファイル名を正規化する (.sql 拡張子を保証する)。
fn normalize_file_name(name: &str) -> Result<String, AppError> {
    let name = validate_component(name)?;
    if name.to_ascii_lowercase().ends_with(".sql") {
        Ok(name.to_string())
    } else {
        Ok(format!("{name}.sql"))
    }
}

/// 接続名に対応するクエリファイル保存ディレクトリを返す。
fn connection_dir(sqlfiles_dir: &Path, connection: &str) -> Result<PathBuf, AppError> {
    let connection = validate_component(connection)?;
    Ok(sqlfiles_dir.join(connection))
}

fn file_path(
    sqlfiles_dir: &Path,
    connection: &str,
    file_name: &str,
) -> Result<PathBuf, AppError> {
    let file_name = normalize_file_name(file_name)?;
    Ok(connection_dir(sqlfiles_dir, connection)?.join(file_name))
}

/// 接続のクエリファイル一覧を返す (名前昇順)。
pub fn list_query_files(
    sqlfiles_dir: &Path,
    connection: &str,
) -> Result<Vec<String>, AppError> {
    let dir = connection_dir(sqlfiles_dir, connection)?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names: Vec<String> = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.to_ascii_lowercase().ends_with(".sql"))
        .collect();
    names.sort();
    Ok(names)
}

pub fn read_query_file(
    sqlfiles_dir: &Path,
    connection: &str,
    file_name: &str,
) -> Result<String, AppError> {
    let path = file_path(sqlfiles_dir, connection, file_name)?;
    if !path.exists() {
        return Err(AppError::QueryFile(format!(
            "ファイルが見つかりません: {}",
            path.display()
        )));
    }
    Ok(fs::read_to_string(&path)?)
}

pub fn write_query_file(
    sqlfiles_dir: &Path,
    connection: &str,
    file_name: &str,
    content: &str,
) -> Result<(), AppError> {
    let path = file_path(sqlfiles_dir, connection, file_name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

/// 空のクエリファイルを新規作成し、正規化されたファイル名を返す。
pub fn create_query_file(
    sqlfiles_dir: &Path,
    connection: &str,
    file_name: &str,
) -> Result<String, AppError> {
    let normalized = normalize_file_name(file_name)?;
    let path = file_path(sqlfiles_dir, connection, &normalized)?;
    if path.exists() {
        return Err(AppError::QueryFile(format!(
            "同名のファイルが既に存在します: {normalized}"
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, "")?;
    Ok(normalized)
}

pub fn delete_query_file(
    sqlfiles_dir: &Path,
    connection: &str,
    file_name: &str,
) -> Result<(), AppError> {
    let path = file_path(sqlfiles_dir, connection, file_name)?;
    if !path.exists() {
        return Err(AppError::QueryFile(format!(
            "ファイルが見つかりません: {}",
            path.display()
        )));
    }
    fs::remove_file(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "queryfolio-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn test_validate_component() {
        assert!(validate_component("normal-name").is_ok());
        assert!(validate_component("").is_err());
        assert!(validate_component("   ").is_err());
        assert!(validate_component("..").is_err());
        assert!(validate_component(".hidden").is_err());
        assert!(validate_component("a/b").is_err());
        assert!(validate_component("a\\b").is_err());
        assert!(validate_component("../../etc/passwd").is_err());
    }

    #[test]
    fn test_normalize_file_name() {
        assert_eq!(normalize_file_name("query").unwrap(), "query.sql");
        assert_eq!(normalize_file_name("query.sql").unwrap(), "query.sql");
        assert_eq!(normalize_file_name("query.SQL").unwrap(), "query.SQL");
        assert!(normalize_file_name("../evil").is_err());
    }

    #[test]
    fn test_query_file_crud() {
        let dir = test_dir();
        let connection = "test-conn";

        assert_eq!(
            list_query_files(&dir, connection).unwrap(),
            Vec::<String>::new()
        );

        let name = create_query_file(&dir, connection, "my query").unwrap();
        assert_eq!(name, "my query.sql");

        // 同名の再作成はエラー
        assert!(create_query_file(&dir, connection, "my query").is_err());

        write_query_file(&dir, connection, &name, "SELECT 1;").unwrap();
        assert_eq!(
            read_query_file(&dir, connection, &name).unwrap(),
            "SELECT 1;"
        );

        assert_eq!(
            list_query_files(&dir, connection).unwrap(),
            vec!["my query.sql"]
        );

        delete_query_file(&dir, connection, &name).unwrap();
        assert_eq!(
            list_query_files(&dir, connection).unwrap(),
            Vec::<String>::new()
        );

        // 後始末
        let _ = fs::remove_dir_all(&dir);
    }
}
