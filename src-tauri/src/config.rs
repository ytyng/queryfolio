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
        .ok_or_else(|| AppError::Config("Could not determine the home directory".into()))?;
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

/// 初回起動時に自動作成する config.yml のテンプレート。
/// そのままで有効な設定 (接続 0 件) としてパースできる内容にする。
const CONFIG_TEMPLATE: &str = r#"# QueryFolio config file
# See config.example.yaml in the repository for the full format.
# https://github.com/ytyng/queryfolio

# Connection definitions. Either write servers inline, or use a source
# declaration (exactly one of command / env / file) to fetch the YAML
# from elsewhere.
#
# Inline example:
# sql_servers:
#   - name: local-sqlite
#     description: "Local SQLite file"
#     engine: sqlite
#     schema: ~/data/example.sqlite3
#   - name: dev-postgres
#     engine: postgres
#     host: localhost
#     port: 5432
#     schema: development_db
#     user: dev_user
#     password: your_password
#
# Fetch from 1Password:
# sql_servers:
#   command: op read "op://development/queryfolio/config-yaml"

sql_servers: []

# Where query files are stored (default: ~/.config/queryfolio/sqlfiles)
# sqlfiles_dir: ~/queries
#
# Query files live under <sqlfiles_dir>/<folder>/<name>.sql. The per-connection
# folder is <host>_<engine>_<schema>_<user> by default (the connection name is
# not used). Set `folder_name:` on a server to pin the folder explicitly.
"#;

/// 実在する設定ファイルのパスを返す。config.yml / config.yaml のどちらも
/// 無ければ None。
pub fn existing_config_path() -> Result<Option<PathBuf>, AppError> {
    let dir = app_config_dir()?;
    for name in ["config.yml", "config.yaml"] {
        let path = dir.join(name);
        if path.exists() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// config.yml / config.yaml が無ければテンプレートを作成する。
/// 作成した場合は Some(作成パス) を返す。既に存在する場合と、
/// QUERYFOLIO_CONFIG_YAML 環境変数で上書き中の場合は None。
pub fn ensure_config_file() -> Result<Option<String>, AppError> {
    let env_override = std::env::var("QUERYFOLIO_CONFIG_YAML")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if env_override {
        return Ok(None);
    }
    ensure_config_file_in(&app_config_dir()?)
}

/// dir 内の設定ファイルのパス。config.yml を優先し、無ければ config.yaml、
/// どちらも無ければ config.yml のパスを返す。
fn config_path_in(dir: &std::path::Path) -> PathBuf {
    let yml = dir.join("config.yml");
    if yml.exists() {
        return yml;
    }
    let yaml = dir.join("config.yaml");
    if yaml.exists() {
        return yaml;
    }
    yml
}

fn ensure_config_file_in(dir: &std::path::Path) -> Result<Option<String>, AppError> {
    let yml = dir.join("config.yml");
    let yaml = dir.join("config.yaml");
    if yml.exists() || yaml.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(&yml, CONFIG_TEMPLATE)?;
    Ok(Some(yml.display().to_string()))
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
    /// queryfolio extension: the ssh-agent socket to use for agent
    /// authentication (equivalent to OpenSSH's IdentityAgent). Use "none" to
    /// disable the agent. When omitted, the agent socket is resolved from
    /// ~/.ssh/config (IdentityAgent) and then SSH_AUTH_SOCK. This lets a GUI
    /// launch reach an agent it did not inherit in its environment (e.g. the
    /// 1Password SSH agent).
    #[serde(default)]
    pub identity_agent: Option<String>,
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
    /// queryfolio 独自拡張: クエリファイルの保存フォルダ名を明示する。
    /// 省略時は <host>_<engine>_<schema>_<user> から組み立てる
    /// (name はフォルダ名には使わない)。sqlfiles_folder_name を参照。
    #[serde(default)]
    pub folder_name: Option<String>,
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
    /// queryfolio 独自拡張: true の場合、行を返さない文 (INSERT / UPDATE /
    /// DELETE / DDL 等) の実行を拒否する。省略時 false。
    /// SELECT に副作用のある関数 (nextval 等) までは防げない事故防止ガード。
    #[serde(default)]
    pub readonly: bool,
    /// queryfolio 独自拡張: true の場合、危険な文 (WHERE 無しの UPDATE /
    /// DELETE、DROP / TRUNCATE 等) の実行を許可する。省略時 false で、
    /// これらの文は誤操作による全行破壊・テーブル消失を防ぐため拒否される。
    /// true にしても、フロントエンドは実行前に確認を求める。
    #[serde(default)]
    pub allow_dangerous_statements: bool,
    /// queryfolio 独自拡張: 接続一覧での表示グループ名。
    /// sql_servers のグループエントリ (group_name + sql_servers) に
    /// 属するサーバーへ parse_server_entries が設定する。
    /// サーバーエントリ直下の group_name: はグループエントリの検証
    /// (空チェック・未知キー拒否) を迂回するため受け付けない (無視される)。
    #[serde(default, skip_deserializing)]
    pub group_name: Option<String>,
}

/// フォルダ名としてファイルシステム上安全になるようサニタイズする。
/// パス区切り (/ \) や NUL を _ に置換し、先頭ドット (不可視/相対) を避ける。
/// query_files::validate_component が拒否する文字を事前に潰しておく。
fn sanitize_folder_component(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            _ => c,
        })
        .collect();
    s = s.trim().to_string();
    if s.is_empty() {
        return "_".to_string();
    }
    if s.starts_with('.') {
        s.insert(0, '_');
    }
    s
}

impl ServerConfig {
    /// クエリファイルの保存フォルダ名を返す。
    /// folder_name が設定されていればそれを使い、無ければ
    /// <host>_<engine>_<schema>_<user> を組み立てる (name は使わない)。
    /// パス要素として安全になるよう区切り文字等はサニタイズする。
    pub fn sqlfiles_folder_name(&self) -> String {
        if let Some(folder) = self.folder_name.as_deref() {
            let folder = folder.trim();
            if !folder.is_empty() {
                return sanitize_folder_component(folder);
            }
        }
        let joined = [
            self.host.as_deref().unwrap_or(""),
            self.engine.as_str(),
            self.schema.as_deref().unwrap_or(""),
            self.user.as_deref().unwrap_or(""),
        ]
        .join("_");
        sanitize_folder_component(&joined)
    }
}

/// フロントエンドに渡す SSH トンネル情報。パスワードや鍵等の機密は含めない。
#[derive(Debug, Clone, Serialize)]
pub struct SshTunnelInfo {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl From<&SshTunnelConfig> for SshTunnelInfo {
    fn from(tunnel: &SshTunnelConfig) -> Self {
        Self {
            host: tunnel.host.clone(),
            port: tunnel.port,
            user: tunnel.user.clone(),
        }
    }
}

/// フロントエンドに渡す接続先情報。パスワード等の機密は含めない。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInfo {
    pub name: String,
    pub description: Option<String>,
    pub engine: String,
    pub has_ssh_tunnel: bool,
    /// 接続先ホスト (未設定なら null)
    pub host: Option<String>,
    /// 接続先ポート (未設定なら null)
    pub port: Option<u16>,
    /// 接続ユーザー (未設定なら null)
    pub user: Option<String>,
    /// 設定上のデフォルト database (スキーマ)
    pub schema: Option<String>,
    /// SSH トンネル情報 (機密を除く)。トンネル未使用なら null
    pub ssh_tunnel: Option<SshTunnelInfo>,
    /// 読み取り専用接続 (書き込み系の文の実行を拒否する)
    pub readonly: bool,
    /// 危険な文 (WHERE 無し UPDATE/DELETE、DROP/TRUNCATE 等) の実行を許可する。
    /// フロントエンドは true の接続でも実行前に確認を求める
    pub allow_dangerous_statements: bool,
    /// 接続一覧での表示グループ名 (グループ未所属なら null)
    pub group_name: Option<String>,
}

impl From<&ServerConfig> for ConnectionInfo {
    fn from(server: &ServerConfig) -> Self {
        Self {
            name: server.name.clone(),
            description: server.description.clone(),
            engine: server.engine.clone(),
            has_ssh_tunnel: server.ssh_tunnel.is_some(),
            host: server.host.clone(),
            port: server.port,
            user: server.user.clone(),
            schema: server.schema.clone(),
            ssh_tunnel: server.ssh_tunnel.as_ref().map(SshTunnelInfo::from),
            readonly: server.readonly,
            allow_dangerous_statements: server.allow_dangerous_statements,
            group_name: server.group_name.clone(),
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

/// resolve_servers の結果。サーバー一覧に加えて、ソース宣言で取得した
/// YAML のトップレベル `ai:` セクション (未検証の生値) も返す。
/// AI 設定の検証・解決は ai::resolve_ai_config が行う
/// (ローカル config.yml 側の ai は AppConfig::local_ai で取る)。
#[derive(Debug)]
pub struct ResolvedServers {
    pub servers: Vec<ServerConfig>,
    /// 取得 YAML のトップレベル ai セクション。インライン定義の場合は None
    pub fetched_ai: Option<serde_yaml::Value>,
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
                "Config file not found. Create {} (see config.example.yaml)",
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
        Ok(config_path_in(&app_config_dir()?))
    }

    /// LIMIT 未指定の SELECT に自動付与する行数上限。
    /// 省略時は 500。0 を指定すると無効。
    pub fn default_limit(&self) -> u64 {
        self.doc
            .get("default_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(500)
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
            AppError::Config("The config has no sql_servers key".into())
        })?;

        if value.is_sequence() {
            return Ok(ServersSource::Inline);
        }

        let mapping = value.as_mapping().ok_or_else(|| {
            AppError::Config(
                "sql_servers must be a list of server definitions, or a source declaration \
                 mapping (command / env / file)"
                    .into(),
            )
        })?;

        let mut sources = vec![];
        for (key, val) in mapping {
            let key = key.as_str().unwrap_or_default();
            let text = val.as_str().map(|s| s.to_string()).ok_or_else(|| {
                AppError::Config(format!("sql_servers.{key} must be a string"))
            })?;
            match key {
                "command" => sources.push(ServersSource::Command(text)),
                "env" => sources.push(ServersSource::Env(text)),
                "file" => sources.push(ServersSource::File(text)),
                other => {
                    return Err(AppError::Config(format!(
                        "Unknown key '{other}' in sql_servers (only command / env / file are allowed)"
                    )));
                }
            }
        }

        match sources.len() {
            1 => Ok(sources.into_iter().next().unwrap()),
            0 => Err(AppError::Config(
                "A sql_servers source declaration requires exactly one of command / env / file"
                    .into(),
            )),
            _ => Err(AppError::Config(
                "A sql_servers source declaration cannot have more than one of command / env / file"
                    .into(),
            )),
        }
    }

    /// ローカル config.yml トップレベルの `ai:` セクション (未検証の生値)。
    pub fn local_ai(&self) -> Option<serde_yaml::Value> {
        self.doc.get("ai").cloned()
    }

    /// 接続サーバー一覧を解決する。ソース宣言の場合は取得を伴う。
    /// 取得した YAML のトップレベル ai セクションもあわせて返す。
    pub async fn resolve_servers(&self) -> Result<ResolvedServers, AppError> {
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
                Ok(ResolvedServers {
                    servers: parse_server_entries(&servers, &templates, "config (inline)")?,
                    fetched_ai: None,
                })
            }
            ServersSource::Command(command) => {
                let yaml = run_source_command(&command).await?;
                parse_fetched_servers(&yaml, &format!("command: {command}"))
            }
            ServersSource::Env(env_name) => {
                let yaml = std::env::var(&env_name).map_err(|_| {
                    AppError::Config(format!(
                        "Environment variable {env_name} is not set \
                         (GUI apps launched from Finder / Dock do not inherit shell env vars)"
                    ))
                })?;
                parse_fetched_servers(&yaml, &format!("env: {env_name}"))
            }
            ServersSource::File(path) => {
                let file_path = expand_tilde(&path);
                if !file_path.exists() {
                    return Err(AppError::Config(format!(
                        "sql_servers file not found: {}",
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
            Ok(ServersSource::Inline) => "inline".to_string(),
            Ok(ServersSource::Command(command)) => format!("command: {command}"),
            Ok(ServersSource::Env(env_name)) => format!("env: {env_name}"),
            Ok(ServersSource::File(path)) => format!("file: {path}"),
            Err(e) => format!("(error: {e})"),
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
            source: format!("(error: {e})"),
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
                source: format!("(error: {e})"),
                sqlfiles_dir: String::new(),
            }
        }
    }
}

/// QUERYFOLIO_CONFIG_YAML 環境変数で設定が上書きされているか。
/// 上書き中は編集対象のファイルが存在しないため、エディタから編集できない。
fn config_env_override() -> bool {
    std::env::var("QUERYFOLIO_CONFIG_YAML")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// 設定エディタ用に config.yml の中身を読む。
/// ファイルがまだ無い場合はテンプレートを作成してから読む。
pub fn read_config_file() -> Result<String, AppError> {
    if config_env_override() {
        return Err(AppError::Config(
            "The config is overridden by QUERYFOLIO_CONFIG_YAML, so there is no file to edit"
                .into(),
        ));
    }
    read_config_file_in(&app_config_dir()?)
}

fn read_config_file_in(dir: &std::path::Path) -> Result<String, AppError> {
    ensure_config_file_in(dir)?;
    Ok(std::fs::read_to_string(config_path_in(dir))?)
}

/// 設定エディタからの保存。YAML として妥当なことを確認してから書き込む。
///
/// 書き込みは一時ファイル + rename で行い、途中で失敗しても既存の設定を
/// 半端な内容で壊さないようにする。
pub fn write_config_file(content: &str) -> Result<String, AppError> {
    if config_env_override() {
        return Err(AppError::Config(
            "The config is overridden by QUERYFOLIO_CONFIG_YAML, so it cannot be saved".into(),
        ));
    }
    write_config_file_in(&app_config_dir()?, content)
}

fn write_config_file_in(dir: &std::path::Path, content: &str) -> Result<String, AppError> {
    // 壊れた YAML をそのまま保存すると次回起動で接続一覧を失うため、
    // 保存前にマッピングとしてパースできることを確認する
    parse_mapping(content, "the edited config")?;

    std::fs::create_dir_all(dir)?;
    let path = config_path_in(dir);

    // 既存ファイルのパーミッションを引き継ぐ。新規なら 600
    // (接続パスワードを含み得るため他ユーザーに読ませない)
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600)
    };

    let temp = path.with_extension("yml.tmp");
    // 作成時からパーミッションを指定する。書いてから set_permissions すると、
    // その間だけ umask 依存 (通常 644) の権限で中身が置かれ、パスワードを
    // 含む設定を同一マシンの他ユーザーに読まれる隙ができる
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&temp)?;
        // mode は新規作成時にしか効かないため、前回の中断等で temp が
        // 残っていた場合に備えて明示的にも設定する
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    std::fs::write(&temp, content)?;
    std::fs::rename(&temp, &path)?;
    Ok(path.display().to_string())
}

/// sql_servers がソース宣言の `command:` かどうか。
/// メニュー項目の出し分けに使う。設定が読めない場合は false。
pub fn sql_servers_source_is_command() -> bool {
    matches!(
        AppConfig::load().and_then(|c| c.servers_source()),
        Ok(ServersSource::Command(_))
    )
}

/// sql_servers のソース宣言 `command:` を実行して、取得した生の YAML を返す。
/// 読み取り専用ビュー用。command 以外のソースではエラーにする。
pub async fn fetch_sql_servers_source_yaml() -> Result<String, AppError> {
    let config = AppConfig::load()?;
    match config.servers_source()? {
        ServersSource::Command(command) => run_source_command(&command).await,
        _ => Err(AppError::Config(
            "sql_servers does not use a command source declaration".into(),
        )),
    }
}

fn parse_mapping(yaml_text: &str, source: &str) -> Result<serde_yaml::Mapping, AppError> {
    let doc: serde_yaml::Value = serde_yaml::from_str(yaml_text)
        .map_err(|e| AppError::Config(format!("Failed to parse YAML from {source}: {e}")))?;
    doc.as_mapping().cloned().ok_or_else(|| {
        AppError::Config(format!("{source} is not a YAML mapping"))
    })
}

/// ソース宣言で取得した YAML をパースする。
/// sql-agent-mcp-server 互換フォーマット (sql_servers リスト + sql_server_templates)。
/// トップレベルに `ai:` セクションがあればあわせて返す (queryfolio 独自拡張)。
/// 取得先でさらにソース宣言を使う再帰は禁止 (ループ防止のため深さ 1 まで)。
fn parse_fetched_servers(
    yaml_text: &str,
    source: &str,
) -> Result<ResolvedServers, AppError> {
    let mapping = parse_mapping(yaml_text, source)?;
    let servers_value = mapping.get("sql_servers").ok_or_else(|| {
        AppError::Config(format!("{source} has no sql_servers key"))
    })?;
    let servers = servers_value.as_sequence().ok_or_else(|| {
        AppError::Config(format!(
            "sql_servers in {source} is not a list \
             (recursive source declarations are not allowed)"
        ))
    })?;
    let templates = mapping
        .get("sql_server_templates")
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    Ok(ResolvedServers {
        servers: parse_server_entries(servers, &templates, source)?,
        fetched_ai: mapping.get("ai").cloned(),
    })
}

/// sql_servers のリスト項目をパースする。項目は次のどちらか:
/// - サーバー定義そのもの
/// - グループエントリ (group_name + ネストした sql_servers リスト)。
///   ネストしたサーバーへフラット化し、各サーバーの group_name に記録する。
///   グループの中にさらにグループを書く再帰は禁止 (深さ 1 まで)。
fn parse_server_entries(
    servers: &[serde_yaml::Value],
    templates: &[serde_yaml::Value],
    source: &str,
) -> Result<Vec<ServerConfig>, AppError> {
    let mut result = Vec::new();
    for entry_value in servers {
        let entry = entry_value.as_mapping().ok_or_else(|| {
            AppError::Config(format!("A sql_servers entry in {source} is not a mapping"))
        })?;
        if !entry.contains_key("sql_servers") {
            result.push(parse_server_entry(entry_value, templates, source)?);
            continue;
        }

        // グループエントリ。typo をサイレントに飲み込まないよう未知キーは拒否する
        for (key, _) in entry {
            let key = key.as_str().unwrap_or_default();
            if key != "group_name" && key != "sql_servers" {
                return Err(AppError::Config(format!(
                    "Unknown key '{key}' in a sql_servers group entry in {source} \
                     (only group_name / sql_servers are allowed)"
                )));
            }
        }
        let group_name = entry
            .get("group_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::Config(format!(
                    "A sql_servers group entry in {source} requires a non-empty group_name"
                ))
            })?;
        let grouped = entry
            .get("sql_servers")
            .and_then(|v| v.as_sequence())
            .ok_or_else(|| {
                AppError::Config(format!(
                    "sql_servers in group '{group_name}' in {source} must be a list"
                ))
            })?;
        for server_value in grouped {
            let is_nested_group = server_value
                .as_mapping()
                .is_some_and(|m| m.contains_key("sql_servers"));
            if is_nested_group {
                return Err(AppError::Config(format!(
                    "Nested groups are not allowed in group '{group_name}' in {source}"
                )));
            }
            let mut server = parse_server_entry(server_value, templates, source)?;
            server.group_name = Some(group_name.to_string());
            result.push(server);
        }
    }
    Ok(result)
}

fn parse_server_entry(
    server_value: &serde_yaml::Value,
    templates: &[serde_yaml::Value],
    source: &str,
) -> Result<ServerConfig, AppError> {
    let expanded = expand_template(server_value, templates)?;
    serde_yaml::from_value(expanded).map_err(|e| {
        AppError::Config(format!(
            "Failed to parse a sql_servers entry in {source}: {e}"
        ))
    })
}

/// ソース宣言の command を実行して stdout を返す。
///
/// shlex で argv に分解し、シェルを介さず実行する。シェルメタ文字が混入しても
/// 解釈されないためコマンドインジェクションの余地が無い。その代わり
/// パイプ・リダイレクト・変数展開は使えない (単一コマンド前提)。
async fn run_source_command(command: &str) -> Result<String, AppError> {
    let argv = shlex::split(command).ok_or_else(|| {
        AppError::Config(format!(
            "Failed to parse sql_servers command (unbalanced quotes?): {command}"
        ))
    })?;
    if argv.is_empty() {
        return Err(AppError::Config("sql_servers command is empty".into()));
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
            "sql_servers command timed out ({SOURCE_COMMAND_TIMEOUT_SECS}s): {command} \
             (it may be hanging on 1Password or another auth prompt)"
        ))
    })?
    .map_err(|e| {
        AppError::Config(format!("Failed to run sql_servers command: {command}: {e}"))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Config(format!(
            "sql_servers command exited with an error (code={:?}): {command}\nstderr: {}",
            output.status.code(),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err(AppError::Config(format!(
            "sql_servers command produced no output: {command}"
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
        .ok_or_else(|| AppError::Config("A sql_servers entry is not a mapping".into()))?;

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
                "Template '{template_name}' not found in sql_server_templates"
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
        let servers = config.resolve_servers().await.unwrap().servers;
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "dev-postgres");
        assert_eq!(servers[0].port, Some(5432));
        assert!(servers[0].ssh_tunnel.is_none());
        assert!(servers[0].group_name.is_none());
    }

    #[tokio::test]
    async fn test_grouped_servers() {
        // グループエントリはフラット化され、各サーバーに group_name が付く。
        // グループと直書きサーバーの混在も設定順のまま解決される
        let config = config_from_yaml(
            r#"
sql_servers:
  - group_name: production
    sql_servers:
      - name: prod-main
        engine: mysql
        host: prod.example.com
      - name: prod-replica
        engine: mysql
        host: replica.example.com
  - name: standalone
    engine: sqlite
    schema: /tmp/x.db
  - group_name: development
    sql_servers:
      - name: dev-db
        engine: postgres
        host: localhost
"#,
        );
        let servers = config.resolve_servers().await.unwrap().servers;
        let summary: Vec<(&str, Option<&str>)> = servers
            .iter()
            .map(|s| (s.name.as_str(), s.group_name.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("prod-main", Some("production")),
                ("prod-replica", Some("production")),
                ("standalone", None),
                ("dev-db", Some("development")),
            ]
        );
        // ConnectionInfo にも伝わる
        let info = ConnectionInfo::from(&servers[0]);
        assert_eq!(info.group_name.as_deref(), Some("production"));
    }

    #[tokio::test]
    async fn test_flat_entry_group_name_is_ignored() {
        // サーバーエントリ直下の group_name: はグループエントリの検証を
        // 迂回できてしまうため、デシリアライズしない (無視される)
        let config = config_from_yaml(
            r#"
sql_servers:
  - name: sneaky
    engine: sqlite
    schema: /tmp/x.db
    group_name: bypassed
"#,
        );
        let servers = config.resolve_servers().await.unwrap().servers;
        assert!(servers[0].group_name.is_none());
    }

    #[tokio::test]
    async fn test_group_requires_non_empty_group_name() {
        let config = config_from_yaml(
            r#"
sql_servers:
  - group_name: ""
    sql_servers:
      - name: a
        engine: sqlite
        schema: /tmp/a.db
"#,
        );
        let err = config.resolve_servers().await.unwrap_err().to_string();
        assert!(err.contains("group_name"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_group_rejects_nested_group() {
        let config = config_from_yaml(
            r#"
sql_servers:
  - group_name: outer
    sql_servers:
      - group_name: inner
        sql_servers:
          - name: a
            engine: sqlite
            schema: /tmp/a.db
"#,
        );
        let err = config.resolve_servers().await.unwrap_err().to_string();
        assert!(err.contains("Nested groups"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_group_rejects_unknown_key() {
        // グループエントリの typo (servers: 等) をサイレントに無視しない
        let config = config_from_yaml(
            r#"
sql_servers:
  - group_name: g
    sql_servers: []
    description: typo-extra-key
"#,
        );
        let err = config.resolve_servers().await.unwrap_err().to_string();
        assert!(err.contains("Unknown key 'description'"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_group_with_template() {
        // グループ内のサーバーでも sql_server_templates を継承できる
        let config = config_from_yaml(
            r#"
sql_servers:
  - group_name: shared
    sql_servers:
      - name: db-a
        template: base
        schema: a_db
sql_server_templates:
  - name: base
    engine: mysql
    host: db.example.com
    port: 3306
    user: shared_user
"#,
        );
        let servers = config.resolve_servers().await.unwrap().servers;
        assert_eq!(servers[0].name, "db-a");
        assert_eq!(servers[0].engine, "mysql");
        assert_eq!(servers[0].host.as_deref(), Some("db.example.com"));
        assert_eq!(servers[0].schema.as_deref(), Some("a_db"));
        assert_eq!(servers[0].group_name.as_deref(), Some("shared"));
    }

    #[tokio::test]
    async fn test_readonly_flag() {
        // readonly は省略可能 (デフォルト false)。true 指定は ConnectionInfo に伝わる
        let config = config_from_yaml(
            r#"
sql_servers:
  - name: writable-db
    engine: sqlite
    schema: /tmp/x.db
  - name: readonly-db
    engine: sqlite
    schema: /tmp/x.db
    readonly: true
"#,
        );
        let servers = config.resolve_servers().await.unwrap().servers;
        assert!(!servers[0].readonly);
        assert!(servers[1].readonly);
        assert!(!ConnectionInfo::from(&servers[0]).readonly);
        assert!(ConnectionInfo::from(&servers[1]).readonly);
    }

    #[tokio::test]
    async fn test_connection_info_exposes_host_port_user_and_ssh() {
        // ConnectionInfo は host/port/user と SSH トンネル情報 (機密を除く) を
        // フロントへ渡す。パスワードや鍵は含めない。
        let config = config_from_yaml(
            r#"
sql_servers:
  - name: tunneled-db
    engine: postgres
    host: 10.0.0.5
    port: 5432
    user: app_user
    password: db-secret
    schema: app_db
    ssh_tunnel:
      host: bastion.example.com
      port: 2222
      user: jump
      password: ssh-secret
      private_key_path: /home/me/.ssh/id_ed25519
"#,
        );
        let servers = config.resolve_servers().await.unwrap().servers;
        let info = ConnectionInfo::from(&servers[0]);
        assert_eq!(info.host.as_deref(), Some("10.0.0.5"));
        assert_eq!(info.port, Some(5432));
        assert_eq!(info.user.as_deref(), Some("app_user"));
        assert!(info.has_ssh_tunnel);
        // 機密がシリアライズに漏れないことを確認する
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("db-secret"));
        assert!(!json.contains("ssh-secret"));
        assert!(!json.contains("id_ed25519"));
        let ssh = info.ssh_tunnel.expect("ssh tunnel info");
        assert_eq!(ssh.host, "bastion.example.com");
        assert_eq!(ssh.port, 2222);
        assert_eq!(ssh.user, "jump");
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
        let servers = config.resolve_servers().await.unwrap().servers;
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
        let servers = config.resolve_servers().await.unwrap().servers;
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
        let servers = config.resolve_servers().await.unwrap().servers;
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
        let servers = config.resolve_servers().await.unwrap().servers;
        assert_eq!(servers[0].name, "from-file");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_inline_has_no_fetched_ai_and_local_ai_is_returned() {
        // インライン定義では fetched_ai は無く、ローカルの ai は local_ai で取れる
        let config = config_from_yaml(
            r#"
sql_servers:
  - name: x
    engine: sqlite
    schema: /tmp/x.db
ai:
  provider: openai
  api_key: sk-local
"#,
        );
        let resolved = config.resolve_servers().await.unwrap();
        assert!(resolved.fetched_ai.is_none());
        let local = config.local_ai().unwrap();
        assert_eq!(
            local.get("api_key").and_then(|v| v.as_str()),
            Some("sk-local")
        );
    }

    #[tokio::test]
    async fn test_fetched_yaml_ai_is_extracted() {
        // ソース宣言で取得した YAML のトップレベル ai が fetched_ai として返る
        let config = config_from_yaml(
            r#"
sql_servers:
  command: '/bin/echo "{sql_servers: [{name: x, engine: sqlite, schema: /tmp/x.db}], ai: {provider: openai, api_key: sk-fetched}}"'
"#,
        );
        let resolved = config.resolve_servers().await.unwrap();
        assert_eq!(resolved.servers.len(), 1);
        let fetched = resolved.fetched_ai.unwrap();
        assert_eq!(
            fetched.get("api_key").and_then(|v| v.as_str()),
            Some("sk-fetched")
        );
        // ローカル側には ai が無い
        assert!(config.local_ai().is_none());
    }

    #[tokio::test]
    async fn test_fetched_yaml_without_ai() {
        // 取得 YAML に ai が無ければ fetched_ai は None
        let config = config_from_yaml(
            r#"
sql_servers:
  command: '/bin/echo "sql_servers: [{name: x, engine: sqlite, schema: /tmp/x.db}]"'
"#,
        );
        let resolved = config.resolve_servers().await.unwrap();
        assert!(resolved.fetched_ai.is_none());
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
        assert!(result.unwrap_err().to_string().contains("more than one"));
    }

    #[tokio::test]
    async fn test_source_unknown_key_is_error() {
        let config = config_from_yaml("sql_servers:\n  url: op://x/y/z\n");
        let result = config.resolve_servers().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown key"));
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
        assert!(result.unwrap_err().to_string().contains("recursive"));
    }

    #[test]
    fn test_default_limit() {
        let config = config_from_yaml("sql_servers: []\n");
        assert_eq!(config.default_limit(), 500);
        let config = config_from_yaml("sql_servers: []\ndefault_limit: 100\n");
        assert_eq!(config.default_limit(), 100);
        let config = config_from_yaml("sql_servers: []\ndefault_limit: 0\n");
        assert_eq!(config.default_limit(), 0);
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
    fn test_ensure_config_file_in() {
        let dir = std::env::temp_dir().join(format!(
            "queryfolio-ensure-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        // 無ければ作成して Some(パス) を返す
        let created = ensure_config_file_in(&dir).unwrap();
        assert!(created.is_some());
        assert!(dir.join("config.yml").exists());

        // 既に存在すれば None (上書きしない)
        assert!(ensure_config_file_in(&dir).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 設定エディタの読み書き。無ければテンプレートを作ってから読み、
    /// 保存した内容がそのまま読み戻せる。
    #[test]
    fn test_read_write_config_file_in() {
        let dir = std::env::temp_dir().join(format!(
            "queryfolio-editor-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        // ファイルが無い状態でもテンプレートが作られて読める
        let initial = read_config_file_in(&dir).unwrap();
        assert!(initial.contains("sql_servers"));

        let edited = "sql_servers:\n  - name: edited\n    engine: sqlite\n    schema: /tmp/a.db\n";
        let saved_path = write_config_file_in(&dir, edited).unwrap();
        assert_eq!(saved_path, dir.join("config.yml").display().to_string());
        assert_eq!(read_config_file_in(&dir).unwrap(), edited);
        // 一時ファイルを残さない
        assert!(!dir.join("config.yml.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 壊れた YAML は保存を拒否し、既存の設定を残す。
    #[test]
    fn test_write_config_file_in_rejects_invalid_yaml() {
        let dir = std::env::temp_dir().join(format!(
            "queryfolio-editor-invalid-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let valid = "sql_servers: []\n";
        write_config_file_in(&dir, valid).unwrap();

        // マッピングとしてパースできない内容
        assert!(write_config_file_in(&dir, "sql_servers: [\n").is_err());
        // YAML ではあるがマッピングではない
        assert!(write_config_file_in(&dir, "- just\n- a list\n").is_err());
        // 既存の内容は壊れていない
        assert_eq!(read_config_file_in(&dir).unwrap(), valid);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 保存時に既存ファイルのパーミッションを引き継ぐ (新規は 600)。
    #[cfg(unix)]
    #[test]
    fn test_write_config_file_in_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "queryfolio-editor-perm-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        // 新規作成は 600
        write_config_file_in(&dir, "sql_servers: []\n").unwrap();
        let path = dir.join("config.yml");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        // 既存のパーミッションは維持する
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_config_file_in(&dir, "sql_servers: []\n# edited\n").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// config.yaml (拡張子 yaml) を使っている場合も、そのファイルへ保存する。
    #[test]
    fn test_write_config_file_in_keeps_yaml_extension() {
        let dir = std::env::temp_dir().join(format!(
            "queryfolio-editor-yaml-ext-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.yaml"), "sql_servers: []\n").unwrap();

        let edited = "sql_servers: []\n# edited\n";
        let saved_path = write_config_file_in(&dir, edited).unwrap();
        assert_eq!(saved_path, dir.join("config.yaml").display().to_string());
        assert!(!dir.join("config.yml").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("config.yaml")).unwrap(),
            edited
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_config_template_is_valid() {
        // テンプレートはそのままで有効な設定 (接続 0 件) としてパースできること
        let config = config_from_yaml(CONFIG_TEMPLATE);
        let servers = config.resolve_servers().await.unwrap().servers;
        assert!(servers.is_empty());
        config.resolve_sqlfiles_dir().unwrap();
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
            folder_name: None,
            engine: "mysql".into(),
            host: Some("h".into()),
            port: Some(3306),
            schema: Some("db".into()),
            user: Some("u".into()),
            password: Some("secret".into()),
            ssh_tunnel: None,
            readonly: false,
            allow_dangerous_statements: false,
            group_name: None,
        };
        let info = ConnectionInfo::from(&server);
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("secret"));
    }

    fn server_with(
        folder_name: Option<&str>,
        host: Option<&str>,
        engine: &str,
        schema: Option<&str>,
        user: Option<&str>,
    ) -> ServerConfig {
        ServerConfig {
            name: "conn-name".into(),
            description: None,
            folder_name: folder_name.map(|s| s.to_string()),
            engine: engine.into(),
            host: host.map(|s| s.to_string()),
            port: None,
            schema: schema.map(|s| s.to_string()),
            user: user.map(|s| s.to_string()),
            password: None,
            ssh_tunnel: None,
            readonly: false,
            allow_dangerous_statements: false,
            group_name: None,
        }
    }

    #[test]
    fn test_sqlfiles_folder_name() {
        // folder_name があればそれを使う (name は使わない)
        let s = server_with(Some("my-folder"), Some("h"), "mysql", Some("db"), Some("u"));
        assert_eq!(s.sqlfiles_folder_name(), "my-folder");

        // folder_name が空文字列ならフォールバック
        let s = server_with(Some("   "), Some("h"), "mysql", Some("db"), Some("u"));
        assert_eq!(s.sqlfiles_folder_name(), "h_mysql_db_u");

        // folder_name 無し → <host>_<engine>_<schema>_<user>
        let s = server_with(
            None,
            Some("db.example.com"),
            "postgres",
            Some("prod"),
            Some("app"),
        );
        assert_eq!(s.sqlfiles_folder_name(), "db.example.com_postgres_prod_app");

        // sqlite: host/user 無し、schema はファイルパス → 区切りをサニタイズ
        let s = server_with(None, None, "sqlite", Some("/Users/me/data.db"), None);
        assert_eq!(s.sqlfiles_folder_name(), "_sqlite__Users_me_data.db_");

        // 先頭ドットは避ける (不可視/相対パス化を防ぐ)
        let s = server_with(Some(".hidden"), None, "sqlite", None, None);
        assert_eq!(s.sqlfiles_folder_name(), "_.hidden");
    }
}
