use crate::db::Engine;
use crate::error::AppError;

/// psql 風メタコマンド (\l, \dt など) をカタログ照会 SQL に変換する。
///
/// 読み取り系のカタログ照会のみ対応する。\c (接続切替) や \i (ファイル実行)
/// のような状態を持つコマンドは対象外。
/// 入力がメタコマンドでなければ None、未対応のメタコマンドはエラーを返す。
pub fn translate(engine: Engine, input: &str) -> Result<Option<String>, AppError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('\\') {
        return Ok(None);
    }
    // SQL の癖で末尾に ; を付けても動くよう、末尾のセミコロンは無視する
    let trimmed = trimmed.trim_end_matches(|c: char| c == ';' || c.is_whitespace());
    let mut parts = trimmed.split_whitespace();
    let command = parts.next().unwrap_or("");
    let arg = parts.next();

    let sql = match engine {
        Engine::Postgres => postgres_meta(command, arg)?,
        Engine::MySql => mysql_meta(command, arg)?,
        Engine::Sqlite => sqlite_meta(command, arg)?,
    };
    Ok(Some(sql))
}

/// テーブル名引数を検証する。SQL に埋め込むため、識別子として安全な文字のみ許可する。
/// クォート付き識別子 (スペースや記号入り) は非対応。
/// \d のほか、スキーマブラウザ (schema_info) のテーブル名検証にも使う。
pub(crate) fn validate_relation_name(name: &str) -> Result<&str, AppError> {
    let parts: Vec<&str> = name.split('.').collect();
    let valid = !parts.is_empty()
        && parts.len() <= 2
        && parts.iter().all(|part| {
            let mut chars = part.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        });
    if !valid {
        return Err(AppError::Config(format!(
            "Invalid table name: {name} \
             (only simple identifiers like schema.table are supported)"
        )));
    }
    Ok(name)
}

fn unsupported(command: &str, supported: &str) -> AppError {
    AppError::Config(format!(
        "Unsupported meta command: {command} (supported: {supported})"
    ))
}

fn postgres_meta(command: &str, arg: Option<&str>) -> Result<String, AppError> {
    let sql = match (command, arg) {
        ("\\l" | "\\list", _) => "SELECT d.datname AS name, \
             pg_catalog.pg_get_userbyid(d.datdba) AS owner, \
             pg_catalog.pg_encoding_to_char(d.encoding) AS encoding, \
             d.datcollate AS collate, d.datctype AS ctype \
             FROM pg_catalog.pg_database d \
             WHERE d.datistemplate = false ORDER BY 1"
            .to_string(),
        ("\\dt", _) => relation_list_sql("('r','p')"),
        ("\\dv", _) => relation_list_sql("('v','m')"),
        ("\\d", None) => relation_list_sql("('r','p','v','m','S')"),
        ("\\d", Some(name)) => {
            let name = validate_relation_name(name)?;
            format!(
                "SELECT a.attname AS column, \
                 pg_catalog.format_type(a.atttypid, a.atttypmod) AS type, \
                 CASE WHEN a.attnotnull THEN 'not null' ELSE '' END AS nullable, \
                 pg_catalog.pg_get_expr(d.adbin, d.adrelid) AS default \
                 FROM pg_catalog.pg_attribute a \
                 LEFT JOIN pg_catalog.pg_attrdef d \
                   ON a.attrelid = d.adrelid AND a.attnum = d.adnum \
                 WHERE a.attrelid = '{name}'::regclass \
                   AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum"
            )
        }
        ("\\dn", _) => "SELECT n.nspname AS name, \
             pg_catalog.pg_get_userbyid(n.nspowner) AS owner \
             FROM pg_catalog.pg_namespace n \
             WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
             ORDER BY 1"
            .to_string(),
        ("\\du", _) => "SELECT r.rolname AS role_name, r.rolsuper AS superuser, \
             r.rolcreaterole AS create_role, r.rolcreatedb AS create_db, \
             r.rolcanlogin AS can_login \
             FROM pg_catalog.pg_roles r ORDER BY 1"
            .to_string(),
        _ => {
            return Err(unsupported(
                command,
                "\\l \\list \\dt \\dv \\dn \\du \\d [table]",
            ));
        }
    };
    Ok(sql)
}

/// Postgres のリレーション一覧 SQL (relkind の集合で絞り込む)。
fn relation_list_sql(relkinds: &str) -> String {
    format!(
        "SELECT n.nspname AS schema, c.relname AS name, \
         CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned table' \
           WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view' \
           WHEN 'S' THEN 'sequence' ELSE c.relkind::text END AS type, \
         pg_catalog.pg_get_userbyid(c.relowner) AS owner \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE c.relkind IN {relkinds} \
           AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
         ORDER BY 1, 2"
    )
}

fn mysql_meta(command: &str, arg: Option<&str>) -> Result<String, AppError> {
    let sql = match (command, arg) {
        ("\\l" | "\\list", _) => "SHOW DATABASES".to_string(),
        // SHOW TABLES はビューも含むため、\dt はベーステーブルに絞る
        ("\\dt", _) => "SHOW FULL TABLES WHERE Table_type = 'BASE TABLE'".to_string(),
        ("\\d", None) => "SHOW TABLES".to_string(),
        ("\\dv", _) => "SHOW FULL TABLES WHERE Table_type = 'VIEW'".to_string(),
        ("\\d", Some(name)) => {
            let name = validate_relation_name(name)?;
            // schema.table を `schema`.`table` にクォートする
            let quoted = name
                .split('.')
                .map(|part| format!("`{part}`"))
                .collect::<Vec<_>>()
                .join(".");
            format!("DESCRIBE {quoted}")
        }
        ("\\du", _) => {
            "SELECT User AS user, Host AS host FROM mysql.user ORDER BY 1, 2".to_string()
        }
        _ => {
            return Err(unsupported(
                command,
                "\\l \\list \\dt \\dv \\du \\d [table]",
            ));
        }
    };
    Ok(sql)
}

fn sqlite_meta(command: &str, arg: Option<&str>) -> Result<String, AppError> {
    let sql = match (command, arg) {
        ("\\l" | "\\list", _) => "PRAGMA database_list".to_string(),
        ("\\dt", _) => "SELECT name, type FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name"
            .to_string(),
        ("\\dv", _) => "SELECT name, type FROM sqlite_master \
             WHERE type = 'view' ORDER BY name"
            .to_string(),
        ("\\d", None) => "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
             ORDER BY type, name"
            .to_string(),
        ("\\d", Some(name)) => {
            let name = validate_relation_name(name)?;
            format!("PRAGMA table_info(\"{name}\")")
        }
        _ => {
            return Err(unsupported(command, "\\l \\list \\dt \\dv \\d [table]"));
        }
    };
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trailing_semicolon_is_ignored() {
        let sql = translate(Engine::Postgres, "\\dt;").unwrap().unwrap();
        assert!(sql.contains("pg_catalog.pg_class"));
        let sql = translate(Engine::Postgres, "\\d users;").unwrap().unwrap();
        assert!(sql.contains("'users'::regclass"));
        let sql = translate(Engine::MySql, "\\l ;;").unwrap().unwrap();
        assert_eq!(sql, "SHOW DATABASES");
    }

    #[test]
    fn test_non_meta_returns_none() {
        assert!(translate(Engine::Postgres, "SELECT 1").unwrap().is_none());
        assert!(translate(Engine::MySql, "  SHOW TABLES").unwrap().is_none());
    }

    #[test]
    fn test_postgres_meta() {
        let sql = translate(Engine::Postgres, "\\l").unwrap().unwrap();
        assert!(sql.contains("pg_database"));
        let sql = translate(Engine::Postgres, "\\list").unwrap().unwrap();
        assert!(sql.contains("pg_database"));
        let sql = translate(Engine::Postgres, "\\dt").unwrap().unwrap();
        assert!(sql.contains("('r','p')"));
        let sql = translate(Engine::Postgres, "\\d users").unwrap().unwrap();
        assert!(sql.contains("'users'::regclass"));
        let sql = translate(Engine::Postgres, "\\d public.users")
            .unwrap()
            .unwrap();
        assert!(sql.contains("'public.users'::regclass"));
        let sql = translate(Engine::Postgres, "\\du").unwrap().unwrap();
        assert!(sql.contains("pg_roles"));
        let sql = translate(Engine::Postgres, "\\dn").unwrap().unwrap();
        assert!(sql.contains("pg_namespace"));
    }

    #[test]
    fn test_mysql_meta() {
        assert_eq!(
            translate(Engine::MySql, "\\l").unwrap().unwrap(),
            "SHOW DATABASES"
        );
        assert_eq!(
            translate(Engine::MySql, "\\dt").unwrap().unwrap(),
            "SHOW FULL TABLES WHERE Table_type = 'BASE TABLE'"
        );
        assert_eq!(
            translate(Engine::MySql, "\\d").unwrap().unwrap(),
            "SHOW TABLES"
        );
        assert_eq!(
            translate(Engine::MySql, "\\d users").unwrap().unwrap(),
            "DESCRIBE `users`"
        );
        assert_eq!(
            translate(Engine::MySql, "\\d mydb.users").unwrap().unwrap(),
            "DESCRIBE `mydb`.`users`"
        );
    }

    #[test]
    fn test_sqlite_meta() {
        let sql = translate(Engine::Sqlite, "\\dt").unwrap().unwrap();
        assert!(sql.contains("sqlite_master"));
        // _ が LIKE ワイルドカード扱いされないよう ESCAPE 句付き
        assert!(sql.contains("ESCAPE"));
        assert_eq!(
            translate(Engine::Sqlite, "\\d users").unwrap().unwrap(),
            "PRAGMA table_info(\"users\")"
        );
    }

    #[test]
    fn test_injection_is_rejected() {
        // SQL インジェクションにつながる引数は拒否される
        assert!(translate(Engine::Postgres, "\\d users'; DROP TABLE x; --").is_err());
        assert!(translate(Engine::Postgres, "\\d users'||x").is_err());
        assert!(translate(Engine::MySql, "\\d `users`").is_err());
        assert!(translate(Engine::Sqlite, "\\d a\"b").is_err());
        assert!(translate(Engine::Postgres, "\\d a.b.c").is_err());
    }

    #[test]
    fn test_unsupported_command_is_error() {
        let err = translate(Engine::Postgres, "\\c otherdb").unwrap_err();
        assert!(err.to_string().contains("Unsupported meta command"));
        assert!(translate(Engine::MySql, "\\dn").is_err());
        assert!(translate(Engine::Sqlite, "\\du").is_err());
        assert!(translate(Engine::Postgres, "\\x").is_err());
    }
}
