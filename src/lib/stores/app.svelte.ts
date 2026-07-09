import * as api from "$lib/api";
import type { ConnectionInfo, QueryResult } from "$lib/api";

const AUTO_SAVE_DELAY_MS = 1000;

let connections = $state<ConnectionInfo[]>([]);
let selectedConnection = $state<string | null>(null);
let files = $state<string[]>([]);
let selectedFile = $state<string | null>(null);
let editorContent = $state("");
let queryResult = $state<QueryResult | null>(null);
let errorMessage = $state<string | null>(null);
let running = $state(false);
let loadingConnections = $state(false);
let dirty = $state(false);

let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;

const toErrorMessage = (e: unknown): string =>
  typeof e === "string" ? e : e instanceof Error ? e.message : String(e);

const loadConnections = async () => {
  loadingConnections = true;
  errorMessage = null;
  try {
    connections = await api.getConnections();
  } catch (e) {
    errorMessage = toErrorMessage(e);
    connections = [];
  } finally {
    loadingConnections = false;
  }
};

/// 接続設定を再読込する (プール・SSH トンネルも破棄される)
const reloadConnections = async () => {
  try {
    await api.resetConnections();
  } catch (e) {
    errorMessage = toErrorMessage(e);
    return;
  }
  await loadConnections();
};

// 保留中の自動保存を確定させる。保存に失敗した場合は false を返す。
// 呼び出し元は false の時に画面遷移を中断し、未保存の編集を守ること。
const flushPendingSave = async (): Promise<boolean> => {
  if (autoSaveTimer) {
    clearTimeout(autoSaveTimer);
    autoSaveTimer = null;
  }
  if (dirty && selectedConnection && selectedFile) {
    return saveCurrentFile();
  }
  return true;
};

const selectConnection = async (name: string) => {
  if (!(await flushPendingSave())) {
    return;
  }
  selectedConnection = name;
  selectedFile = null;
  editorContent = "";
  queryResult = null;
  errorMessage = null;
  dirty = false;
  try {
    files = await api.listQueryFiles(name);
  } catch (e) {
    errorMessage = toErrorMessage(e);
    files = [];
  }
};

const selectFile = async (fileName: string) => {
  if (!selectedConnection) {
    return;
  }
  if (!(await flushPendingSave())) {
    return;
  }
  try {
    editorContent = await api.readQueryFile(selectedConnection, fileName);
    selectedFile = fileName;
    dirty = false;
    errorMessage = null;
  } catch (e) {
    errorMessage = toErrorMessage(e);
  }
};

const createFile = async (fileName: string) => {
  if (!selectedConnection) {
    return;
  }
  try {
    const normalized = await api.createQueryFile(selectedConnection, fileName);
    files = await api.listQueryFiles(selectedConnection);
    await selectFile(normalized);
  } catch (e) {
    errorMessage = toErrorMessage(e);
  }
};

const deleteFile = async (fileName: string) => {
  if (!selectedConnection) {
    return;
  }
  try {
    await api.deleteQueryFile(selectedConnection, fileName);
    files = await api.listQueryFiles(selectedConnection);
    if (selectedFile === fileName) {
      selectedFile = null;
      editorContent = "";
      dirty = false;
    }
  } catch (e) {
    errorMessage = toErrorMessage(e);
  }
};

// 現在のファイルを保存する。成功したら true。
// 失敗時は dirty を保持したまま errorMessage を設定する。
const saveCurrentFile = async (): Promise<boolean> => {
  if (!selectedConnection || !selectedFile) {
    return true;
  }
  try {
    await api.writeQueryFile(selectedConnection, selectedFile, editorContent);
    dirty = false;
    return true;
  } catch (e) {
    errorMessage = `保存に失敗しました: ${toErrorMessage(e)}`;
    return false;
  }
};

/// エディタからの変更通知。自動保存をデバウンスして予約する。
const updateEditorContent = (content: string) => {
  if (content === editorContent) {
    return;
  }
  editorContent = content;
  dirty = true;
  if (autoSaveTimer) {
    clearTimeout(autoSaveTimer);
  }
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null;
    void saveCurrentFile();
  }, AUTO_SAVE_DELAY_MS);
};

const runQuery = async (sql: string) => {
  if (!selectedConnection) {
    errorMessage = "接続先を選択してください";
    return;
  }
  if (!sql.trim()) {
    errorMessage = "実行する SQL がありません";
    return;
  }
  if (!(await flushPendingSave())) {
    return;
  }
  running = true;
  errorMessage = null;
  // 失敗時に前回の結果を誤認・誤エクスポートしないよう、実行前にクリアする
  queryResult = null;
  try {
    queryResult = await api.runQuery(selectedConnection, sql);
  } catch (e) {
    errorMessage = toErrorMessage(e);
  } finally {
    running = false;
  }
};

export default {
  get connections() {
    return connections;
  },
  get selectedConnection() {
    return selectedConnection;
  },
  get files() {
    return files;
  },
  get selectedFile() {
    return selectedFile;
  },
  get editorContent() {
    return editorContent;
  },
  get queryResult() {
    return queryResult;
  },
  get errorMessage() {
    return errorMessage;
  },
  get running() {
    return running;
  },
  get loadingConnections() {
    return loadingConnections;
  },
  get dirty() {
    return dirty;
  },
  loadConnections,
  reloadConnections,
  selectConnection,
  selectFile,
  createFile,
  deleteFile,
  saveCurrentFile,
  updateEditorContent,
  runQuery,
};
