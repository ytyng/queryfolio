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
pub(crate) fn connection_dir(
    sqlfiles_dir: &Path,
    connection: &str,
) -> Result<PathBuf, AppError> {
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

/// ディレクトリ直下の .sql ファイル名を昇順で返す。存在しなければ空。
/// (list_query_files と search_query_files で列挙条件を共有し、
///  隠しファイル/拡張子判定/ソートが片方だけズレるのを防ぐ)
/// dot 始まりの隠しファイルは除外する。validate_component が dot 始まりの名前を
/// 拒否する (= CRUD で開けない) のと一貫させ、手動配置された隠し .sql の中身が
/// 検索プレビューから漏れないようにする。
fn list_sql_file_names(dir: &Path) -> Result<Vec<String>, AppError> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names: Vec<String> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .filter(|name| name.to_ascii_lowercase().ends_with(".sql"))
        .collect();
    names.sort();
    Ok(names)
}

/// 接続のクエリファイル一覧を返す (名前昇順)。
pub fn list_query_files(
    sqlfiles_dir: &Path,
    connection: &str,
) -> Result<Vec<String>, AppError> {
    list_sql_file_names(&connection_dir(sqlfiles_dir, connection)?)
}

/// クエリファイル検索の 1 ヒット。
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct FileSearchHit {
    /// ヒットしたファイル名 (.sql 付き)
    pub file_name: String,
    /// ファイル名が query に一致したか
    pub name_match: bool,
    /// 中身が一致した最初の行 (プレビュー用。名前のみ一致なら None)
    pub content_preview: Option<String>,
}

/// プレビュー行の最大文字数 (これを超えたら末尾を省略記号にする)。
const PREVIEW_MAX_CHARS: usize = 120;

/// 検索結果の最大件数。名前昇順で先頭からこの数で打ち切る
/// (モーダルの一覧を短く保ち、多数ファイル環境での読み取りコストも抑える)。
const MAX_SEARCH_HITS: usize = 50;

/// プレビュー行を前後の空白除去 + 長さ制限で整形する。
fn truncate_preview(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= PREVIEW_MAX_CHARS {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(PREVIEW_MAX_CHARS).collect();
    format!("{cut}…")
}

/// 接続のクエリファイルをファイル名・中身で検索する。
/// 大文字小文字を区別しない部分一致。中身は最初に一致した行をプレビューとして返す。
/// 名前昇順で、名前一致または中身一致したファイルのみ返す。
/// (rg/grep のような外部プロセスは使わない。クエリファイルは少数のため
///  純 Rust で読み取る方が堅牢で、外部依存・インジェクション面も持たない)
pub fn search_query_files(
    sqlfiles_dir: &Path,
    connection: &str,
    query: &str,
) -> Result<Vec<FileSearchHit>, AppError> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(vec![]);
    }
    let dir = connection_dir(sqlfiles_dir, connection)?;
    let names = list_sql_file_names(&dir)?;

    let mut hits = Vec::new();
    for name in names {
        let name_match = name.to_lowercase().contains(&needle);
        // 中身検索。読めないファイル (バイナリ等) はスキップし、名前一致だけで拾う
        let content_preview = fs::read_to_string(dir.join(&name))
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|line| line.to_lowercase().contains(&needle))
                    .map(truncate_preview)
            });
        if name_match || content_preview.is_some() {
            hits.push(FileSearchHit {
                file_name: name,
                name_match,
                content_preview,
            });
            // 名前昇順で先頭から上限まで。以降のファイルは読まずに打ち切る
            if hits.len() >= MAX_SEARCH_HITS {
                break;
            }
        }
    }
    Ok(hits)
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

    #[test]
    fn test_search_query_files() {
        let dir = test_dir().join("search");
        let connection = "test-conn";

        create_query_file(&dir, connection, "users report").unwrap();
        write_query_file(
            &dir,
            connection,
            "users report",
            "SELECT * FROM users WHERE active = 1;",
        )
        .unwrap();
        create_query_file(&dir, connection, "orders").unwrap();
        write_query_file(
            &dir,
            connection,
            "orders",
            "SELECT id, total FROM orders;",
        )
        .unwrap();

        // 空クエリは空
        assert!(search_query_files(&dir, connection, "  ").unwrap().is_empty());

        // ファイル名一致 (大文字小文字を区別しない)
        let hits = search_query_files(&dir, connection, "USERS").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_name, "users report.sql");
        assert!(hits[0].name_match);
        // "users" は中身にもあるのでプレビューが付く
        assert!(hits[0].content_preview.as_deref().unwrap().contains("users"));

        // 中身のみ一致 (ファイル名は "orders" だが中身に total がある)
        let hits = search_query_files(&dir, connection, "total").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_name, "orders.sql");
        assert!(!hits[0].name_match);
        assert_eq!(
            hits[0].content_preview.as_deref(),
            Some("SELECT id, total FROM orders;")
        );

        // どちらにも無い語は 0 件
        assert!(search_query_files(&dir, connection, "zzz").unwrap().is_empty());

        // 手動配置された隠し .sql は検索対象外 (中身プレビューを漏らさない)。
        // validate_component が dot 始まりを拒否するため create 経由では作れないので
        // 直接ファイルを書き込んで再現する
        fs::write(
            connection_dir(&dir, connection).unwrap().join(".secret.sql"),
            "SELECT secret_total FROM vault;",
        )
        .unwrap();
        assert!(search_query_files(&dir, connection, "secret")
            .unwrap()
            .is_empty());
        assert!(!list_query_files(&dir, connection)
            .unwrap()
            .iter()
            .any(|f| f.starts_with('.')));

        // 存在しない接続ディレクトリは 0 件
        assert!(search_query_files(&dir, "no-such-conn", "users")
            .unwrap()
            .is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_preview() {
        assert_eq!(truncate_preview("  SELECT 1  "), "SELECT 1");
        let long = "x".repeat(200);
        let out = truncate_preview(&long);
        assert_eq!(out.chars().count(), PREVIEW_MAX_CHARS + 1); // +1 は省略記号
        assert!(out.ends_with('…'));
    }
}
