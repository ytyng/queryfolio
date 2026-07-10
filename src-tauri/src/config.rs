use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// ソース宣言コマンドの実行タイムアウト (秒)。
/// 1Password 等の認証待ちで無限ハングするとコマンド呼び出しが固まるため必須。
const SOURCE_COMMAND_TIMEOUT_SECS: u64 = 60;

/// ~/.config/queryfolio ディレクトリを返す。
pub fn app_config_dir() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Config("ホームディレクトリを特定できません".into()))?;
    Ok(home.join(".config").join("queryfolio"))
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

/// sql_servers のソース宣言 (マッピング形式)。
/// command / env / file のうち、ちょうど 1 つだけを指定する。
#[derive(Debug, Clone)]
enum ServersSource {
    /// サーバー定義がそのまま書かれている (リスト形式)
    Inline,
    /// コマンドを実行して stdout を YAML として使う
    Command(String),
    /// 環境変数の中身を YAML として使う
    Env(String),
    /// ファイルを読んで YAML として使う
    File(String),
}

/// フロントエンドの情報表示用。設定の解決結果 (機密を含まない)。
#[derive(Debug, Serialize)]
pub struct ConfigInfo {
    pub config_path: String,
    pub config_exists: bool,
    pub source: String,
    pub sqlfiles_dir: String,
}

/// ~/.config/queryfolio/config.yml (無ければ config.yaml) のパース結果。
///
/// トップレベルキー:
/// - sql_servers: サーバー定義リスト、またはソース宣言マッピング
/// - sql_server_templates: 接続情報の雛形 (リスト形式の時のみ有効)
/// - sqlfiles_dir: クエリファイル保存ディレクトリ (任意)
pub struct AppConfig {
    doc: serde_yaml::Mapping,
    /// 読み込んだファイルのパス。QUERYFOLIO_CONFIG_YAML 環境変数由来なら None
    source_path: Option<PathBuf>,
}

impl AppConfig {
    /// 設定をロードする。
    /// QUERYFOLIO_CONFIG_YAML 環境変数があればそれを設定ファイルの内容として
    /// 扱う (開発・テスト用オーバーライド)。無ければ config.yml / config.yaml を読む。
    pub fn load() -> Result<Self, AppError> {
        if let Ok(yaml) = std::env::var("QUERYFOLIO_CONFIG_YAML") {
            if !yaml.trim().is_empty() {
                let doc = parse_mapping(&yaml, "env QUERYFOLIO_CONFIG_YAML")?;
                return Ok(Self {
                    doc,
                    source_path: None,
                });
            }
        }

        let path = Self::find_config_path()?;
        if !path.exists() {
            return Err(AppError::Config(format!(
                "設定ファイルが見つかりません。{} を作成してください (config.example.yaml 参照)",
                path.display()
            )));
        }
        let text = std::fs::read_to_string(&path)?;
        let doc = parse_mapping(&text, &path.display().to_string())?;
        Ok(Self {
            doc,
            source_path: Some(path),
        })
    }

    /// config.yml を優先し、無ければ config.yaml、どちらも無ければ
    /// デフォルトの config.yml のパスを返す。
    fn find_config_path() -> Result<PathBuf, AppError> {
        let dir = app_config_dir()?;
        let yml = dir.join("config.yml");
        if yml.exists() {
            return Ok(yml);
        }
        let yaml = dir.join("config.yaml");
        if yaml.exists() {
            return Ok(yaml);
        }
        Ok(yml)
    }

    /// クエリファイルの保存ディレクトリを解決する。
    pub fn resolve_sqlfiles_dir(&self) -> Result<PathBuf, AppError> {
        match self.doc.get("sqlfiles_dir").and_then(|v| v.as_str()) {
            Some(dir) if !dir.trim().is_empty() => Ok(expand_tilde(dir)),
            _ => Ok(app_config_dir()?.join("sqlfiles")),
        }
    }

    fn servers_source(&self) -> Result<ServersSource, AppError> {
        let value = self.doc.get("sql_servers").ok_or_else(|| {
            AppError::Config("設定に sql_servers がありません".into())
        })?;

        if value.is_sequence() {
            return Ok(ServersSource::Inline);
        }

        let mapping = value.as_mapping().ok_or_else(|| {
            AppError::Config(
                "sql_servers はサーバー定義のリスト、またはソース宣言 (command / env / file) の\
                 マッピングで指定してください"
                    .into(),
            )
        })?;

        let mut sources = vec![];
        for (key, val) in mapping {
            let key = key.as_str().unwrap_or_default();
            let text = val.as_str().map(|s| s.to_string()).ok_or_else(|| {
                AppError::Config(format!("sql_servers.{key} は文字列で指定してください"))
            })?;
            match key {
                "command" => sources.push(ServersSource::Command(text)),
                "env" => sources.push(ServersSource::Env(text)),
                "file" => sources.push(ServersSource::File(text)),
                other => {
                    return Err(AppError::Config(format!(
                        "sql_servers に不明なキー '{other}' があります (command / env / file のみ)"
                    )));
                }
            }
        }

        match sources.len() {
            1 => Ok(sources.into_iter().next().unwrap()),
            0 => Err(AppError::Config(
                "sql_servers のソース宣言には command / env / file のいずれか 1 つを指定してください"
                    .into(),
            )),
            _ => Err(AppError::Config(
                "sql_servers のソース宣言に command / env / file を複数指定することはできません"
                    .into(),
            )),
        }
    }

    /// 接続サーバー一覧を解決する。ソース宣言の場合は取得を伴う。
    pub async fn resolve_servers(&self) -> Result<Vec<ServerConfig>, AppError> {
        match self.servers_source()? {
            ServersSource::Inline => {
                let servers = self
                    .doc
                    .get("sql_servers")
                    .and_then(|v| v.as_sequence())
                    .cloned()
                    .unwrap_or_default();
                let templates = self
                    .doc
                    .get("sql_server_templates")
                    .and_then(|v| v.as_sequence())
                    .cloned()
                    .unwrap_or_default();
                parse_server_entries(&servers, &templates, "config (inline)")
            }
            ServersSource::Command(command) => {
                let yaml = run_source_command(&command).await?;
                parse_fetched_servers(&yaml, &format!("command: {command}"))
            }
            ServersSource::Env(env_name) => {
                let yaml = std::env::var(&env_name).map_err(|_| {
                    AppError::Config(format!(
                        "環境変数 {env_name} が設定されていません \
                         (Finder / Dock から起動した GUI には shell の環境変数は渡りません)"
                    ))
                })?;
                parse_fetched_servers(&yaml, &format!("env: {env_name}"))
            }
            ServersSource::File(path) => {
                let file_path = expand_tilde(&path);
                if !file_path.exists() {
                    return Err(AppError::Config(format!(
                        "sql_servers の file が見つかりません: {}",
                        file_path.display()
                    )));
                }
                let yaml = std::fs::read_to_string(&file_path)?;
                parse_fetched_servers(&yaml, &file_path.display().to_string())
            }
        }
    }

    /// 情報表示用のサマリを返す (機密を含まない)。
    pub fn info(&self) -> Result<ConfigInfo, AppError> {
        let config_path = match &self.source_path {
            Some(path) => path.display().to_string(),
            None => "(env QUERYFOLIO_CONFIG_YAML)".to_string(),
        };
        let source = match self.servers_source() {
            Ok(ServersSource::Inline) => "inline (直書き)".to_string(),
            Ok(ServersSource::Command(command)) => format!("command: {command}"),
            Ok(ServersSource::Env(env_name)) => format!("env: {env_name}"),
            Ok(ServersSource::File(path)) => format!("file: {path}"),
            Err(e) => format!("(エラー: {e})"),
        };
        Ok(ConfigInfo {
            config_path,
            config_exists: true,
            source,
            sqlfiles_dir: self.resolve_sqlfiles_dir()?.display().to_string(),
        })
    }
}

/// 設定ファイルが読めない場合も含めて情報表示用サマリを作る。
pub fn config_info() -> ConfigInfo {
    match AppConfig::load() {
        Ok(config) => config.info().unwrap_or_else(|e| ConfigInfo {
            config_path: String::new(),
            config_exists: true,
            source: format!("(エラー: {e})"),
            sqlfiles_dir: String::new(),
        }),
        Err(e) => {
            // load 失敗には「ファイルが無い」以外に「存在するが YAML が壊れている」
            // 場合があるため、存在判定はパースの成否と独立に行う
            let (config_path, config_exists) = match AppConfig::find_config_path() {
                Ok(path) => (path.display().to_string(), path.exists()),
                Err(_) => (String::new(), false),
            };
            ConfigInfo {
                config_path,
                config_exists,
                source: format!("(エラー: {e})"),
                sqlfiles_dir: String::new(),
            }
        }
    }
}

fn parse_mapping(yaml_text: &str, source: &str) -> Result<serde_yaml::Mapping, AppError> {
    let doc: serde_yaml::Value = serde_yaml::from_str(yaml_text)
        .map_err(|e| AppError::Config(format!("{source} の YAML パースに失敗: {e}")))?;
    doc.as_mapping().cloned().ok_or_else(|| {
        AppError::Config(format!("{source} は YAML マッピングではありません"))
    })
}

/// ソース宣言で取得した YAML をパースする。
/// sql-agent-mcp-server 互換フォーマット (sql_servers リスト + sql_server_templates)。
/// 取得先でさらにソース宣言を使う再帰は禁止 (ループ防止のため深さ 1 まで)。
fn parse_fetched_servers(
    yaml_text: &str,
    source: &str,
) -> Result<Vec<ServerConfig>, AppError> {
    let mapping = parse_mapping(yaml_text, source)?;
    let servers_value = mapping.get("sql_servers").ok_or_else(|| {
        AppError::Config(format!("{source} に sql_servers がありません"))
    })?;
    let servers = servers_value.as_sequence().ok_or_else(|| {
        AppError::Config(format!(
            "{source} の sql_servers がリストではありません \
             (ソース宣言の再帰指定はできません)"
        ))
    })?;
    let templates = mapping
        .get("sql_server_templates")
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    parse_server_entries(servers, &templates, source)
}

fn parse_server_entries(
    servers: &[serde_yaml::Value],
    templates: &[serde_yaml::Value],
    source: &str,
) -> Result<Vec<ServerConfig>, AppError> {
    let mut result = Vec::new();
    for server_value in servers {
        let expanded = expand_template(server_value, templates)?;
        let server: ServerConfig = serde_yaml::from_value(expanded).map_err(|e| {
            AppError::Config(format!(
                "{source} の sql_servers エントリのパースに失敗: {e}"
            ))
        })?;
        result.push(server);
    }
    Ok(result)
}

/// ソース宣言の command を実行して stdout を返す。
///
/// shlex で argv に分解し、シェルを介さず実行する。シェルメタ文字が混入しても
/// 解釈されないためコマンドインジェクションの余地が無い。その代わり
/// パイプ・リダイレクト・変数展開は使えない (単一コマンド前提)。
async fn run_source_command(command: &str) -> Result<String, AppError> {
    let argv = shlex::split(command).ok_or_else(|| {
        AppError::Config(format!(
            "sql_servers の command の解析に失敗 (クォート不整合等): {command}"
        ))
    })?;
    if argv.is_empty() {
        return Err(AppError::Config("sql_servers の command が空です".into()));
    }

    let output = tokio::time::timeout(
        Duration::from_secs(SOURCE_COMMAND_TIMEOUT_SECS),
        tokio::process::Command::new(&argv[0])
            .args(&argv[1..])
            // Finder / Dock から起動した GUI の PATH は最小構成 (/usr/bin:/bin 等) で、
            // Homebrew の op 等が見つからないため定番パスを補う
            .env("PATH", supplemented_path())
            // タイムアウトで future が drop された時に子プロセスを残さない
            // (認証待ちでハングした op が遺児化し、リトライで多重起動するのを防ぐ)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        AppError::Config(format!(
            "sql_servers の command がタイムアウト ({SOURCE_COMMAND_TIMEOUT_SECS}秒): {command} \
             (1Password 等の認証待ちでハングしている可能性)"
        ))
    })?
    .map_err(|e| {
        AppError::Config(format!("sql_servers の command の実行に失敗: {command}: {e}"))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Config(format!(
            "sql_servers の command が異常終了 (code={:?}): {command}\nstderr: {}",
            output.status.code(),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err(AppError::Config(format!(
            "sql_servers の command の出力が空: {command}"
        )));
    }
    Ok(stdout)
}

/// PATH に Homebrew 等の定番ディレクトリを補ったものを返す。
fn supplemented_path() -> String {
    supplement_path(&std::env::var("PATH").unwrap_or_default())
}

fn supplement_path(base: &str) -> String {
    let mut path = base.to_string();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let already = base.split(':').any(|p| p == extra);
        if !already {
            if !path.is_empty() {
                path.push(':');
            }
            path.push_str(extra);
        }
    }
    path
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

    fn config_from_yaml(yaml: &str) -> AppConfig {
        AppConfig {
            doc: parse_mapping(yaml, "test").unwrap(),
            source_path: None,
        }
    }

    #[tokio::test]
    async fn test_inline_servers() {
        let config = config_from_yaml(
            r#"
sql_servers:
  - name: dev-postgres
    description: "dev"
    engine: postgres
    host: localhost
    port: 5432
    schema: dev_db
    user: dev_user
    password: secret
"#,
        );
        let servers = config.resolve_servers().await.unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "dev-postgres");
        assert_eq!(servers[0].port, Some(5432));
        assert!(servers[0].ssh_tunnel.is_none());
    }

    #[tokio::test]
    async fn test_inline_with_template() {
        let config = config_from_yaml(
            r#"
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
"#,
        );
        let servers = config.resolve_servers().await.unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].engine, "mysql");
        assert_eq!(servers[0].host.as_deref(), Some("db.example.com"));
        assert_eq!(servers[0].port, Some(3306));
        // サーバー側の指定がテンプレートを上書きする
        assert_eq!(servers[1].port, Some(3307));
    }

    #[tokio::test]
    async fn test_source_command() {
        // /bin/echo で sql-agent 互換 YAML を出力させる
        let config = config_from_yaml(
            r#"
sql_servers:
  command: '/bin/echo "sql_servers: [{name: from-command, engine: sqlite, schema: /tmp/x.db}]"'
"#,
        );
        let servers = config.resolve_servers().await.unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "from-command");
    }

    #[tokio::test]
    async fn test_source_env() {
        std::env::set_var(
            "QUERYFOLIO_TEST_SERVERS_YAML",
            "sql_servers: [{name: from-env, engine: sqlite, schema: /tmp/x.db}]",
        );
        let config = config_from_yaml(
            r#"
sql_servers:
  env: QUERYFOLIO_TEST_SERVERS_YAML
"#,
        );
        let servers = config.resolve_servers().await.unwrap();
        assert_eq!(servers[0].name, "from-env");
    }

    #[tokio::test]
    async fn test_source_file() {
        let path = std::env::temp_dir().join(format!(
            "queryfolio-config-test-{}.yaml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "sql_servers: [{name: from-file, engine: sqlite, schema: /tmp/x.db}]",
        )
        .unwrap();
        let config = config_from_yaml(&format!(
            "sql_servers:\n  file: {}",
            path.display()
        ));
        let servers = config.resolve_servers().await.unwrap();
        assert_eq!(servers[0].name, "from-file");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_source_multiple_keys_is_error() {
        let config = config_from_yaml(
            r#"
sql_servers:
  command: /bin/echo x
  file: /tmp/x.yaml
"#,
        );
        let result = config.resolve_servers().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("複数指定"));
    }

    #[tokio::test]
    async fn test_source_unknown_key_is_error() {
        let config = config_from_yaml("sql_servers:\n  url: op://x/y/z\n");
        let result = config.resolve_servers().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不明なキー"));
    }

    #[tokio::test]
    async fn test_fetched_yaml_cannot_recurse() {
        // 取得先の YAML がさらにソース宣言を持つ場合はエラー
        let config = config_from_yaml(
            r#"
sql_servers:
  command: '/bin/echo "sql_servers: {command: /bin/echo deeper}"'
"#,
        );
        let result = config.resolve_servers().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("再帰"));
    }

    #[test]
    fn test_sqlfiles_dir_default_and_custom() {
        let config = config_from_yaml("sql_servers: []\n");
        let default_dir = config.resolve_sqlfiles_dir().unwrap();
        assert!(default_dir.ends_with(".config/queryfolio/sqlfiles"));

        let config = config_from_yaml("sql_servers: []\nsqlfiles_dir: ~/my-queries\n");
        let custom = config.resolve_sqlfiles_dir().unwrap();
        assert_eq!(custom, dirs::home_dir().unwrap().join("my-queries"));
    }

    #[test]
    fn test_expand_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~/foo/bar"), home.join("foo/bar"));
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn test_supplement_path() {
        // 無ければ追加される
        let path = supplement_path("/usr/bin:/bin");
        assert!(path.split(':').any(|p| p == "/opt/homebrew/bin"));
        assert!(path.split(':').any(|p| p == "/usr/local/bin"));
        // 既にあれば重複追加しない
        let path = supplement_path("/opt/homebrew/bin:/usr/bin");
        let count = path.split(':').filter(|p| *p == "/opt/homebrew/bin").count();
        assert_eq!(count, 1);
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
