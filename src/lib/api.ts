import { invoke } from "@tauri-apps/api/core";

export interface ConnectionInfo {
  name: string;
  description: string | null;
  engine: string;
  has_ssh_tunnel: boolean;
  schema: string | null;
  readonly: boolean;
}

export interface QueryResult {
  columns: string[];
  rows: unknown[][];
  row_count: number;
  affected_rows: number | null;
  truncated: boolean;
  applied_limit: number | null;
  elapsed_ms: number;
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

export interface ConfigInfo {
  config_path: string;
  config_exists: boolean;
  source: string;
  sqlfiles_dir: string;
}

export const getConnections = () =>
  invoke<ConnectionInfo[]>("get_connections");

export const resetConnections = () => invoke<void>("reset_connections");

export const runQuery = (connection: string, sql: string, maxRows?: number) =>
  invoke<QueryResult>("run_query", { connection, sql, maxRows });

/// クエリ実行履歴を新しい順に返す。search は SQL の部分一致 (大小無視)。
export const listQueryHistory = (
  connection: string,
  search?: string,
  limit?: number,
) => invoke<QueryHistoryEntry[]>("list_query_history", { connection, search, limit });

export const listQueryFiles = (connection: string) =>
  invoke<string[]>("list_query_files", { connection });

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

export const listSchemas = (connection: string) =>
  invoke<string[]>("list_schemas", { connection });

export const setActiveSchema = (connection: string, schema: string) =>
  invoke<void>("set_active_schema", { connection, schema });

export const getActiveSchema = (connection: string) =>
  invoke<string | null>("get_active_schema", { connection });

export const getConfigInfo = () => invoke<ConfigInfo>("get_config_info");

/// config.yml が無ければテンプレートを作成する。作成した場合はそのパスを返す。
export const ensureConfigFile = () =>
  invoke<string | null>("ensure_config_file");
