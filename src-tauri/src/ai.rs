use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// model 省略時に使う OpenAI のデフォルトモデル。
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-5.1";

/// base_url 省略時の OpenAI API ベース URL。
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// AI API リクエストのタイムアウト (秒)。
const AI_REQUEST_TIMEOUT_SECS: u64 = 60;

/// エラーメッセージに含める API レスポンス本文の最大長。
const ERROR_BODY_MAX_CHARS: usize = 500;

/// config.yml のトップレベル、またはソース宣言で取得した接続 YAML の
/// トップレベルに書ける `ai:` セクション。
/// api_key を含むためフロントエンドには渡さない (フロントには
/// get_ai_info で AiInfo のみを返す)。
#[derive(Debug, Clone, Deserialize)]
pub struct AiConfig {
    /// AI プロバイダー。現状 "openai" のみ対応 (省略時 "openai")
    #[serde(default = "default_provider")]
    pub provider: String,
    pub api_key: String,
    /// モデル名 (省略時 DEFAULT_OPENAI_MODEL)
    #[serde(default)]
    pub model: Option<String>,
    /// OpenAI 互換 API 用のベース URL (省略時 DEFAULT_OPENAI_BASE_URL)
    #[serde(default)]
    pub base_url: Option<String>,
}

fn default_provider() -> String {
    "openai".to_string()
}

impl AiConfig {
    /// YAML の `ai:` セクションの値をパース・検証する。
    pub fn from_value(value: &serde_yaml::Value) -> Result<Self, AppError> {
        let config: AiConfig = serde_yaml::from_value(value.clone())
            .map_err(|e| AppError::Ai(format!("Failed to parse the 'ai' section: {e}")))?;
        if config.provider != "openai" {
            return Err(AppError::Ai(format!(
                "Unsupported AI provider '{}' (only 'openai' is supported)",
                config.provider
            )));
        }
        if config.api_key.trim().is_empty() {
            return Err(AppError::Ai(
                "The 'ai' section has an empty api_key".into(),
            ));
        }
        Ok(config)
    }

    /// 使用するモデル名 (省略時はデフォルトモデル)。
    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_OPENAI_MODEL)
    }

    /// API のベース URL (省略時は OpenAI 公式。末尾スラッシュは除去)。
    fn base_url(&self) -> &str {
        self.base_url
            .as_deref()
            .unwrap_or(DEFAULT_OPENAI_BASE_URL)
            .trim_end_matches('/')
    }
}

/// ローカル config.yml と接続 YAML (ソース宣言で取得) それぞれの
/// トップレベル `ai:` セクションから AI 設定を解決する。
/// 両方ある場合は接続 YAML 側を優先する (API キーを 1Password 等の
/// 接続 YAML 側に置けるようにするため)。どちらにも無ければ None。
pub fn resolve_ai_config(
    local: Option<&serde_yaml::Value>,
    fetched: Option<&serde_yaml::Value>,
) -> Result<Option<AiConfig>, AppError> {
    match fetched.or(local) {
        Some(value) => Ok(Some(AiConfig::from_value(value)?)),
        None => Ok(None),
    }
}

/// フロントエンドに渡す AI 設定の情報。api_key は含めない。
#[derive(Debug, Serialize)]
pub struct AiInfo {
    pub configured: bool,
    pub model: String,
}

/// エンジン名を SQL 方言の表示名に変換する (プロンプト用)。
fn dialect_name(engine: &str) -> String {
    match engine.to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => "PostgreSQL".to_string(),
        "mysql" | "mariadb" => "MySQL".to_string(),
        "sqlite" | "sqlite3" => "SQLite".to_string(),
        other => other.to_string(),
    }
}

/// SQL 生成用の system prompt を組み立てる。
/// LLM に送るのはスキーマ情報 (テーブル名・カラム名) と方言・アクティブ
/// スキーマ名のみ。クエリの結果データや接続情報 (ホスト・認証情報) は
/// 絶対に含めない。
pub fn build_sql_system_prompt(
    engine: &str,
    active_schema: Option<&str>,
    schema_map: &BTreeMap<String, Vec<String>>,
) -> String {
    let dialect = dialect_name(engine);
    let mut prompt = format!(
        "You are a SQL assistant for a {dialect} database. \
         Write a single SQL statement in the {dialect} dialect that fulfills \
         the user's request, using only the tables and columns listed below.\n\
         Return ONLY the SQL statement, no markdown fences, no explanation.\n"
    );
    if let Some(schema) = active_schema.filter(|s| !s.trim().is_empty()) {
        prompt.push_str(&format!("The active schema (database) is '{schema}'.\n"));
    }
    prompt.push_str("\nTables and columns:\n");
    if schema_map.is_empty() {
        prompt.push_str("(no tables found)\n");
    }
    for (table, columns) in schema_map {
        prompt.push_str(&format!("- {table} ({})\n", columns.join(", ")));
    }
    prompt
}

/// EXPLAIN 解説用の system prompt を組み立てる。
/// LLM に送るのはスキーマ情報 (テーブル名・カラム名)・方言・アクティブ
/// スキーマ名のみ (SQL と実行計画は user message 側)。実行計画はクエリの
/// 結果データではなくプランナー出力なので送ってよい。接続情報 (ホスト・
/// 認証情報) は絶対に含めない。
pub fn build_explain_system_prompt(
    engine: &str,
    active_schema: Option<&str>,
    schema_map: &BTreeMap<String, Vec<String>>,
) -> String {
    let dialect = dialect_name(engine);
    let mut prompt = format!(
        "You are a {dialect} query performance expert. The user provides a \
         SQL statement and its execution plan ({dialect} EXPLAIN output).\n\
         Respond in Markdown with the following sections:\n\
         1. **Bottlenecks** — identify the dominant costs in the plan \
         (full scans, row estimate mismatches, expensive joins, sorts, etc.). \
         If the plan is already efficient, say so.\n\
         2. **Index suggestions** — concrete CREATE INDEX statements with a \
         short rationale, using only the tables and columns listed below. \
         If no index would help, say so.\n\
         3. **Query rewrite** — a rewritten query only if it would improve \
         the plan.\n\
         Be specific and concise. Use fenced code blocks for SQL.\n"
    );
    if let Some(schema) = active_schema.filter(|s| !s.trim().is_empty()) {
        prompt.push_str(&format!("The active schema (database) is '{schema}'.\n"));
    }
    prompt.push_str("\nTables and columns:\n");
    if schema_map.is_empty() {
        prompt.push_str("(no tables found)\n");
    }
    for (table, columns) in schema_map {
        prompt.push_str(&format!("- {table} ({})\n", columns.join(", ")));
    }
    prompt
}

/// EXPLAIN 解説用の user message (SQL + 実行計画テキスト) を組み立てる。
pub fn build_explain_user_message(sql: &str, plan_text: &str) -> String {
    format!(
        "SQL:\n```sql\n{}\n```\n\nExecution plan:\n```\n{}\n```",
        sql.trim(),
        plan_text.trim()
    )
}

/// LLM の応答が ```sql フェンス付きで返ってきた場合に中身を取り出す。
/// フェンスが無ければ前後の空白だけ除去して返す。
pub fn strip_sql_fences(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // 先頭行の言語タグ (sql 等) を読み飛ばす
        let body = match rest.split_once('\n') {
            Some((_lang, body)) => body,
            None => rest,
        };
        let body = body.strip_suffix("```").unwrap_or(body);
        return body.trim().to_string();
    }
    trimmed.to_string()
}

/// エラーメッセージ用にレスポンス本文を切り詰める。
fn truncate_for_error(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= ERROR_BODY_MAX_CHARS {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(ERROR_BODY_MAX_CHARS).collect();
    format!("{truncated}...")
}

/// OpenAI Chat Completions API を呼び、アシスタント応答のテキストを返す。
/// AI 機能 (SQL 生成 / エラー修正 / EXPLAIN 解説 等) の共通基盤。
/// 呼び出し側は目的別の Tauri コマンドとしてプロンプトを組み立てること
/// (フロントから任意プロンプトを送れる汎用コマンドは作らない)。
pub async fn chat_complete(
    config: &AiConfig,
    system: &str,
    user: &str,
) -> Result<String, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(AI_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| AppError::Ai(format!("Failed to build the HTTP client: {e}")))?;
    let url = format!("{}/chat/completions", config.base_url());
    let body = serde_json::json!({
        "model": config.model(),
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });

    let response = client
        .post(&url)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Ai(format!("The AI API request failed: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| AppError::Ai(format!("Failed to read the AI API response: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Ai(format!(
            "The AI API returned an error (HTTP {}): {}",
            status.as_u16(),
            truncate_for_error(&text)
        )));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Ai(format!("Failed to parse the AI API response: {e}")))?;
    let content = json
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| AppError::Ai("The AI API response has no message content".into()))?;
    Ok(content.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> serde_yaml::Value {
        serde_yaml::from_str(text).unwrap()
    }

    #[test]
    fn test_ai_config_from_value_full() {
        let config = AiConfig::from_value(&yaml(
            "provider: openai\napi_key: sk-test\nmodel: gpt-5.2\nbase_url: https://example.com/v1",
        ))
        .unwrap();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.api_key, "sk-test");
        assert_eq!(config.model(), "gpt-5.2");
        assert_eq!(config.base_url(), "https://example.com/v1");
    }

    #[test]
    fn test_ai_config_defaults() {
        // provider / model / base_url は省略できる
        let config = AiConfig::from_value(&yaml("api_key: sk-test")).unwrap();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model(), DEFAULT_OPENAI_MODEL);
        assert_eq!(config.base_url(), DEFAULT_OPENAI_BASE_URL);
    }

    #[test]
    fn test_ai_config_base_url_trailing_slash() {
        let config =
            AiConfig::from_value(&yaml("api_key: sk-test\nbase_url: https://example.com/v1/"))
                .unwrap();
        assert_eq!(config.base_url(), "https://example.com/v1");
    }

    #[test]
    fn test_ai_config_unknown_provider_is_error() {
        let err = AiConfig::from_value(&yaml("provider: anthropic\napi_key: sk-test"))
            .unwrap_err();
        assert!(err.to_string().contains("Unsupported AI provider"));
    }

    #[test]
    fn test_ai_config_missing_api_key_is_error() {
        assert!(AiConfig::from_value(&yaml("provider: openai")).is_err());
        let err = AiConfig::from_value(&yaml("provider: openai\napi_key: \"  \"")).unwrap_err();
        assert!(err.to_string().contains("empty api_key"));
    }

    #[test]
    fn test_resolve_ai_config_prefers_fetched() {
        // ローカル config.yml と接続 YAML の両方にあれば接続 YAML 側を使う
        let local = yaml("api_key: sk-local\nmodel: local-model");
        let fetched = yaml("api_key: sk-fetched\nmodel: fetched-model");
        let config = resolve_ai_config(Some(&local), Some(&fetched))
            .unwrap()
            .unwrap();
        assert_eq!(config.api_key, "sk-fetched");
        assert_eq!(config.model(), "fetched-model");
    }

    #[test]
    fn test_resolve_ai_config_local_only() {
        let local = yaml("api_key: sk-local");
        let config = resolve_ai_config(Some(&local), None).unwrap().unwrap();
        assert_eq!(config.api_key, "sk-local");
    }

    #[test]
    fn test_resolve_ai_config_fetched_only() {
        let fetched = yaml("api_key: sk-fetched");
        let config = resolve_ai_config(None, Some(&fetched)).unwrap().unwrap();
        assert_eq!(config.api_key, "sk-fetched");
    }

    #[test]
    fn test_resolve_ai_config_none() {
        assert!(resolve_ai_config(None, None).unwrap().is_none());
    }

    #[test]
    fn test_resolve_ai_config_invalid_fetched_is_error() {
        // 接続 YAML 側が優先されるため、そちらが不正ならローカルに
        // フォールバックせずエラーにする (誤ったキーで動き続けない)
        let local = yaml("api_key: sk-local");
        let fetched = yaml("provider: unknown\napi_key: sk-fetched");
        assert!(resolve_ai_config(Some(&local), Some(&fetched)).is_err());
    }

    #[test]
    fn test_strip_sql_fences() {
        // ```sql フェンス付き
        assert_eq!(
            strip_sql_fences("```sql\nSELECT * FROM users;\n```"),
            "SELECT * FROM users;"
        );
        // 言語タグ無しのフェンス
        assert_eq!(strip_sql_fences("```\nSELECT 1;\n```"), "SELECT 1;");
        // 1 行フェンス
        assert_eq!(strip_sql_fences("```SELECT 1```"), "SELECT 1");
        // フェンス無しは前後の空白のみ除去
        assert_eq!(strip_sql_fences("  SELECT 1;\n"), "SELECT 1;");
        // 閉じフェンスが無い場合も先頭フェンスは剥がす
        assert_eq!(strip_sql_fences("```sql\nSELECT 1;"), "SELECT 1;");
        // 複数行の SQL は中の改行を保持する
        assert_eq!(
            strip_sql_fences("```sql\nSELECT a\nFROM t;\n```"),
            "SELECT a\nFROM t;"
        );
    }

    #[test]
    fn test_build_sql_system_prompt() {
        let mut schema_map = BTreeMap::new();
        schema_map.insert(
            "users".to_string(),
            vec!["id".to_string(), "name".to_string()],
        );
        schema_map.insert("orders".to_string(), vec!["id".to_string()]);
        let prompt = build_sql_system_prompt("postgres", Some("app_db"), &schema_map);
        assert!(prompt.contains("PostgreSQL"));
        assert!(prompt.contains("'app_db'"));
        assert!(prompt.contains("- users (id, name)"));
        assert!(prompt.contains("- orders (id)"));
        assert!(prompt.contains("Return ONLY the SQL statement"));
    }

    #[test]
    fn test_build_sql_system_prompt_no_schema() {
        // アクティブスキーマ無し・テーブル無しでも壊れないこと
        let prompt = build_sql_system_prompt("sqlite", None, &BTreeMap::new());
        assert!(prompt.contains("SQLite"));
        assert!(prompt.contains("(no tables found)"));
        assert!(!prompt.contains("active schema"));
        // 空文字のスキーマ名は含めない
        let prompt = build_sql_system_prompt("mysql", Some(""), &BTreeMap::new());
        assert!(prompt.contains("MySQL"));
        assert!(!prompt.contains("active schema"));
    }

    #[test]
    fn test_build_explain_system_prompt() {
        let mut schema_map = BTreeMap::new();
        schema_map.insert(
            "users".to_string(),
            vec!["id".to_string(), "name".to_string()],
        );
        let prompt = build_explain_system_prompt("postgres", Some("app_db"), &schema_map);
        assert!(prompt.contains("PostgreSQL"));
        assert!(prompt.contains("'app_db'"));
        assert!(prompt.contains("- users (id, name)"));
        assert!(prompt.contains("Bottlenecks"));
        assert!(prompt.contains("Index suggestions"));
        assert!(prompt.contains("Query rewrite"));
        // アクティブスキーマ無し・テーブル無しでも壊れないこと
        let prompt = build_explain_system_prompt("sqlite", None, &BTreeMap::new());
        assert!(prompt.contains("SQLite"));
        assert!(prompt.contains("(no tables found)"));
        assert!(!prompt.contains("active schema"));
    }

    #[test]
    fn test_build_explain_user_message() {
        let message = build_explain_user_message(
            "EXPLAIN QUERY PLAN\nSELECT * FROM t\n",
            "id\tparent\tdetail\n2\t0\tSCAN t\n",
        );
        assert!(message.contains("SQL:\n```sql\nEXPLAIN QUERY PLAN\nSELECT * FROM t\n```"));
        assert!(message.contains("Execution plan:\n```\nid\tparent\tdetail\n2\t0\tSCAN t\n```"));
    }

    #[test]
    fn test_truncate_for_error() {
        assert_eq!(truncate_for_error(" short "), "short");
        let long = "x".repeat(ERROR_BODY_MAX_CHARS + 100);
        let truncated = truncate_for_error(&long);
        assert!(truncated.chars().count() == ERROR_BODY_MAX_CHARS + 3);
        assert!(truncated.ends_with("..."));
    }
}
