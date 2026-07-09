import { invoke } from "@tauri-apps/api/core";

export interface ConnectionInfo {
  name: string;
  description: string | null;
  engine: string;
  has_ssh_tunnel: boolean;
}

export interface QueryResult {
  columns: string[];
  rows: unknown[][];
  row_count: number;
  affected_rows: number | null;
  truncated: boolean;
  elapsed_ms: number;
}

export interface AppSettings {
  config_yaml_path: string | null;
  config_yaml_getter_command: string | null;
  sqlfiles_dir: string | null;
}

export const getConnections = () =>
  invoke<ConnectionInfo[]>("get_connections");

export const resetConnections = () => invoke<void>("reset_connections");

export const runQuery = (connection: string, sql: string, maxRows?: number) =>
  invoke<QueryResult>("run_query", { connection, sql, maxRows });

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

export const getSettings = () => invoke<AppSettings>("get_settings");

export const saveSettings = (settings: AppSettings) =>
  invoke<void>("save_settings", { settings });
