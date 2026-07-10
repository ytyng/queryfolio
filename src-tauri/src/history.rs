//! クエリ実行履歴の記録・検索。
//!
//! 接続ごとに JSONL 形式 (~/.config/queryfolio/history/<connection>.jsonl)
//! で追記し、上限行数を超えたら古い行を落としてローテーションする。
//! SQL 文面にはパスワード等の機密が含まれ得るため、履歴ディレクトリは
//! 700、履歴ファイルは 600 のパーミッションで作成する。

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::AppError;
use crate::query_files::validate_component;

/// 接続あたりの履歴の上限行数。
pub const MAX_HISTORY_LINES: usize = 10_000;

/// ローテーション後に残す行数。上限より少なくすることで、上限到達後に
/// 追記のたび全書き換えが走るのを防ぐ (次のローテーションまで
/// MAX_HISTORY_LINES - ROTATED_KEEP_LINES 回は追記だけで済む)。
const ROTATED_KEEP_LINES: usize = 9_000;

/// list_query_history で limit 未指定時に返す件数。
pub const DEFAULT_LIST_LIMIT: usize = 200;

/// 履歴 1 件分のレコード。JSONL の 1 行に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// 実行時刻 (ISO 8601 / RFC 3339)
    pub time: String,
    pub sql: String,
    /// 実行時のアクティブスキーマ (database)
    pub schema: Option<String>,
    /// 取得行数または影響行数 (失敗時は None)
    pub row_count: Option<u64>,
    /// 所要時間 (ミリ秒)
    pub elapsed_ms: u64,
    pub success: bool,
}

/// 接続ごとの履歴ファイル行数をキャッシュするマネージャ。
/// 追記のたびに全行を読み直すのを避け、初回アクセス時のみ実ファイルを
/// 数える。プロセス内で追記を直列化するため Mutex で保護する
/// (単一ユーザーのデスクトップアプリなので競合はプロセス内のみ)。
#[derive(Default)]
pub struct HistoryManager {
    counts: Mutex<HashMap<String, usize>>,
}

/// デフォルトの履歴保存ディレクトリ (~/.config/queryfolio/history)。
pub fn default_history_dir() -> Result<PathBuf, AppError> {
    Ok(config::app_config_dir()?.join("history"))
}

/// 接続名に対応する履歴ファイルのパスを返す。
/// 接続名はパス要素になるため validate_component で検証する。
fn history_file(history_dir: &Path, connection: &str) -> Result<PathBuf, AppError> {
    let connection = validate_component(connection)
        .map_err(|e| AppError::History(format!("Invalid connection name: {e}")))?;
    Ok(history_dir.join(format!("{connection}.jsonl")))
}

/// 履歴ディレクトリを作成し、パーミッションを 700 に設定する。
fn ensure_history_dir(history_dir: &Path) -> Result<(), AppError> {
    fs::create_dir_all(history_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(history_dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// ファイルの行数を数える (初回アクセス時のみ呼ばれる)。
fn count_lines(path: &Path) -> Result<usize, AppError> {
    if !path.exists() {
        return Ok(0);
    }
    let reader = BufReader::new(fs::File::open(path)?);
    Ok(reader.lines().count())
}

/// パーミッション 600 で書き込み用にファイルを開く共通オプション。
fn open_options_600(append: bool) -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

impl HistoryManager {
    /// 履歴を 1 件追記する。上限超過時は古い行を落としてローテーションする。
    pub fn append(
        &self,
        history_dir: &Path,
        connection: &str,
        entry: &HistoryEntry,
    ) -> Result<(), AppError> {
        self.append_with_limits(
            history_dir,
            connection,
            entry,
            MAX_HISTORY_LINES,
            ROTATED_KEEP_LINES,
        )
    }

    /// 上限をパラメータ化した実装本体 (テストで小さい上限を使うため分離)。
    fn append_with_limits(
        &self,
        history_dir: &Path,
        connection: &str,
        entry: &HistoryEntry,
        max_lines: usize,
        keep_lines: usize,
    ) -> Result<(), AppError> {
        let path = history_file(history_dir, connection)?;
        let line = serde_json::to_string(entry)
            .map_err(|e| AppError::History(format!("Failed to serialize an entry: {e}")))?;

        // ロックで追記処理全体を直列化し、カウンタと実ファイルの整合を保つ
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let mut count = match counts.get(connection) {
            Some(count) => *count,
            None => {
                // 初回アクセス: 既存行数を数え、旧バージョンや手動作成の
                // ファイルでもパーミッションが 600 になるよう是正する
                let existing = count_lines(&path)?;
                #[cfg(unix)]
                if path.exists() {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
                }
                existing
            }
        };

        ensure_history_dir(history_dir)?;
        let mut file = open_options_600(true).open(&path)?;
        writeln!(file, "{line}")?;
        count += 1;

        if count > max_lines {
            count = rotate_file(&path, keep_lines)?;
        }
        counts.insert(connection.to_string(), count);
        Ok(())
    }
}

/// 履歴ファイルの末尾 keep_lines 行だけを残して書き直す。
/// 一時ファイルに書いてから rename することで、途中失敗しても
/// 元ファイルが壊れないようにする。残した行数を返す。
fn rotate_file(path: &Path, keep_lines: usize) -> Result<usize, AppError> {
    let reader = BufReader::new(fs::File::open(path)?);
    let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;
    let start = lines.len().saturating_sub(keep_lines);
    let kept = &lines[start..];

    let temp_path = path.with_extension("jsonl.tmp");
    {
        let mut file = open_options_600(false).open(&temp_path)?;
        for line in kept {
            writeln!(file, "{line}")?;
        }
        file.sync_all()?;
    }
    fs::rename(&temp_path, path)?;
    Ok(kept.len())
}

/// 履歴を新しい順に返す。search を指定すると SQL の部分一致
/// (大文字小文字を区別しない) で絞り込む。
pub fn list_history(
    history_dir: &Path,
    connection: &str,
    search: Option<&str>,
    limit: usize,
) -> Result<Vec<HistoryEntry>, AppError> {
    let path = history_file(history_dir, connection)?;
    if !path.exists() {
        return Ok(vec![]);
    }
    let needle = search
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let reader = BufReader::new(fs::File::open(&path)?);
    let mut entries: Vec<HistoryEntry> = vec![];
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // 壊れた行 (クラッシュ時の書きかけ等) は無視して読み進める
        let Ok(entry) = serde_json::from_str::<HistoryEntry>(&line) else {
            continue;
        };
        if let Some(needle) = &needle {
            if !entry.sql.to_lowercase().contains(needle) {
                continue;
            }
        }
        entries.push(entry);
    }
    // ファイルは追記順 = 古い順なので、反転して新しい順にする
    entries.reverse();
    entries.truncate(limit);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sql: &str, success: bool) -> HistoryEntry {
        HistoryEntry {
            time: "2026-07-11T12:00:00+09:00".into(),
            sql: sql.into(),
            schema: Some("main".into()),
            row_count: if success { Some(3) } else { None },
            elapsed_ms: 12,
            success,
        }
    }

    #[test]
    fn test_append_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let manager = HistoryManager::default();

        manager.append(dir.path(), "conn", &entry("SELECT 1", true)).unwrap();
        manager.append(dir.path(), "conn", &entry("SELECT 2", false)).unwrap();
        manager.append(dir.path(), "conn", &entry("SELECT 3", true)).unwrap();

        // 新しい順に返る
        let entries = list_history(dir.path(), "conn", None, 100).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].sql, "SELECT 3");
        assert_eq!(entries[2].sql, "SELECT 1");
        assert!(entries[0].success);
        assert!(!entries[1].success);
        assert_eq!(entries[1].row_count, None);
        assert_eq!(entries[0].row_count, Some(3));

        // limit で先頭 (新しい方) から切り詰める
        let entries = list_history(dir.path(), "conn", None, 2).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sql, "SELECT 3");
        assert_eq!(entries[1].sql, "SELECT 2");

        // 接続別にファイルが分かれる
        assert_eq!(list_history(dir.path(), "other", None, 100).unwrap().len(), 0);
    }

    #[test]
    fn test_list_search() {
        let dir = tempfile::tempdir().unwrap();
        let manager = HistoryManager::default();

        manager
            .append(dir.path(), "conn", &entry("SELECT * FROM users", true))
            .unwrap();
        manager
            .append(dir.path(), "conn", &entry("SELECT * FROM orders", true))
            .unwrap();
        manager
            .append(dir.path(), "conn", &entry("UPDATE users SET a = 1", true))
            .unwrap();

        // 部分一致 (大文字小文字を区別しない)
        let entries = list_history(dir.path(), "conn", Some("USERS"), 100).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sql, "UPDATE users SET a = 1");
        assert_eq!(entries[1].sql, "SELECT * FROM users");

        let entries = list_history(dir.path(), "conn", Some("orders"), 100).unwrap();
        assert_eq!(entries.len(), 1);

        // 空白のみの検索語は全件扱い
        let entries = list_history(dir.path(), "conn", Some("  "), 100).unwrap();
        assert_eq!(entries.len(), 3);

        let entries = list_history(dir.path(), "conn", Some("no-match"), 100).unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let manager = HistoryManager::default();

        // 上限 5 / ローテーション後 3 行で 7 回追記する
        for i in 1..=7 {
            manager
                .append_with_limits(
                    dir.path(),
                    "conn",
                    &entry(&format!("SELECT {i}"), true),
                    5,
                    3,
                )
                .unwrap();
        }
        // 6 回目で上限超過 → 末尾 3 行 (4,5,6) に縮小、7 回目で 4 行になる
        let entries = list_history(dir.path(), "conn", None, 100).unwrap();
        let sqls: Vec<&str> = entries.iter().map(|e| e.sql.as_str()).collect();
        assert_eq!(sqls, vec!["SELECT 7", "SELECT 6", "SELECT 5", "SELECT 4"]);

        // 一時ファイルが残っていない
        assert!(!dir.path().join("conn.jsonl.tmp").exists());
    }

    #[test]
    fn test_count_recovery_across_instances() {
        let dir = tempfile::tempdir().unwrap();

        // 別インスタンス (再起動相当) でも既存行数を数え直してローテーションする
        let manager1 = HistoryManager::default();
        for i in 1..=4 {
            manager1
                .append_with_limits(dir.path(), "conn", &entry(&format!("A{i}"), true), 5, 3)
                .unwrap();
        }
        let manager2 = HistoryManager::default();
        for i in 1..=2 {
            manager2
                .append_with_limits(dir.path(), "conn", &entry(&format!("B{i}"), true), 5, 3)
                .unwrap();
        }
        // 4 + 2 = 6 行 → 5 行超過でローテーションされ 3 行になる
        let entries = list_history(dir.path(), "conn", None, 100).unwrap();
        let sqls: Vec<&str> = entries.iter().map(|e| e.sql.as_str()).collect();
        assert_eq!(sqls, vec!["B2", "B1", "A4"]);
    }

    #[test]
    fn test_broken_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let manager = HistoryManager::default();
        manager.append(dir.path(), "conn", &entry("SELECT 1", true)).unwrap();

        // クラッシュ等による壊れた行が混ざっても読み進められる
        let path = dir.path().join("conn.jsonl");
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{broken json").unwrap();
        drop(file);
        manager.append(dir.path(), "conn", &entry("SELECT 2", true)).unwrap();

        let entries = list_history(dir.path(), "conn", None, 100).unwrap();
        let sqls: Vec<&str> = entries.iter().map(|e| e.sql.as_str()).collect();
        assert_eq!(sqls, vec!["SELECT 2", "SELECT 1"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let history_dir = dir.path().join("history");
        let manager = HistoryManager::default();
        manager.append(&history_dir, "conn", &entry("SELECT 1", true)).unwrap();

        let dir_mode = fs::metadata(&history_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        let file_mode = fs::metadata(history_dir.join("conn.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);

        // ローテーション後もパーミッションが維持される
        for i in 0..10 {
            manager
                .append_with_limits(&history_dir, "conn", &entry(&format!("S{i}"), true), 5, 3)
                .unwrap();
        }
        let file_mode = fs::metadata(history_dir.join("conn.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn test_invalid_connection_name() {
        let dir = tempfile::tempdir().unwrap();
        let manager = HistoryManager::default();
        // パストラバーサルになる接続名は拒否する
        assert!(manager
            .append(dir.path(), "../evil", &entry("SELECT 1", true))
            .is_err());
        assert!(list_history(dir.path(), "a/b", None, 10).is_err());
        assert!(list_history(dir.path(), ".hidden", None, 10).is_err());
    }
}
