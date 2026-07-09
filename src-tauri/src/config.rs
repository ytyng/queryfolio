use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::settings::AppSettings;

/// getter command の実行タイムアウト (秒)。
/// 1Password 等の認証待ちで無限ハングするとコマンド呼び出しが固まるため必須。
const GETTER_COMMAND_TIMEOUT_SECS: u64 = 60;

/// SSH トンネル設定。sql-agent-mcp-server の config.yaml と互換。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTunnelConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub private_key_passphrase: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

/// 接続先サーバー設定。sql-agent-mcp-server の config.yaml と互換。
/// queryfolio では engine: sqlite を拡張し、schema を DB ファイルパスとして扱う。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub engine: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

/// フロントエンドに渡す接続先情報。パスワード等の機密は含めない。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInfo {
    pub name: String,
    pub description: Option<String>,
    pub engine: String,
    pub has_ssh_tunnel: bool,
}

impl From<&ServerConfig> for ConnectionInfo {
    fn from(server: &ServerConfig) -> Self {
        Self {
            name: server.name.clone(),
            description: server.description.clone(),
            engine: server.engine.clone(),
            has_ssh_tunnel: server.ssh_tunnel.is_some(),
        }
    }
}

/// 接続設定 YAML を解決してサーバー一覧を返す。
///
/// 解決順 (sql-agent-mcp-server の config_loader.py と同方針):
/// 1. QUERYFOLIO_CONFIG_YAML 環境変数 (YAML 文字列リテラル)
/// 2. QUERYFOLIO_CONFIG_YAML_GETTER_COMMAND 環境変数 (実行して stdout を使う)
/// 3. アプリ設定の config_yaml_getter_command
/// 4. アプリ設定の config_yaml_path (デフォルト ~/.config/queryfolio/config.yaml)
pub async fn load_servers(settings: &AppSettings) -> Result<Vec<ServerConfig>, AppError> {
    let (yaml_text, source) = resolve_yaml_text(settings).await?;
    parse_servers(&yaml_text, &source)
}

async fn resolve_yaml_text(settings: &AppSettings) -> Result<(String, String), AppError> {
    if let Ok(yaml) = std::env::var("QUERYFOLIO_CONFIG_YAML") {
        if !yaml.trim().is_empty() {
            return Ok((yaml, "env QUERYFOLIO_CONFIG_YAML".into()));
        }
    }

    if let Ok(command) = std::env::var("QUERYFOLIO_CONFIG_YAML_GETTER_COMMAND") {
        if !command.trim().is_empty() {
            let yaml = run_getter_command(&command).await?;
            return Ok((yaml, "env QUERYFOLIO_CONFIG_YAML_GETTER_COMMAND".into()));
        }
    }

    if let Some(command) = &settings.config_yaml_getter_command {
        if !command.trim().is_empty() {
            let yaml = run_getter_command(command).await?;
            return Ok((yaml, "settings config_yaml_getter_command".into()));
        }
    }

    let path = settings.resolve_config_yaml_path()?;
    if !path.exists() {
        return Err(AppError::Config(format!(
            "接続設定が見つかりません。{} を作成するか、設定画面で getter command を指定してください",
            path.display()
        )));
    }
    let yaml = std::fs::read_to_string(&path)?;
    Ok((yaml, path.display().to_string()))
}

/// getter command を実行して stdout を返す。
///
/// shlex で argv に分解し、シェルを介さず実行する。シェルメタ文字が混入しても
/// 解釈されないためコマンドインジェクションの余地が無い。その代わり
/// パイプ・リダイレクト・変数展開は使えない (単一コマンド前提)。
async fn run_getter_command(command: &str) -> Result<String, AppError> {
    let argv = shlex::split(command).ok_or_else(|| {
        AppError::Config(format!(
            "getter command の解析に失敗 (クォート不整合等): {command}"
        ))
    })?;
    if argv.is_empty() {
        return Err(AppError::Config("getter command が空です".into()));
    }

    let output = tokio::time::timeout(
        Duration::from_secs(GETTER_COMMAND_TIMEOUT_SECS),
        tokio::process::Command::new(&argv[0])
            .args(&argv[1..])
            // タイムアウトで future が drop された時に子プロセスを残さない
            // (認証待ちでハングした op が遺児化し、リトライで多重起動するのを防ぐ)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        AppError::Config(format!(
            "getter command がタイムアウト ({GETTER_COMMAND_TIMEOUT_SECS}秒): {command} \
             (1Password 等の認証待ちでハングしている可能性)"
        ))
    })?
    .map_err(|e| AppError::Config(format!("getter command の実行に失敗: {command}: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Config(format!(
            "getter command が異常終了 (code={:?}): {command}\nstderr: {}",
            output.status.code(),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err(AppError::Config(format!(
            "getter command の出力が空: {command}"
        )));
    }
    Ok(stdout)
}

/// YAML テキストをパースし、テンプレート展開済みのサーバー一覧を返す。
fn parse_servers(yaml_text: &str, source: &str) -> Result<Vec<ServerConfig>, AppError> {
    let doc: serde_yaml::Value = serde_yaml::from_str(yaml_text)
        .map_err(|e| AppError::Config(format!("{source} の YAML パースに失敗: {e}")))?;

    let mapping = doc
        .as_mapping()
        .ok_or_else(|| AppError::Config(format!("{source} は YAML マッピングではありません")))?;

    let templates = mapping
        .get("sql_server_templates")
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();

    let servers_value = mapping
        .get("sql_servers")
        .and_then(|v| v.as_sequence())
        .ok_or_else(|| {
            AppError::Config(format!("{source} に sql_servers がありません"))
        })?;

    let mut servers = Vec::new();
    for server_value in servers_value {
        let expanded = expand_template(server_value, &templates)?;
        let server: ServerConfig = serde_yaml::from_value(expanded).map_err(|e| {
            AppError::Config(format!("{source} の sql_servers エントリのパースに失敗: {e}"))
        })?;
        servers.push(server);
    }
    Ok(servers)
}

/// `template: <名前>` を持つサーバーエントリに、sql_server_templates の
/// 同名テンプレートをシャローマージで継承させる。
/// サーバー側で指定したキーはテンプレートの同名キーを上書きする。
fn expand_template(
    server_value: &serde_yaml::Value,
    templates: &[serde_yaml::Value],
) -> Result<serde_yaml::Value, AppError> {
    let server_map = server_value
        .as_mapping()
        .ok_or_else(|| AppError::Config("sql_servers のエントリがマッピングではありません".into()))?;

    let template_name = match server_map.get("template").and_then(|v| v.as_str()) {
        Some(name) => name.to_string(),
        None => return Ok(server_value.clone()),
    };

    let template = templates
        .iter()
        .filter_map(|t| t.as_mapping())
        .find(|t| {
            t.get("name").and_then(|v| v.as_str()) == Some(template_name.as_str())
        })
        .ok_or_else(|| {
            AppError::Config(format!(
                "sql_server_templates にテンプレート '{template_name}' がありません"
            ))
        })?;

    let mut merged = template.clone();
    // テンプレート自身の name はサーバー名ではないので除去する
    merged.remove("name");
    for (key, value) in server_map {
        if key.as_str() == Some("template") {
            continue;
        }
        merged.insert(key.clone(), value.clone());
    }
    Ok(serde_yaml::Value::Mapping(merged))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_servers_basic() {
        let yaml = r#"
sql_servers:
  - name: dev-postgres
    description: "dev"
    engine: postgres
    host: localhost
    port: 5432
    schema: dev_db
    user: dev_user
    password: secret
"#;
        let servers = parse_servers(yaml, "test").unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "dev-postgres");
        assert_eq!(servers[0].engine, "postgres");
        assert_eq!(servers[0].port, Some(5432));
        assert!(servers[0].ssh_tunnel.is_none());
    }

    #[test]
    fn test_parse_servers_with_template() {
        let yaml = r#"
sql_servers:
  - template: shared-host
    name: app-db
    schema: app_db
  - template: shared-host
    name: log-db
    schema: log_db
    port: 3307
sql_server_templates:
  - name: shared-host
    engine: mysql
    host: db.example.com
    port: 3306
    user: shared_user
    password: shared_password
"#;
        let servers = parse_servers(yaml, "test").unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "app-db");
        assert_eq!(servers[0].engine, "mysql");
        assert_eq!(servers[0].host.as_deref(), Some("db.example.com"));
        assert_eq!(servers[0].port, Some(3306));
        assert_eq!(servers[0].schema.as_deref(), Some("app_db"));
        // サーバー側の指定がテンプレートを上書きする
        assert_eq!(servers[1].port, Some(3307));
    }

    #[test]
    fn test_parse_servers_unknown_template() {
        let yaml = r#"
sql_servers:
  - template: no-such-template
    name: app-db
"#;
        let result = parse_servers(yaml, "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_servers_with_ssh_tunnel() {
        let yaml = r#"
sql_servers:
  - name: remote-db
    engine: postgres
    host: localhost
    port: 5432
    schema: remote_db
    user: remote_user
    password: remote_password
    ssh_tunnel:
      host: ssh.example.com
      user: ssh_user
      private_key_path: ~/.ssh/id_rsa
"#;
        let servers = parse_servers(yaml, "test").unwrap();
        let tunnel = servers[0].ssh_tunnel.as_ref().unwrap();
        assert_eq!(tunnel.host, "ssh.example.com");
        assert_eq!(tunnel.port, 22);
        assert_eq!(
            tunnel.private_key_path.as_deref(),
            Some("~/.ssh/id_rsa")
        );
    }

    #[test]
    fn test_connection_info_hides_password() {
        let server = ServerConfig {
            name: "s".into(),
            description: None,
            engine: "mysql".into(),
            host: Some("h".into()),
            port: Some(3306),
            schema: Some("db".into()),
            user: Some("u".into()),
            password: Some("secret".into()),
            ssh_tunnel: None,
        };
        let info = ConnectionInfo::from(&server);
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("secret"));
    }
}
