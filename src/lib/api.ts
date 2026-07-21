import { invoke } from "@tauri-apps/api/core";

/// SSH トンネル情報 (機密を除く。バックエンドの config::SshTunnelInfo に対応)
export interface SshTunnelInfo {
  host: string;
  port: number;
  user: string;
}

export interface ConnectionInfo {
  name: string;
  description: string | null;
  engine: string;
  has_ssh_tunnel: boolean;
  /// 接続先ホスト (未設定なら null)
  host: string | null;
  /// 接続先ポート (未設定なら null)
  port: number | null;
  /// 接続ユーザー (未設定なら null)
  user: string | null;
  schema: string | null;
  /// SSH トンネル情報 (機密を除く)。トンネル未使用なら null
  ssh_tunnel: SshTunnelInfo | null;
  readonly: boolean;
  /// 危険な文 (WHERE 無し UPDATE/DELETE、DROP/TRUNCATE) の実行を許可する接続。
  /// true でも実行前に確認を求める
  allow_dangerous_statements: boolean;
  /// 接続一覧での表示グループ名 (グループ未所属なら null)
  group_name: string | null;
}

export interface QueryResult {
  columns: string[];
  rows: unknown[][];
  row_count: number;
  affected_rows: number | null;
  truncated: boolean;
  applied_limit: number | null;
  elapsed_ms: number;
  /// `\c` でアクティブスキーマを切り替えた場合の切替先 (それ以外は null)
  switched_schema: string | null;
}

/// クエリ実行履歴の 1 件分 (バックエンドの history::HistoryEntry に対応)
export interface QueryHistoryEntry {
  /// 実行時刻 (ISO 8601)
  time: string;
  sql: string;
  /// 実行時のアクティブスキーマ (database)
  schema: string | null;
  /// 取得行数または影響行数 (失敗時は null)
  row_count: number | null;
  elapsed_ms: number;
  success: boolean;
}

/// テーブル / ビューの情報 (バックエンドの schema_info::TableInfo に対応)
export interface TableInfo {
  /// テーブル名 (スキーマ修飾なし)
  name: string;
  /// 所属スキーマ名 (PostgreSQL のみ。MySQL / SQLite は null)
  schema: string | null;
  /// "table" または "view"
  kind: string;
  /// SQL に埋め込める修飾名。listColumns の table 引数やエディタへの
  /// 挿入にはこの値を使う
  qualified_name: string;
}

/// カラムの情報 (バックエンドの schema_info::ColumnInfo に対応)
export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
}

/// AI 設定の情報 (バックエンドの ai::AiInfo に対応)。api_key は含まれない
export interface AiInfo {
  configured: boolean;
  /// 使用モデル名 (未設定時は空文字)
  model: string;
}

export interface ConfigInfo {
  config_path: string;
  config_exists: boolean;
  source: string;
  sqlfiles_dir: string;
}

export const getConnections = () =>
  invoke<ConnectionInfo[]>("get_connections");

export const resetConnections = () => invoke<void>("reset_connections");

/// writable は Writable スイッチの状態 (未指定・false は読み取り専用の安全側)。
/// config の readonly: true 接続では、writable に関わらず書き込みは拒否される。
export const runQuery = (
  connection: string,
  sql: string,
  maxRows?: number,
  writable?: boolean,
) => invoke<QueryResult>("run_query", { connection, sql, maxRows, writable });

/// 接続で実行中のクエリにキャンセルを要求する。実行中でなければ false。
/// キャンセルされた run_query は CANCELLED_ERROR_MESSAGE のエラーで返る。
export const cancelQuery = (connection: string) =>
  invoke<boolean>("cancel_query", { connection });

/// バックエンドの AppError::Cancelled が返す文字列 (キャンセル判定用)
export const CANCELLED_ERROR_MESSAGE = "Query cancelled";

/// 危険な文 (WHERE 無し UPDATE/DELETE、DROP/TRUNCATE) なら理由を、そうでなければ
/// null を返す。allow_dangerous_statements が有効な接続で、実行前の確認要否を
/// 判断するために使う (無効な接続では runQuery 側が拒否する)。
export const checkDangerousStatement = (connection: string, sql: string) =>
  invoke<string | null>("check_dangerous_statement", { connection, sql });

/// クエリ実行履歴を新しい順に返す。search は SQL の部分一致 (大小無視)。
export const listQueryHistory = (
  connection: string,
  search?: string,
  limit?: number,
) => invoke<QueryHistoryEntry[]>("list_query_history", { connection, search, limit });

export const listQueryFiles = (connection: string) =>
  invoke<string[]>("list_query_files", { connection });

/// クエリファイル検索の 1 ヒット (バックエンドの query_files::FileSearchHit に対応)
export interface FileSearchHit {
  /// ヒットしたファイル名 (.sql 付き)
  file_name: string;
  /// ファイル名が query に一致したか
  name_match: boolean;
  /// 中身が一致した最初の行 (プレビュー用。名前のみ一致なら null)
  content_preview: string | null;
}

/// 接続のクエリファイルをファイル名・中身で検索する (大文字小文字を区別しない部分一致)。
export const searchQueryFiles = (connection: string, query: string) =>
  invoke<FileSearchHit[]>("search_query_files", { connection, query });

export const readQueryFile = (connection: string, fileName: string) =>
  invoke<string>("read_query_file", { connection, fileName });

export const writeQueryFile = (
  connection: string,
  fileName: string,
  content: string,
) => invoke<void>("write_query_file", { connection, fileName, content });

export const createQueryFile = (connection: string, fileName: string) =>
  invoke<string>("create_query_file", { connection, fileName });

export const deleteQueryFile = (connection: string, fileName: string) =>
  invoke<void>("delete_query_file", { connection, fileName });

export const renameQueryFile = (
  connection: string,
  oldName: string,
  newName: string,
) => invoke<string>("rename_query_file", { connection, oldName, newName });

export const listSchemas = (connection: string) =>
  invoke<string[]>("list_schemas", { connection });

export const setActiveSchema = (connection: string, schema: string) =>
  invoke<void>("set_active_schema", { connection, schema });

export const getActiveSchema = (connection: string) =>
  invoke<string | null>("get_active_schema", { connection });

/// テーブル / ビューの一覧を返す。refresh = true でキャッシュを破棄して再取得。
export const listTables = (connection: string, refresh?: boolean) =>
  invoke<TableInfo[]>("list_tables", { connection, refresh });

/// テーブルのカラム一覧を返す。table には TableInfo.qualified_name を渡す。
export const listColumns = (connection: string, table: string) =>
  invoke<ColumnInfo[]>("list_columns", { connection, table });

/// テーブル名 → カラム名リストのマップを返す (SQL 補完用)。
export const getSchemaMap = (connection: string) =>
  invoke<Record<string, string[]>>("get_schema_map", { connection });

/// テーブルの主キーを構成するカラム名を返す (結果グリッドのセル編集用)。
/// 主キーが無いテーブルでは空配列。
export const getPrimaryKeys = (connection: string, table: string) =>
  invoke<string[]>("get_primary_keys", { connection, table });

/// 結果グリッドのセル編集を UPDATE 群として 1 トランザクションで適用する。
/// writable の意味は runQuery と同じ (未指定・false は読み取り専用)。
/// 合計の影響行数を返す。
export const runStatements = (
  connection: string,
  statements: string[],
  writable?: boolean,
) => invoke<number>("run_statements", { connection, statements, writable });

/// AI 設定の情報を返す。`ai:` セクションが無い場合は configured: false。
/// セクションはあるが不正 (不明 provider 等) な場合は reject される。
export const getAiInfo = () => invoke<AiInfo>("get_ai_info");

/// 自然言語の指示から SQL を生成して返す (実行はしない)。
export const aiGenerateSql = (connection: string, instruction: string) =>
  invoke<string>("ai_generate_sql", { connection, instruction });

/// 失敗した SQL とエラーメッセージから修正案の SQL を返す (実行はしない)。
export const aiFixSql = (
  connection: string,
  sql: string,
  errorMessage: string,
) => invoke<string>("ai_fix_sql", { connection, sql, errorMessage });

/// エンジン別の EXPLAIN プレフィックスを付けた SQL を組み立てて返す
/// (実行はしない)。SELECT / WITH 以外の文は reject される。
export const buildExplainSql = (connection: string, sql: string) =>
  invoke<string>("build_explain_sql", { connection, sql });

/// EXPLAIN の実行計画を AI に解説させ、Markdown テキストを返す。
export const aiExplainPlan = (
  connection: string,
  sql: string,
  planText: string,
) => invoke<string>("ai_explain_plan", { connection, sql, planText });

/// カーソル位置の SQL 文を AI に平易に解説させ、Markdown テキストを返す
/// (実行はしない)。
export const aiExplainSql = (connection: string, sql: string) =>
  invoke<string>("ai_explain_sql", { connection, sql });

export const getConfigInfo = () => invoke<ConfigInfo>("get_config_info");

/// config.yml が無ければテンプレートを作成する。作成した場合はそのパスを返す。
export const ensureConfigFile = () =>
  invoke<string | null>("ensure_config_file");

/// 設定エディタ用に config.yml の中身を読む (無ければテンプレートを作成してから読む)。
export const readConfigFile = () => invoke<string>("read_config_file");

/// 設定エディタからの保存。書き込んだファイルのパスを返す。
export const writeConfigFile = (content: string) =>
  invoke<string>("write_config_file", { content });

/// sql_servers のソース宣言 command を実行して取得した生の YAML を返す
/// (コピー用ビュー用。表示先では編集できるが保存はしない)。
export const readSqlServersSourceYaml = () =>
  invoke<string>("read_sql_servers_source_yaml");
