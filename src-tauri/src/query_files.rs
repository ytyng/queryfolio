use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// パス要素として安全な名前かを検証して返す。
/// パストラバーサルや不可視ファイルを防ぐ。
/// (history.rs でも接続名の検証に使う)
pub(crate) fn validate_component(name: &str) -> Result<&str, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::QueryFile("The name is empty".into()));
    }
    if name.starts_with('.') {
        return Err(AppError::QueryFile(format!(
            "Names starting with a dot are not allowed: {name}"
        )));
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(AppError::QueryFile(format!(
            "The name contains invalid characters: {name}"
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
            "File not found: {}",
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
            "A file with the same name already exists: {normalized}"
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
            "File not found: {}",
            path.display()
        )));
    }
    fs::remove_file(&path)?;
    Ok(())
}

/// クエリファイルをリネームし、正規化された新しいファイル名を返す。
/// 新旧が同名 (正規化後) なら no-op で新名を返す。
pub fn rename_query_file(
    sqlfiles_dir: &Path,
    connection: &str,
    old_name: &str,
    new_name: &str,
) -> Result<String, AppError> {
    let old_normalized = normalize_file_name(old_name)?;
    let new_normalized = normalize_file_name(new_name)?;
    if old_normalized == new_normalized {
        return Ok(new_normalized);
    }
    let old_path = file_path(sqlfiles_dir, connection, &old_normalized)?;
    if !old_path.exists() {
        return Err(AppError::QueryFile(format!(
            "File not found: {}",
            old_path.display()
        )));
    }
    // 衝突判定は case-insensitive で行う (case-insensitive FS の実挙動と揃え、
    // フロントの判定とも一致させる)。リネーム対象自身 (old) は除外するので、
    // 大文字小文字だけを変える改名 (Test.sql -> test.sql) は許可される。
    let new_lower = new_normalized.to_ascii_lowercase();
    let dir = connection_dir(sqlfiles_dir, connection)?;
    if dir.exists() {
        for entry in fs::read_dir(&dir)?.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name != old_normalized && name.to_ascii_lowercase() == new_lower {
                return Err(AppError::QueryFile(format!(
                    "A file with the same name already exists: {new_normalized}"
                )));
            }
        }
    }
    let new_path = file_path(sqlfiles_dir, connection, &new_normalized)?;
    fs::rename(&old_path, &new_path)?;
    Ok(new_normalized)
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

    #[test]
    fn test_rename_query_file() {
        let dir = test_dir().join("rename");
        let connection = "test-conn";

        create_query_file(&dir, connection, "old").unwrap();
        write_query_file(&dir, connection, "old", "SELECT 1;").unwrap();

        // リネーム成功 (内容は保持される)
        let renamed = rename_query_file(&dir, connection, "old", "new").unwrap();
        assert_eq!(renamed, "new.sql");
        assert_eq!(
            list_query_files(&dir, connection).unwrap(),
            vec!["new.sql"]
        );
        assert_eq!(
            read_query_file(&dir, connection, "new").unwrap(),
            "SELECT 1;"
        );

        // 既存名への変更は拒否
        create_query_file(&dir, connection, "other").unwrap();
        assert!(rename_query_file(&dir, connection, "new", "other").is_err());

        // 同名 (正規化後) への変更は no-op
        assert_eq!(
            rename_query_file(&dir, connection, "new", "new.sql").unwrap(),
            "new.sql"
        );

        // 存在しないファイルのリネームはエラー
        assert!(rename_query_file(&dir, connection, "missing", "x").is_err());

        // 不正な新名は拒否 (パストラバーサル)
        assert!(rename_query_file(&dir, connection, "new", "../evil").is_err());
        assert!(rename_query_file(&dir, connection, "new", "a/b").is_err());

        // 大文字小文字違いの別ファイルへの改名は拒否 (case-insensitive 判定)
        assert!(rename_query_file(&dir, connection, "new", "OTHER").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_rename_query_file_case_only() {
        let dir = test_dir().join("rename-case");
        let connection = "test-conn";

        create_query_file(&dir, connection, "Report").unwrap();
        write_query_file(&dir, connection, "Report", "SELECT 2;").unwrap();

        // 自分自身の大文字小文字だけを変える改名は許可される
        let renamed =
            rename_query_file(&dir, connection, "Report", "report").unwrap();
        assert_eq!(renamed, "report.sql");
        assert_eq!(
            read_query_file(&dir, connection, "report").unwrap(),
            "SELECT 2;"
        );
        // case-insensitive FS では 1 ファイルのまま、case-sensitive FS でも
        // 旧名は残らない (rename 済み)
        let files = list_query_files(&dir, connection).unwrap();
        assert!(files.iter().any(|f| f.eq_ignore_ascii_case("report.sql")));
        assert!(!files.contains(&"Report.sql".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }
}
