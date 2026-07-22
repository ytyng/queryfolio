//! `queryfolio://` URI と CLI 引数を解釈する共通ルーター。
//!
//! URI (deep link) と CLI サブコマンドの両方をここで [`Route`] に落とし、
//! lib.rs 側が [`Route`] をディスパッチする。今後アクションを増やす時は
//! ここに variant とパースを足すだけで URI / CLI の両方に対応できる
//! (「queryfolio:// と同様のルートで機能を追加していけるように」の要)。
//!
//! パス解決 ([`resolve_open_target`]) はセキュリティ上重要なので Tauri に依存
//! させず純粋な std だけで書き、単体テストで境界を固める。開けるのは
//! 「クエリファイル保存ディレクトリ (`sqlfiles_dir`) 直下の接続フォルダにある
//! `.sql` ファイル」だけで、`..` によるトラバーサルや保存領域外のパスは拒否する。

use std::path::{Component, Path, PathBuf};

/// URI スキーム名 (`queryfolio://...`)。
pub const URI_SCHEME: &str = "queryfolio";

/// URI / CLI から解釈されたアクション (まだ検証していない生の入力を保持する)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// SQL ファイルをパス指定で開く。`path` は未検証の生パス
    /// (`resolve_open_target` で保存領域配下かを検証してから使う)。
    OpenFile { path: String },
}

/// 開く対象のクエリファイルを、接続名と (正規化済み) ファイル名で表す。
/// フロントエンドはこの接続を選択してこのファイルを開く。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTarget {
    /// 対象ファイルが属する接続の名前 (`ServerConfig::name`)。
    pub connection: String,
    /// 開くファイル名 (`.sql` 付き。接続フォルダ内の 1 要素)。
    pub file_name: String,
}

/// ルーティング・パス解決のエラー。フロントへは Display の文字列で伝える
/// (アプリ内メッセージなので英語)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// `queryfolio://` で始まっていない。
    NotQueryfolioUri,
    /// 未知のアクション (`open` 以外)。
    UnknownAction(String),
    /// 開くパスが空。
    EmptyPath,
    /// クエリファイル保存ディレクトリの外を指している。
    OutsideSqlfilesDir,
    /// 保存ディレクトリ直下の「接続フォルダ / ファイル」の形になっていない。
    NotUnderConnectionFolder,
    /// どの接続のフォルダにも一致しないフォルダ名。
    UnknownFolder(String),
    /// ファイル名が不正 (`.sql` でない・ドット始まり等)。
    InvalidFileName(String),
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::NotQueryfolioUri => {
                write!(f, "Not a {URI_SCHEME}:// URI")
            }
            RouteError::UnknownAction(a) => write!(f, "Unknown action: {a}"),
            RouteError::EmptyPath => write!(f, "The file path is empty"),
            RouteError::OutsideSqlfilesDir => write!(
                f,
                "The path is outside the query files directory"
            ),
            RouteError::NotUnderConnectionFolder => write!(
                f,
                "The path is not a file directly under a connection folder"
            ),
            RouteError::UnknownFolder(folder) => write!(
                f,
                "No connection matches the folder: {folder}"
            ),
            RouteError::InvalidFileName(name) => {
                write!(f, "Invalid query file name: {name}")
            }
        }
    }
}

impl std::error::Error for RouteError {}

/// `queryfolio://open/<path>` 形式の URI を [`Route`] に解釈する。
///
/// アクションは `queryfolio://` の直後、最初の `/` までを取る。残りが開く対象の
/// パス (パーセントエンコードされていればデコードする)。絶対パスが渡ると
/// `queryfolio://open//abs/path.sql` のように `/` が重なるが、`open` を取り出した
/// 残り `/abs/path.sql` がそのままパスになる。
pub fn parse_uri(uri: &str) -> Result<Route, RouteError> {
    let scheme_prefix = format!("{URI_SCHEME}://");
    let rest = uri
        .strip_prefix(&scheme_prefix)
        .ok_or(RouteError::NotQueryfolioUri)?;
    // アクションは最初の `/` まで。`/` が無ければパス無し (= 空パス)。
    let (action, raw_path) = match rest.split_once('/') {
        Some((action, raw)) => (action, raw),
        None => (rest, ""),
    };
    match action {
        "open" => {
            let path = percent_decode(raw_path);
            if path.trim().is_empty() {
                return Err(RouteError::EmptyPath);
            }
            Ok(Route::OpenFile { path })
        }
        other => Err(RouteError::UnknownAction(other.to_string())),
    }
}

/// CLI 引数列 (プログラム名を除いたもの) から [`Route`] を解釈する。
///
/// `open <path>` サブコマンド形式のみ扱う。`queryfolio://` URL 引数は
/// deep-link プラグインが処理するためここでは無視する (二重処理防止)。
pub fn route_from_cli_args<S: AsRef<str>>(args: &[S]) -> Option<Route> {
    let scheme_prefix = format!("{URI_SCHEME}://");
    let args: Vec<&str> = args
        .iter()
        .map(|s| s.as_ref())
        .filter(|s| !s.starts_with(&scheme_prefix))
        .collect();
    let pos = args.iter().position(|a| *a == "open")?;
    let path = args.get(pos + 1)?;
    if path.trim().is_empty() {
        return None;
    }
    Some(Route::OpenFile {
        path: (*path).to_string(),
    })
}

/// 生パスを、保存ディレクトリ配下の接続フォルダにあるクエリファイルとして解決する。
///
/// - `sqlfiles_dir`: クエリファイル保存ディレクトリ (絶対パス想定)。
/// - `folders`: `(フォルダ名, 接続名)` の対応表 (設定順)。フォルダ名は
///   `ServerConfig::sqlfiles_folder_name()` が返すもの。
/// - `raw_path`: 開く対象の生パス (`~` / 相対パスは `home` / `cwd` で展開)。
/// - `home`: `~` 展開に使うホームディレクトリ (無ければ `~` は展開しない)。
/// - `cwd`: 相対パスの基準ディレクトリ (無ければ相対パスはそのまま)。
///
/// 成功条件: 展開・字句正規化したパスが `sqlfiles_dir/<フォルダ>/<name>.sql` の形
/// (ちょうど 2 階層) で、`<フォルダ>` が `folders` に存在し、`<name>.sql` が
/// 妥当なファイル名であること。`..` によるトラバーサルは字句正規化で潰れ、
/// 保存領域外に出れば `OutsideSqlfilesDir` になる (ファイルシステムには触れない)。
pub fn resolve_open_target(
    sqlfiles_dir: &Path,
    folders: &[(String, String)],
    raw_path: &str,
    home: Option<&Path>,
    cwd: Option<&Path>,
) -> Result<OpenTarget, RouteError> {
    let expanded = expand_path(raw_path, home, cwd);
    // 入力パスは cwd で絶対化しているので、比較する base (sqlfiles_dir) も同じ基準で
    // 絶対化する。設定で相対 sqlfiles_dir を使っていても、コピーした絶対パスと
    // 突き合わせられるようにする (相対のままだと strip_prefix が常に外れる)。
    let base_absolute = absolutize(sqlfiles_dir, cwd);
    let normalized = lexical_normalize(&expanded);
    let base = lexical_normalize(&base_absolute);

    let relative = normalized
        .strip_prefix(&base)
        .map_err(|_| RouteError::OutsideSqlfilesDir)?;

    // 保存ディレクトリ直下は「接続フォルダ / ファイル」のちょうど 2 要素。
    let components: Vec<&std::ffi::OsStr> = relative
        .components()
        .map(|c| c.as_os_str())
        .collect();
    if components.len() != 2 {
        return Err(RouteError::NotUnderConnectionFolder);
    }
    let folder = components[0].to_string_lossy().into_owned();
    let file_name = components[1].to_string_lossy().into_owned();

    let connection = folders
        .iter()
        .find(|(f, _)| *f == folder)
        .map(|(_, conn)| conn.clone())
        .ok_or(RouteError::UnknownFolder(folder))?;

    validate_sql_file_name(&file_name)?;

    Ok(OpenTarget {
        connection,
        file_name,
    })
}

/// `.sql` のクエリファイル名として妥当かを検証する
/// (query_files.rs の validate_component / normalize_file_name と同じ方針: 空・
/// ドット始まり・区切り文字を拒否し、拡張子が `.sql` であることを要求する)。
fn validate_sql_file_name(name: &str) -> Result<(), RouteError> {
    let trimmed = name.trim();
    let invalid = trimmed.is_empty()
        || trimmed.starts_with('.')
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || !trimmed.to_ascii_lowercase().ends_with(".sql");
    if invalid {
        return Err(RouteError::InvalidFileName(name.to_string()));
    }
    Ok(())
}

/// 相対パスを `cwd` で絶対化する (絶対パス・`cwd` 無しはそのまま)。
fn absolutize(path: &Path, cwd: Option<&Path>) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(cwd) = cwd {
        cwd.join(path)
    } else {
        path.to_path_buf()
    }
}

/// `~` / 相対パスを展開する (ファイルシステムには触れない字句的展開)。
fn expand_path(raw: &str, home: Option<&Path>, cwd: Option<&Path>) -> PathBuf {
    let raw = raw.trim();
    if let Some(home) = home {
        if raw == "~" {
            return home.to_path_buf();
        }
        if let Some(rest) = raw.strip_prefix("~/") {
            return home.join(rest);
        }
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else if let Some(cwd) = cwd {
        cwd.join(path)
    } else {
        path
    }
}

/// パスの `.` / `..` を字句的に解決する (シンボリックリンクは辿らない)。
/// `..` は 1 つ前の通常要素を取り除く。ルートより上には遡れない。
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // 直前の通常要素を取り除く。ルート直下ではこれ以上遡らない。
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// パーセントエンコード (`%XX`) をデコードする。不完全な `%` はそのまま残す。
/// deep link 経由のパスは空白等がエンコードされ得るため、URI パスに使う。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) =
                (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
            {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 16 進 1 桁を数値へ (それ以外は None)。
fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> Vec<(String, String)> {
        vec![
            ("db1_mysql__root".to_string(), "prod".to_string()),
            ("reporting".to_string(), "reporting-conn".to_string()),
        ]
    }

    #[test]
    fn test_parse_uri_open_absolute() {
        // 絶対パスはスキームの後に `/` が重なる形になる
        assert_eq!(
            parse_uri("queryfolio://open//home/u/.config/queryfolio/sqlfiles/reporting/a.sql"),
            Ok(Route::OpenFile {
                path: "/home/u/.config/queryfolio/sqlfiles/reporting/a.sql".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_uri_percent_decode() {
        assert_eq!(
            parse_uri("queryfolio://open//tmp/my%20query.sql"),
            Ok(Route::OpenFile {
                path: "/tmp/my query.sql".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_uri_errors() {
        assert_eq!(parse_uri("http://open/x"), Err(RouteError::NotQueryfolioUri));
        assert_eq!(
            parse_uri("queryfolio://delete/x"),
            Err(RouteError::UnknownAction("delete".to_string()))
        );
        assert_eq!(parse_uri("queryfolio://open"), Err(RouteError::EmptyPath));
        assert_eq!(parse_uri("queryfolio://open/"), Err(RouteError::EmptyPath));
    }

    #[test]
    fn test_route_from_cli_args() {
        assert_eq!(
            route_from_cli_args(&["open", "/tmp/a.sql"]),
            Some(Route::OpenFile {
                path: "/tmp/a.sql".to_string(),
            })
        );
        // queryfolio:// URL 引数は無視する (deep-link が処理する)
        assert_eq!(
            route_from_cli_args(&["queryfolio://open//tmp/a.sql"]),
            None
        );
        // open の後にパスが無ければ None
        assert_eq!(route_from_cli_args(&["open"]), None);
        // 無関係な引数だけなら None
        assert_eq!(route_from_cli_args(&["--flag", "value"]), None);
    }

    #[test]
    fn test_resolve_open_target_ok() {
        let base = Path::new("/home/u/.config/queryfolio/sqlfiles");
        let target = resolve_open_target(
            base,
            &folders(),
            "/home/u/.config/queryfolio/sqlfiles/reporting/monthly.sql",
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            target,
            OpenTarget {
                connection: "reporting-conn".to_string(),
                file_name: "monthly.sql".to_string(),
            }
        );
    }

    #[test]
    fn test_resolve_open_target_tilde_and_relative() {
        let home = Path::new("/home/u");
        let base = Path::new("~/.config/queryfolio/sqlfiles"); // base も展開される
        // base に ~ が入っていても展開して比較する
        let target = resolve_open_target(
            &expand_path("~/.config/queryfolio/sqlfiles", Some(home), None),
            &folders(),
            "~/.config/queryfolio/sqlfiles/reporting/a.sql",
            Some(home),
            None,
        )
        .unwrap();
        assert_eq!(target.connection, "reporting-conn");
        let _ = base;
    }

    #[test]
    fn test_resolve_open_target_relative_base_with_cwd() {
        // 相対 sqlfiles_dir でも、cwd で絶対化した絶対パスと突き合わせられる
        let cwd = Path::new("/work");
        let base = Path::new("queries"); // 相対
        let target = resolve_open_target(
            base,
            &folders(),
            "/work/queries/reporting/a.sql",
            None,
            Some(cwd),
        )
        .unwrap();
        assert_eq!(target.connection, "reporting-conn");
        // 相対の入力パスも同じ cwd 基準で解決される
        let target = resolve_open_target(
            base,
            &folders(),
            "queries/reporting/a.sql",
            None,
            Some(cwd),
        )
        .unwrap();
        assert_eq!(target.file_name, "a.sql");
    }

    #[test]
    fn test_resolve_open_target_traversal_rejected() {
        let base = Path::new("/data/sqlfiles");
        // .. で保存領域の外に出ようとするパスは拒否
        let err = resolve_open_target(
            base,
            &folders(),
            "/data/sqlfiles/reporting/../../../etc/passwd",
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(err, RouteError::OutsideSqlfilesDir);
    }

    #[test]
    fn test_resolve_open_target_outside() {
        let base = Path::new("/data/sqlfiles");
        assert_eq!(
            resolve_open_target(base, &folders(), "/etc/passwd", None, None)
                .unwrap_err(),
            RouteError::OutsideSqlfilesDir
        );
    }

    #[test]
    fn test_resolve_open_target_unknown_folder() {
        let base = Path::new("/data/sqlfiles");
        assert_eq!(
            resolve_open_target(
                base,
                &folders(),
                "/data/sqlfiles/unknown/a.sql",
                None,
                None,
            )
            .unwrap_err(),
            RouteError::UnknownFolder("unknown".to_string())
        );
    }

    #[test]
    fn test_resolve_open_target_too_deep() {
        let base = Path::new("/data/sqlfiles");
        // 接続フォルダの下にサブディレクトリがある = 2 階層でない
        assert_eq!(
            resolve_open_target(
                base,
                &folders(),
                "/data/sqlfiles/reporting/sub/a.sql",
                None,
                None,
            )
            .unwrap_err(),
            RouteError::NotUnderConnectionFolder
        );
        // 保存ディレクトリ直下のファイル (フォルダ無し) も拒否
        assert_eq!(
            resolve_open_target(base, &folders(), "/data/sqlfiles/a.sql", None, None)
                .unwrap_err(),
            RouteError::NotUnderConnectionFolder
        );
    }

    #[test]
    fn test_resolve_open_target_not_sql() {
        let base = Path::new("/data/sqlfiles");
        assert_eq!(
            resolve_open_target(
                base,
                &folders(),
                "/data/sqlfiles/reporting/notes.txt",
                None,
                None,
            )
            .unwrap_err(),
            RouteError::InvalidFileName("notes.txt".to_string())
        );
        // ドット始まりの隠しファイルも拒否
        assert_eq!(
            resolve_open_target(
                base,
                &folders(),
                "/data/sqlfiles/reporting/.secret.sql",
                None,
                None,
            )
            .unwrap_err(),
            RouteError::InvalidFileName(".secret.sql".to_string())
        );
    }

    #[test]
    fn test_percent_decode_incomplete() {
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz"), "a%zz");
        assert_eq!(percent_decode("%2F"), "/");
    }
}
