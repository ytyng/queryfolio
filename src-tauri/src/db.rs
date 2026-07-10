use std::collections::HashMap;
use std::time::Instant;

use base64::Engine as _;
use futures::TryStreamExt;
use serde::Serialize;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlRow};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Executor, Row, TypeInfo};

use crate::config::ServerConfig;
use crate::error::AppError;
use crate::config::expand_tilde;
use crate::tunnel::SshTunnel;

/// 1 回のクエリで取得する行数の上限デフォルト。
pub const DEFAULT_MAX_ROWS: usize = 1000;

const POOL_MAX_CONNECTIONS: u32 = 3;
const ACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Clone)]
pub enum DbPool {
    MySql(sqlx::MySqlPool),
    Postgres(sqlx::PgPool),
    Sqlite(sqlx::SqlitePool),
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub affected_rows: Option<u64>,
    pub truncated: bool,
    pub elapsed_ms: u64,
    /// 自動付与した LIMIT の値 (付与していなければ None)
    pub applied_limit: Option<u64>,
}

/// 接続名ごとのプールと SSH トンネルを保持するマネージャ。
/// 単一ユーザーのデスクトップアプリなので、プール取得全体を 1 つの
/// tokio Mutex で直列化して二重生成を防ぐ。
#[derive(Default)]
pub struct DbManager {
    inner: tokio::sync::Mutex<DbManagerInner>,
}

#[derive(Default)]
struct DbManagerInner {
    pools: HashMap<String, DbPool>,
    tunnels: HashMap<String, SshTunnel>,
    /// 接続名ごとのアクティブスキーマ (database) のオーバーライド。
    /// 設定の schema と異なる database に切り替えている時のみ存在する。
    schema_overrides: HashMap<String, String>,
}

impl DbManager {
    pub async fn get_pool(&self, server: &ServerConfig) -> Result<DbPool, AppError> {
        let mut inner = self.inner.lock().await;
        if let Some(pool) = inner.pools.get(&server.name) {
            return Ok(pool.clone());
        }

        // アクティブスキーマが切り替えられていれば接続先 database を差し替える
        let mut server = server.clone();
        if let Some(schema) = inner.schema_overrides.get(&server.name) {
            server.schema = Some(schema.clone());
        }
        let server = &server;

        let engine = parse_engine(&server.engine)?;

        // SSH トンネルが必要なら先に確立し、接続先をローカルポートに差し替える
        let (host, port) = match (&server.ssh_tunnel, engine) {
            (Some(_), Engine::Sqlite) => {
                return Err(AppError::Config(
                    "ssh_tunnel cannot be used with sqlite".into(),
                ));
            }
            (Some(tunnel_config), _) => {
                // スキーマ切替等でプールだけ破棄された場合、既存トンネルは
                // 接続先ホストが同じなのでそのまま再利用する
                let local_port = match inner.tunnels.get(&server.name) {
                    Some(tunnel) => tunnel.local_port,
                    None => {
                        let target_host =
                            server.host.clone().unwrap_or_else(|| "localhost".into());
                        let target_port = server.port.unwrap_or(default_port(engine));
                        let tunnel_config = tunnel_config.clone();
                        // ssh2 は blocking なので spawn_blocking で実行する
                        let tunnel = tokio::task::spawn_blocking(move || {
                            SshTunnel::start(&tunnel_config, &target_host, target_port)
                        })
                        .await
                        .map_err(|e| {
                            AppError::SshTunnel(format!("SSH tunnel task failed: {e}"))
                        })??;
                        let local_port = tunnel.local_port;
                        inner.tunnels.insert(server.name.clone(), tunnel);
                        local_port
                    }
                };
                ("127.0.0.1".to_string(), local_port)
            }
            (None, _) => (
                server.host.clone().unwrap_or_else(|| "localhost".into()),
                server.port.unwrap_or(default_port(engine)),
            ),
        };

        let pool = connect(server, engine, &host, port).await?;
        inner.pools.insert(server.name.clone(), pool.clone());
        Ok(pool)
    }

    /// プールとトンネルを全て破棄する。設定リロード時に呼ぶ。
    pub async fn reset(&self) {
        let mut inner = self.inner.lock().await;
        inner.pools.clear();
        inner.tunnels.clear();
        inner.schema_overrides.clear();
    }

    /// 接続のアクティブスキーマ (database) を切り替える。
    /// プールを破棄し、次のクエリから新しい database で接続し直す
    /// (SQL の USE ではなくプール再構築で切り替えることで、プール内の
    /// コネクション間でセッション状態が食い違うのを防ぐ)。
    /// SSH トンネルは接続先ホストが変わらないため維持する。
    pub async fn set_schema_override(&self, connection: &str, schema: String) {
        let mut inner = self.inner.lock().await;
        inner.schema_overrides.insert(connection.to_string(), schema);
        inner.pools.remove(connection);
    }

    /// 接続のアクティブスキーマのオーバーライドを返す (無ければ None)。
    pub async fn schema_override(&self, connection: &str) -> Option<String> {
        self.inner.lock().await.schema_overrides.get(connection).cloned()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Engine {
    MySql,
    Postgres,
    Sqlite,
}

impl DbPool {
    pub(crate) fn engine(&self) -> Engine {
        match self {
            DbPool::MySql(_) => Engine::MySql,
            DbPool::Postgres(_) => Engine::Postgres,
            DbPool::Sqlite(_) => Engine::Sqlite,
        }
    }
}

fn parse_engine(engine: &str) -> Result<Engine, AppError> {
    match engine.to_ascii_lowercase().as_str() {
        "mysql" | "mariadb" => Ok(Engine::MySql),
        "postgres" | "postgresql" => Ok(Engine::Postgres),
        "sqlite" | "sqlite3" => Ok(Engine::Sqlite),
        other => Err(AppError::Config(format!(
            "Unsupported engine: {other} (supported: mysql / postgres / sqlite)"
        ))),
    }
}

fn default_port(engine: Engine) -> u16 {
    match engine {
        Engine::MySql => 3306,
        Engine::Postgres => 5432,
        Engine::Sqlite => 0,
    }
}

async fn connect(
    server: &ServerConfig,
    engine: Engine,
    host: &str,
    port: u16,
) -> Result<DbPool, AppError> {
    match engine {
        Engine::MySql => {
            let mut options = MySqlConnectOptions::new().host(host).port(port);
            if let Some(user) = &server.user {
                options = options.username(user);
            }
            if let Some(password) = &server.password {
                options = options.password(password);
            }
            if let Some(schema) = &server.schema {
                options = options.database(schema);
            }
            let pool = MySqlPoolOptions::new()
                .max_connections(POOL_MAX_CONNECTIONS)
                .acquire_timeout(ACQUIRE_TIMEOUT)
                .connect_with(options)
                .await?;
            Ok(DbPool::MySql(pool))
        }
        Engine::Postgres => {
            let mut options = PgConnectOptions::new().host(host).port(port);
            if let Some(user) = &server.user {
                options = options.username(user);
            }
            if let Some(password) = &server.password {
                options = options.password(password);
            }
            if let Some(schema) = &server.schema {
                options = options.database(schema);
            }
            let pool = PgPoolOptions::new()
                .max_connections(POOL_MAX_CONNECTIONS)
                .acquire_timeout(ACQUIRE_TIMEOUT)
                .connect_with(options)
                .await?;
            Ok(DbPool::Postgres(pool))
        }
        Engine::Sqlite => {
            // sqlite は schema (無ければ host) を DB ファイルパスとして扱う
            let path = server
                .schema
                .as_deref()
                .or(server.host.as_deref())
                .ok_or_else(|| {
                    AppError::Config(
                        "For sqlite, set schema to the database file path".into(),
                    )
                })?;
            let file_path = expand_tilde(path);
            if !file_path.exists() {
                return Err(AppError::Config(format!(
                    "SQLite database file not found: {}",
                    file_path.display()
                )));
            }
            let options = SqliteConnectOptions::new().filename(&file_path);
            let pool = SqlitePoolOptions::new()
                .max_connections(POOL_MAX_CONNECTIONS)
                .acquire_timeout(ACQUIRE_TIMEOUT)
                .connect_with(options)
                .await?;
            Ok(DbPool::Sqlite(pool))
        }
    }
}

/// SQL を実行して結果を返す。
/// 行を返す文 (SELECT 等) は max_rows 件まで取得し、
/// それ以外 (INSERT / UPDATE 等) は affected_rows を返す。
pub async fn run_query(
    pool: &DbPool,
    sql: &str,
    max_rows: usize,
    auto_limit: Option<u64>,
) -> Result<QueryResult, AppError> {
    // psql 風メタコマンド (\l, \dt など) はカタログ照会 SQL に変換して実行する
    let translated = crate::meta_commands::translate(pool.engine(), sql)?;
    let sql = translated.as_deref().unwrap_or(sql);

    if leading_keyword(sql).is_empty() {
        return Err(AppError::Config("The SQL statement is empty".into()));
    }

    // LIMIT 未指定の SELECT にはデフォルトの LIMIT を付与する
    // (メタコマンド変換後の SQL には適用しない)
    let mut applied_limit = None;
    let limited_sql;
    let sql = match auto_limit {
        Some(limit)
            if limit > 0
                && translated.is_none()
                && should_auto_limit(sql, pool.engine()) =>
        {
            // 末尾のコメント・セミコロンを除いた本体の直後に付与する
            // (コメントの後ろに付けると LIMIT がコメントに飲み込まれる)
            let body = &sql[..scan_sql(sql, pool.engine()).body_end];
            limited_sql = format!("{body} LIMIT {limit}");
            applied_limit = Some(limit);
            limited_sql.as_str()
        }
        _ => sql,
    };
    let started = Instant::now();

    if !is_fetch_statement(sql) && !contains_returning(sql) {
        let affected = match pool {
            DbPool::MySql(p) => p.execute(sql).await?.rows_affected(),
            DbPool::Postgres(p) => p.execute(sql).await?.rows_affected(),
            DbPool::Sqlite(p) => p.execute(sql).await?.rows_affected(),
        };
        return Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            row_count: 0,
            affected_rows: Some(affected),
            truncated: false,
            elapsed_ms: started.elapsed().as_millis() as u64,
            applied_limit: None,
        });
    }

    macro_rules! fetch_rows {
        ($pool:expr, $to_json:ident) => {{
            let mut stream = sqlx::query(sql).fetch($pool);
            let mut columns: Vec<String> = vec![];
            let mut rows: Vec<Vec<serde_json::Value>> = vec![];
            let mut truncated = false;
            while let Some(row) = stream.try_next().await? {
                if columns.is_empty() {
                    columns = row
                        .columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect();
                }
                if rows.len() >= max_rows {
                    truncated = true;
                    break;
                }
                let values = (0..row.columns().len())
                    .map(|i| $to_json(&row, i))
                    .collect();
                rows.push(values);
            }
            (columns, rows, truncated)
        }};
    }

    let (mut columns, rows, truncated) = match pool {
        DbPool::MySql(p) => fetch_rows!(p, mysql_value_to_json),
        DbPool::Postgres(p) => fetch_rows!(p, pg_value_to_json),
        DbPool::Sqlite(p) => fetch_rows!(p, sqlite_value_to_json),
    };

    // 0 行の結果でも列ヘッダを表示できるよう、describe で列情報を補完する。
    // SHOW 等 prepare できない文では失敗することがあるため、エラーは無視する。
    if columns.is_empty() {
        let described: Result<Vec<String>, sqlx::Error> = match pool {
            DbPool::MySql(p) => p
                .describe(sql)
                .await
                .map(|d| d.columns().iter().map(|c| c.name().to_string()).collect()),
            DbPool::Postgres(p) => p
                .describe(sql)
                .await
                .map(|d| d.columns().iter().map(|c| c.name().to_string()).collect()),
            DbPool::Sqlite(p) => p
                .describe(sql)
                .await
                .map(|d| d.columns().iter().map(|c| c.name().to_string()).collect()),
        };
        if let Ok(names) = described {
            columns = names;
        }
    }

    Ok(QueryResult {
        row_count: rows.len(),
        columns,
        rows,
        affected_rows: None,
        truncated,
        elapsed_ms: started.elapsed().as_millis() as u64,
        applied_limit,
    })
}

/// 接続先サーバー上の database (スキーマ) 一覧を返す。
/// sqlite は database の概念が単一ファイルなので、設定のパスをそのまま返す。
pub async fn list_schemas(
    pool: &DbPool,
    server: &ServerConfig,
) -> Result<Vec<String>, AppError> {
    match pool {
        DbPool::Postgres(p) => {
            let rows = sqlx::query(
                "SELECT datname FROM pg_catalog.pg_database \
                 WHERE datistemplate = false ORDER BY datname",
            )
            .fetch_all(p)
            .await?;
            Ok(rows
                .iter()
                .filter_map(|row| row.try_get::<String, _>(0).ok())
                .collect())
        }
        DbPool::MySql(p) => {
            let rows = sqlx::query("SHOW DATABASES").fetch_all(p).await?;
            Ok(rows
                .iter()
                .filter_map(|row| row.try_get::<String, _>(0).ok())
                .collect())
        }
        DbPool::Sqlite(_) => {
            let path = server
                .schema
                .as_deref()
                .or(server.host.as_deref())
                .unwrap_or("main");
            Ok(vec![path.to_string()])
        }
    }
}

/// SQL の先頭キーワード (コメントを除く) を小文字で返す。
fn leading_keyword(sql: &str) -> String {
    let mut rest = sql;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map(|(_, r)| r).unwrap_or("");
            continue;
        }
        if let Some(after) = rest.strip_prefix('#') {
            rest = after.split_once('\n').map(|(_, r)| r).unwrap_or("");
            continue;
        }
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map(|(_, r)| r).unwrap_or("");
            continue;
        }
        break;
    }
    rest.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// SQL の走査結果。cleaned はキーワード判定用 (文字列リテラルとコメントを
/// 空白化して小文字化したもの)、body_end は自動 LIMIT の挿入位置
/// (末尾のコメント・セミコロン・空白を除いた本体の終了位置)。
struct SqlScan {
    cleaned: String,
    body_end: usize,
}

/// エンジンごとのコメント・クォート規則で SQL を 1 パス走査する。
/// - 文字列リテラル: ' " ` (二重化エスケープ対応)。Postgres はドル引用
///   ($tag$ ... $tag$) にも対応 (# は Postgres では XOR 演算子なので
///   コメント扱いしない)
/// - コメント: -- と /* */。MySQL は # 行コメントも対象
fn scan_sql(sql: &str, engine: Engine) -> SqlScan {
    let hash_comments = matches!(engine, Engine::MySql);
    let dollar_quotes = matches!(engine, Engine::Postgres);
    let chars: Vec<char> = sql.chars().collect();
    let mut cleaned = String::with_capacity(sql.len());
    let mut body_end = 0;
    let mut byte_pos = 0;
    let mut i = 0;

    // i 番目の文字を消費して byte 位置を進める
    macro_rules! advance {
        () => {{
            byte_pos += chars[i].len_utf8();
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' || c == '`' {
            advance!();
            while i < chars.len() {
                let inner = chars[i];
                advance!();
                if inner == c {
                    if i < chars.len() && chars[i] == c {
                        advance!();
                        continue;
                    }
                    break;
                }
            }
            cleaned.push(' ');
            body_end = byte_pos;
        } else if dollar_quotes && c == '$' {
            // $tag$ ... $tag$ のドル引用を検出する
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if j < chars.len() && chars[j] == '$' {
                let tag: String = chars[i..=j].iter().collect();
                let tag_chars = j - i + 1;
                for _ in 0..tag_chars {
                    advance!();
                }
                // 閉じタグを探す
                loop {
                    if i >= chars.len() {
                        break;
                    }
                    if chars[i] == '$' && chars[i..].starts_with(&tag.chars().collect::<Vec<_>>()[..]) {
                        for _ in 0..tag_chars {
                            advance!();
                        }
                        break;
                    }
                    advance!();
                }
                cleaned.push(' ');
                body_end = byte_pos;
            } else {
                cleaned.push('$');
                advance!();
                body_end = byte_pos;
            }
        } else if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            while i < chars.len() && chars[i] != '\n' {
                advance!();
            }
            cleaned.push(' ');
        } else if hash_comments && c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                advance!();
            }
            cleaned.push(' ');
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            advance!();
            advance!();
            while i < chars.len() {
                if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    advance!();
                    advance!();
                    break;
                }
                advance!();
            }
            cleaned.push(' ');
        } else {
            let is_code = !c.is_whitespace() && c != ';';
            cleaned.push(c.to_ascii_lowercase());
            advance!();
            if is_code {
                body_end = byte_pos;
            }
        }
    }
    SqlScan { cleaned, body_end }
}

/// デフォルト LIMIT を安全に付与できる文かを判定する。
/// 対象は SELECT 系のみ。LIMIT / FETCH / OFFSET / FOR UPDATE / INTO /
/// WITH ... INSERT 等の語を含む場合は、構文エラーや意味の変化を避けるため
/// 付与しない (保守的側に倒す。スキップしてもクライアント側の max_rows
/// 打ち切りが安全網になる)。
fn should_auto_limit(sql: &str, engine: Engine) -> bool {
    // VALUES (SQLite では LIMIT 不可) や TABLE は対象にせず、
    // SELECT / WITH のみに限定する
    if !matches!(leading_keyword(sql).as_str(), "select" | "with") {
        return false;
    }
    let cleaned = scan_sql(sql, engine).cleaned;
    let veto_words = [
        "limit", "fetch", "offset", "for", "into", "insert", "update", "delete",
        "lock", "returning",
    ];
    !cleaned
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| veto_words.contains(&word))
}

/// RETURNING 句を含むかを単語境界で判定する。
/// INSERT / UPDATE / DELETE ... RETURNING (Postgres / SQLite) の結果行を
/// 取りこぼさないための判定。文字列リテラル内の単語にも反応する可能性が
/// あるが、その場合も fetch 経路で正しく実行される (affected 表示が
/// 行数表示になるだけ) ため許容する。
fn contains_returning(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut start = 0;
    while let Some(pos) = lower[start..].find("returning") {
        let begin = start + pos;
        let end = begin + "returning".len();
        let before_ok = begin == 0
            || !(bytes[begin - 1].is_ascii_alphanumeric() || bytes[begin - 1] == b'_');
        let after_ok = end == bytes.len()
            || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

/// 行を返す文かどうかを先頭キーワードで判定する。
fn is_fetch_statement(sql: &str) -> bool {
    matches!(
        leading_keyword(sql).as_str(),
        "select"
            | "with"
            | "show"
            | "describe"
            | "desc"
            | "explain"
            | "pragma"
            | "values"
            | "table"
            | "call"
    )
}

fn bytes_to_json(bytes: Vec<u8>) -> serde_json::Value {
    match String::from_utf8(bytes) {
        Ok(s) => serde_json::Value::String(s),
        Err(e) => serde_json::Value::String(format!(
            "base64:{}",
            base64::engine::general_purpose::STANDARD.encode(e.as_bytes())
        )),
    }
}

/// 指定した型でのデコードを試み、成功したら JSON にして返すマクロ。
/// NULL は JSON null になる。型不一致は次の候補へフォールスルーする。
macro_rules! try_decode {
    ($row:expr, $i:expr, $t:ty, $conv:expr) => {
        match $row.try_get::<Option<$t>, _>($i) {
            Ok(Some(v)) => {
                #[allow(clippy::redundant_closure_call)]
                return ($conv)(v);
            }
            Ok(None) => return serde_json::Value::Null,
            Err(_) => {}
        }
    };
}

/// どの型でもデコードできなかった場合の最終フォールバック。
macro_rules! decode_fallback {
    ($row:expr, $i:expr) => {{
        try_decode!($row, $i, String, |v: String| serde_json::Value::String(v));
        try_decode!($row, $i, Vec<u8>, bytes_to_json);
        let type_name = $row.column($i).type_info().name().to_string();
        serde_json::Value::String(format!("<undecodable: {type_name}>"))
    }};
}

fn json_number_f64(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v)
        .map(serde_json::Value::Number)
        .unwrap_or_else(|| serde_json::Value::String(v.to_string()))
}

/// JavaScript の Number は 2^53-1 (MAX_SAFE_INTEGER) を超える整数を
/// 表現できず、Tauri の invoke 境界で丸められてしまう。
/// 安全範囲を超える 64bit 整数は文字列で返して精度を保つ。
const JS_MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

fn json_i64(v: i64) -> serde_json::Value {
    if (-JS_MAX_SAFE_INTEGER..=JS_MAX_SAFE_INTEGER).contains(&v) {
        serde_json::json!(v)
    } else {
        serde_json::Value::String(v.to_string())
    }
}

fn json_u64(v: u64) -> serde_json::Value {
    if v <= JS_MAX_SAFE_INTEGER as u64 {
        serde_json::json!(v)
    } else {
        serde_json::Value::String(v.to_string())
    }
}

fn format_naive_datetime(v: chrono::NaiveDateTime) -> serde_json::Value {
    serde_json::Value::String(v.format("%Y-%m-%d %H:%M:%S%.f").to_string())
}

fn mysql_value_to_json(row: &MySqlRow, i: usize) -> serde_json::Value {
    let type_name = row.column(i).type_info().name().to_string();
    match type_name.as_str() {
        "BOOLEAN" => {
            try_decode!(row, i, bool, |v: bool| serde_json::Value::Bool(v));
        }
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => {
            try_decode!(row, i, i64, json_i64);
        }
        // YEAR は sqlx 内部で UNSIGNED フラグ付きのため u64 側でデコードする
        "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED"
        | "INT UNSIGNED" | "BIGINT UNSIGNED" | "YEAR" => {
            try_decode!(row, i, u64, json_u64);
        }
        "FLOAT" | "DOUBLE" => {
            try_decode!(row, i, f64, json_number_f64);
        }
        "DECIMAL" => {
            // 精度を保つため文字列で返す
            try_decode!(row, i, rust_decimal::Decimal, |v: rust_decimal::Decimal| {
                serde_json::Value::String(v.to_string())
            });
        }
        "DATE" => {
            try_decode!(row, i, chrono::NaiveDate, |v: chrono::NaiveDate| {
                serde_json::Value::String(v.format("%Y-%m-%d").to_string())
            });
        }
        "TIME" => {
            try_decode!(row, i, chrono::NaiveTime, |v: chrono::NaiveTime| {
                serde_json::Value::String(v.format("%H:%M:%S%.f").to_string())
            });
        }
        "DATETIME" => {
            try_decode!(row, i, chrono::NaiveDateTime, format_naive_datetime);
        }
        "TIMESTAMP" => {
            try_decode!(
                row,
                i,
                chrono::DateTime<chrono::Utc>,
                |v: chrono::DateTime<chrono::Utc>| serde_json::Value::String(
                    v.to_rfc3339()
                )
            );
        }
        "JSON" => {
            try_decode!(row, i, serde_json::Value, |v| v);
        }
        _ => {}
    }
    decode_fallback!(row, i)
}

fn pg_value_to_json(row: &PgRow, i: usize) -> serde_json::Value {
    let type_name = row.column(i).type_info().name().to_string();
    match type_name.as_str() {
        "BOOL" => {
            try_decode!(row, i, bool, |v: bool| serde_json::Value::Bool(v));
        }
        // Postgres の数値型は型互換が厳密なため、カラム型と同じ幅でデコードする
        "INT2" => {
            try_decode!(row, i, i16, |v: i16| serde_json::json!(v));
        }
        "INT4" => {
            try_decode!(row, i, i32, |v: i32| serde_json::json!(v));
        }
        "INT8" => {
            try_decode!(row, i, i64, json_i64);
        }
        "FLOAT4" => {
            try_decode!(row, i, f32, |v: f32| json_number_f64(v as f64));
        }
        "FLOAT8" => {
            try_decode!(row, i, f64, json_number_f64);
        }
        "NUMERIC" => {
            try_decode!(row, i, rust_decimal::Decimal, |v: rust_decimal::Decimal| {
                serde_json::Value::String(v.to_string())
            });
        }
        "UUID" => {
            try_decode!(row, i, uuid::Uuid, |v: uuid::Uuid| {
                serde_json::Value::String(v.to_string())
            });
        }
        "DATE" => {
            try_decode!(row, i, chrono::NaiveDate, |v: chrono::NaiveDate| {
                serde_json::Value::String(v.format("%Y-%m-%d").to_string())
            });
        }
        "TIME" => {
            try_decode!(row, i, chrono::NaiveTime, |v: chrono::NaiveTime| {
                serde_json::Value::String(v.format("%H:%M:%S%.f").to_string())
            });
        }
        "TIMESTAMP" => {
            try_decode!(row, i, chrono::NaiveDateTime, format_naive_datetime);
        }
        "TIMESTAMPTZ" => {
            try_decode!(
                row,
                i,
                chrono::DateTime<chrono::Utc>,
                |v: chrono::DateTime<chrono::Utc>| serde_json::Value::String(
                    v.to_rfc3339()
                )
            );
        }
        "JSON" | "JSONB" => {
            try_decode!(row, i, serde_json::Value, |v| v);
        }
        "BYTEA" => {
            try_decode!(row, i, Vec<u8>, bytes_to_json);
        }
        _ => {}
    }
    decode_fallback!(row, i)
}

fn sqlite_value_to_json(row: &SqliteRow, i: usize) -> serde_json::Value {
    let type_name = row.column(i).type_info().name().to_string();
    match type_name.as_str() {
        "BOOLEAN" => {
            try_decode!(row, i, bool, |v: bool| serde_json::Value::Bool(v));
        }
        "INTEGER" | "INT" => {
            try_decode!(row, i, i64, json_i64);
        }
        "REAL" => {
            try_decode!(row, i, f64, json_number_f64);
        }
        "TEXT" | "DATE" | "DATETIME" | "TIME" => {
            try_decode!(row, i, String, |v: String| serde_json::Value::String(v));
        }
        "BLOB" => {
            try_decode!(row, i, Vec<u8>, bytes_to_json);
        }
        "NUMERIC" => {
            try_decode!(row, i, i64, json_i64);
            try_decode!(row, i, f64, json_number_f64);
        }
        _ => {}
    }
    // sqlite は動的型付けのため、宣言型と実値が一致しないことがある
    try_decode!(row, i, i64, json_i64);
    try_decode!(row, i, f64, json_number_f64);
    decode_fallback!(row, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leading_keyword() {
        assert_eq!(leading_keyword("SELECT 1"), "select");
        assert_eq!(leading_keyword("  \n\t select 1"), "select");
        assert_eq!(leading_keyword("-- comment\nSELECT 1"), "select");
        assert_eq!(leading_keyword("/* c1 */ /* c2 */ UPDATE t SET a=1"), "update");
        assert_eq!(leading_keyword("# mysql comment\nSHOW TABLES"), "show");
        assert_eq!(leading_keyword(""), "");
        assert_eq!(leading_keyword("-- only comment"), "");
    }

    #[test]
    fn test_is_fetch_statement() {
        assert!(is_fetch_statement("SELECT * FROM t"));
        assert!(is_fetch_statement("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(is_fetch_statement("SHOW TABLES"));
        assert!(is_fetch_statement("EXPLAIN SELECT 1"));
        assert!(is_fetch_statement("PRAGMA table_info(t)"));
        assert!(!is_fetch_statement("INSERT INTO t VALUES (1)"));
        assert!(!is_fetch_statement("UPDATE t SET a = 1"));
        assert!(!is_fetch_statement("DELETE FROM t"));
        assert!(!is_fetch_statement("CREATE TABLE t (a int)"));
    }

    #[test]
    fn test_json_64bit_precision() {
        // JS の安全整数範囲内は数値のまま
        assert_eq!(json_i64(42), serde_json::json!(42));
        assert_eq!(json_i64(-9007199254740991), serde_json::json!(-9007199254740991i64));
        assert_eq!(json_u64(9007199254740991), serde_json::json!(9007199254740991u64));
        // 範囲外は文字列で精度を保つ
        assert_eq!(
            json_i64(i64::MAX),
            serde_json::Value::String("9223372036854775807".into())
        );
        assert_eq!(
            json_i64(i64::MIN),
            serde_json::Value::String("-9223372036854775808".into())
        );
        assert_eq!(
            json_u64(u64::MAX),
            serde_json::Value::String("18446744073709551615".into())
        );
    }

    #[test]
    fn test_scan_sql_cleaned() {
        let scan = |s: &str, e| scan_sql(s, e).cleaned;
        assert_eq!(scan("SELECT 'limit' FROM t", Engine::Sqlite), "select   from t");
        assert_eq!(scan("SELECT a -- limit\nFROM t", Engine::Sqlite), "select a  \nfrom t");
        assert_eq!(scan("SELECT /* limit */ a", Engine::Sqlite), "select   a");
        assert_eq!(scan("SELECT 'it''s' FROM t", Engine::Sqlite), "select   from t");
        // MySQL の # 行コメント
        assert_eq!(scan("SELECT a # limit\nFROM t", Engine::MySql), "select a  \nfrom t");
        // Postgres では # は演算子なのでコメント扱いしない
        assert_eq!(scan("SELECT a # b", Engine::Postgres), "select a # b");
        // Postgres のドル引用は文字列として除去
        assert_eq!(
            scan("SELECT $$--not a comment$$ AS s", Engine::Postgres),
            "select   as s"
        );
        assert_eq!(
            scan("SELECT $fn$limit$fn$ AS s", Engine::Postgres),
            "select   as s"
        );
    }

    #[test]
    fn test_scan_sql_body_end() {
        fn body(s: &str, e: Engine) -> &str {
            &s[..scan_sql(s, e).body_end]
        }
        assert_eq!(body("SELECT * FROM t -- note", Engine::Sqlite), "SELECT * FROM t");
        assert_eq!(body("SELECT 1; -- note", Engine::Sqlite), "SELECT 1");
        assert_eq!(body("SELECT 1 /* c */  ;  ", Engine::Sqlite), "SELECT 1");
        // 文字列リテラル内の記号はコードとして残る
        assert_eq!(
            body("SELECT 'a;-- b' FROM t;", Engine::Sqlite),
            "SELECT 'a;-- b' FROM t"
        );
        // コメントの後に続きがあるケース
        assert_eq!(body("SELECT 1 -- c\n+ 2", Engine::Sqlite), "SELECT 1 -- c\n+ 2");
        // MySQL の # コメントも除去される
        assert_eq!(
            body("SELECT * FROM t # inspect", Engine::MySql),
            "SELECT * FROM t"
        );
        // Postgres のドル引用内の -- は切らない
        assert_eq!(
            body("SELECT $$--not a comment$$ AS s", Engine::Postgres),
            "SELECT $$--not a comment$$ AS s"
        );
    }

    #[test]
    fn test_should_auto_limit() {
        let f = |s: &str| should_auto_limit(s, Engine::Sqlite);
        assert!(f("SELECT * FROM users"));
        assert!(f("WITH x AS (SELECT 1) SELECT * FROM x"));
        // リテラル内の limit は無視して付与できる
        assert!(f("SELECT 'limit' FROM t"));
        // 単語境界: limits というテーブル名は veto しない
        assert!(f("SELECT * FROM limits"));
        // 既に LIMIT / FETCH / OFFSET がある
        assert!(!f("SELECT * FROM t LIMIT 10"));
        assert!(!f("SELECT * FROM t FETCH FIRST 10 ROWS ONLY"));
        assert!(!f("SELECT * FROM t OFFSET 5"));
        // サブクエリ内の LIMIT も保守的にスキップ
        assert!(!f("SELECT * FROM (SELECT 1 LIMIT 3) s"));
        // ロック句・DML 混じりの WITH
        assert!(!f("SELECT * FROM t FOR UPDATE"));
        assert!(!f("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x"));
        // SELECT 系以外
        assert!(!f("SHOW TABLES"));
        assert!(!f("UPDATE t SET a = 1"));
        // VALUES は SQLite で LIMIT 不可のため対象外
        assert!(!f("VALUES (1)"));
        // Postgres: ドル引用内の limit は veto しない
        assert!(should_auto_limit(
            "SELECT $$limit$$ AS s",
            Engine::Postgres
        ));
        // MySQL: # コメント内の limit は veto しない (本体には付与できる)
        assert!(should_auto_limit(
            "SELECT * FROM t # limit note",
            Engine::MySql
        ));
    }

    #[test]
    fn test_contains_returning() {
        assert!(contains_returning("INSERT INTO t (a) VALUES (1) RETURNING id"));
        assert!(contains_returning("DELETE FROM t returning *"));
        assert!(contains_returning("UPDATE t SET a=1\nRETURNING a"));
        assert!(!contains_returning("SELECT returning_flag FROM t"));
        assert!(!contains_returning("SELECT * FROM returnings"));
        assert!(!contains_returning("UPDATE t SET a = 1"));
    }

    #[test]
    fn test_parse_engine() {
        assert!(parse_engine("mysql").is_ok());
        assert!(parse_engine("MySQL").is_ok());
        assert!(parse_engine("postgres").is_ok());
        assert!(parse_engine("postgresql").is_ok());
        assert!(parse_engine("sqlite").is_ok());
        assert!(parse_engine("oracle").is_err());
    }

    #[test]
    fn test_bytes_to_json() {
        assert_eq!(
            bytes_to_json(b"hello".to_vec()),
            serde_json::Value::String("hello".into())
        );
        let binary = vec![0xff, 0xfe, 0x00];
        let value = bytes_to_json(binary);
        assert!(value.as_str().unwrap().starts_with("base64:"));
    }

    #[tokio::test]
    async fn test_run_query_sqlite() {
        // :memory: はコネクションごとに別 DB になるため、プールを 1 接続に固定する
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(":memory:")
                    .in_memory(true),
            )
            .await
            .unwrap();
        let pool = DbPool::Sqlite(pool);

        let result = run_query(
            &pool,
            "CREATE TABLE t (id INTEGER, name TEXT, score REAL)",
            10,
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.affected_rows, Some(0));

        let result = run_query(
            &pool,
            "INSERT INTO t VALUES (1, 'alice', 1.5), (2, 'bob', NULL)",
            10,
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.affected_rows, Some(2));

        let result = run_query(&pool, "SELECT * FROM t ORDER BY id", 10, None)
            .await
            .unwrap();
        assert_eq!(result.columns, vec!["id", "name", "score"]);
        assert_eq!(result.row_count, 2);
        assert_eq!(result.rows[0][0], serde_json::json!(1));
        assert_eq!(result.rows[0][1], serde_json::json!("alice"));
        assert_eq!(result.rows[0][2], serde_json::json!(1.5));
        assert_eq!(result.rows[1][2], serde_json::Value::Null);
        assert!(!result.truncated);

        // max_rows での切り詰め
        let result = run_query(&pool, "SELECT * FROM t ORDER BY id", 1, None)
            .await
            .unwrap();
        assert_eq!(result.row_count, 1);
        assert!(result.truncated);

        // 0 行の SELECT でも列ヘッダが返る (describe による補完)
        let result = run_query(&pool, "SELECT * FROM t WHERE id = -1", 10, None)
            .await
            .unwrap();
        assert_eq!(result.row_count, 0);
        assert_eq!(result.columns, vec!["id", "name", "score"]);

        // INSERT ... RETURNING は行を返す
        let result = run_query(
            &pool,
            "INSERT INTO t VALUES (3, 'dave', 2.0) RETURNING id, name",
            10,
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.rows[0][1], serde_json::json!("dave"));

        // psql 風メタコマンドが変換されて実行される
        let result = run_query(&pool, "\\dt", 10, None).await.unwrap();
        assert_eq!(result.row_count, 1);
        assert_eq!(result.rows[0][0], serde_json::json!("t"));

        let result = run_query(&pool, "\\d t", 10, None).await.unwrap();
        // PRAGMA table_info は name カラム (index 1) にカラム名を返す
        let column_names: Vec<&str> = result
            .rows
            .iter()
            .filter_map(|row| row[1].as_str())
            .collect();
        assert_eq!(column_names, vec!["id", "name", "score"]);

        // 未対応メタコマンドはエラー
        assert!(run_query(&pool, "\\du", 10, None).await.is_err());

        // 自動 LIMIT: LIMIT 未指定の SELECT に付与される (末尾 ; も処理)
        let result = run_query(&pool, "SELECT * FROM t ORDER BY id;", 10, Some(2))
            .await
            .unwrap();
        assert_eq!(result.row_count, 2);
        assert_eq!(result.applied_limit, Some(2));

        // 既に LIMIT がある場合は付与しない
        let result = run_query(&pool, "SELECT * FROM t LIMIT 1", 10, Some(2))
            .await
            .unwrap();
        assert_eq!(result.row_count, 1);
        assert_eq!(result.applied_limit, None);

        // メタコマンドには適用しない
        let result = run_query(&pool, "\\dt", 10, Some(2)).await.unwrap();
        assert_eq!(result.applied_limit, None);

        // 末尾コメント付きでも LIMIT がコメントに飲み込まれない
        let result = run_query(
            &pool,
            "SELECT * FROM t ORDER BY id -- trailing note",
            10,
            Some(2),
        )
        .await
        .unwrap();
        assert_eq!(result.row_count, 2);
        assert_eq!(result.applied_limit, Some(2));

        // VALUES は自動 LIMIT の対象外 (SQLite では VALUES ... LIMIT が構文エラー)
        let result = run_query(&pool, "VALUES (1), (2), (3)", 10, Some(2))
            .await
            .unwrap();
        assert_eq!(result.row_count, 3);
        assert_eq!(result.applied_limit, None);
    }
}
