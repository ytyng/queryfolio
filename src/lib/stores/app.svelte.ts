import { toast } from "svelte-sonner";
import * as api from "$lib/api";
import type { ConnectionInfo, QueryResult } from "$lib/api";

const AUTO_SAVE_DELAY_MS = 1000;

/// 結果タブの上限。超過時は最も古い非ピン留めタブを破棄する
const MAX_RESULT_TABS = 10;

/// 結果ペインの 1 タブ分の状態。結果セットに加えて
/// 「何を・どこで・いつ実行したか」を保持し、タブから再実行できるようにする
export interface ResultTab {
  id: number;
  pinned: boolean;
  sql: string;
  connection: string;
  schema: string | null;
  /// 実行開始時刻 (epoch ms)
  executedAt: number;
  result: QueryResult | null;
  error: string | null;
  running: boolean;
}

let connections = $state<ConnectionInfo[]>([]);
let selectedConnection = $state<string | null>(null);
let files = $state<string[]>([]);
let selectedFile = $state<string | null>(null);
let editorContent = $state("");
let resultTabs = $state<ResultTab[]>([]);
let activeTabId = $state<number | null>(null);
let errorMessage = $state<string | null>(null);
let loadingConnections = $state(false);
let dirty = $state(false);
let schemas = $state<string[]>([]);
let activeSchema = $state<string | null>(null);

// タブ ID の連番 (セッション内で一意なら十分なので永続化しない)
let nextTabId = 1;

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

/// 接続設定を再読込する (プール・SSH トンネルも破棄される)。
/// 旧設定の選択状態を残さないよう一旦クリアし、同名の接続が
/// まだ存在する場合のみ再選択する (ファイル一覧も新設定で再取得される)。
/// 失敗した場合は false を返す (errorMessage 設定済み)。
const reloadConnections = async (): Promise<boolean> => {
  if (!(await flushPendingSave())) {
    return false;
  }
  try {
    await api.resetConnections();
  } catch (e) {
    errorMessage = toErrorMessage(e);
    return false;
  }
  const previousConnection = selectedConnection;
  selectedConnection = null;
  files = [];
  selectedFile = null;
  editorContent = "";
  // 設定が丸ごと入れ替わるため、ピン留め含め全タブを破棄する
  resultTabs = [];
  activeTabId = null;
  dirty = false;
  schemas = [];
  activeSchema = null;
  await loadConnections();
  if (errorMessage) {
    return false;
  }
  if (
    previousConnection &&
    connections.some((c) => c.name === previousConnection)
  ) {
    await selectConnection(previousConnection);
  }
  return true;
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
  // 結果タブは接続をまたいで比較できるよう、接続切替では破棄しない
  selectedConnection = name;
  selectedFile = null;
  editorContent = "";
  errorMessage = null;
  dirty = false;
  activeSchema =
    connections.find((c) => c.name === name)?.schema ?? null;
  schemas = [];
  try {
    files = await api.listQueryFiles(name);
  } catch (e) {
    errorMessage = toErrorMessage(e);
    files = [];
  }
  // スキーマ一覧の取得は接続確立を伴うため、失敗しても選択自体は成立させる
  // (エラーは結果ペインに出さず、プルダウンを現在値のみにする)
  try {
    activeSchema = (await api.getActiveSchema(name)) ?? activeSchema;
    schemas = await api.listSchemas(name);
  } catch {
    schemas = activeSchema ? [activeSchema] : [];
  }
};

// アクティブスキーマ (database) を切り替える。成功したら true。
const changeActiveSchema = async (schema: string): Promise<boolean> => {
  if (!selectedConnection || schema === activeSchema) {
    return true;
  }
  if (!(await flushPendingSave())) {
    return false;
  }
  try {
    await api.setActiveSchema(selectedConnection, schema);
    activeSchema = schema;
    errorMessage = null;
    return true;
  } catch (e) {
    errorMessage = toErrorMessage(e);
    return false;
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
    errorMessage = `Failed to save the file: ${toErrorMessage(e)}`;
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

/// 実行結果の書き込み先タブを決める。
/// アクティブな非ピン留めタブがあれば使い回し、無ければ新規タブを作る。
/// 上限到達時は最も古い非ピン留めタブを破棄する。
/// 全タブがピン留めで空きを作れない場合は null を返す (toast で通知済み)。
const prepareTargetTab = (): ResultTab | null => {
  const current = resultTabs.find((t) => t.id === activeTabId);
  if (current && !current.pinned) {
    // 同じタブへの二重書き込みを防ぐ (Cmd+Enter 連打対策)
    // タブ管理系の通知は結果ペインを覆わないよう toast で出す
    if (current.running) {
      toast.warning("A query is already running in this tab.");
      return null;
    }
    return current;
  }
  if (resultTabs.length >= MAX_RESULT_TABS) {
    const oldest = resultTabs
      .filter((t) => !t.pinned)
      .reduce<ResultTab | null>(
        (acc, t) => (acc === null || t.executedAt < acc.executedAt ? t : acc),
        null,
      );
    if (!oldest) {
      toast.warning(
        "All result tabs are pinned. Unpin or close a tab to run a new query.",
      );
      return null;
    }
    resultTabs = resultTabs.filter((t) => t.id !== oldest.id);
  }
  const tab: ResultTab = {
    id: nextTabId++,
    pinned: false,
    sql: "",
    connection: "",
    schema: null,
    executedAt: Date.now(),
    result: null,
    error: null,
    running: false,
  };
  resultTabs = [...resultTabs, tab];
  // 生のオブジェクトではなく $state プロキシ経由の参照を返す
  // (生の参照を書き換えてもリアクティブに反映されないため)
  return resultTabs[resultTabs.length - 1];
};

/// タブに記録された接続・SQL でクエリを実行し、結果をタブへ書き込む
const executeTab = async (tab: ResultTab) => {
  tab.running = true;
  // 失敗時に前回の結果を誤認・誤エクスポートしないよう、実行前にクリアする
  tab.result = null;
  tab.error = null;
  tab.executedAt = Date.now();
  activeTabId = tab.id;
  let result: QueryResult | null = null;
  let error: string | null = null;
  try {
    result = await api.runQuery(tab.connection, tab.sql);
  } catch (e) {
    error = toErrorMessage(e);
  }
  tab.running = false;
  // 実行中に設定再読込などでタブが破棄されていた場合は、
  // 存在しないタブ (detached なオブジェクト) へ書き込まず結果を捨てる
  if (!resultTabs.some((t) => t.id === tab.id)) {
    return;
  }
  tab.result = result;
  tab.error = error;
};

const runQuery = async (sql: string) => {
  // 実行前ガードの通知は、既存の結果タブを覆わないよう toast で出す
  if (!selectedConnection) {
    toast.warning("Select a connection first");
    return;
  }
  if (!sql.trim()) {
    toast.warning("There is no SQL statement to run");
    return;
  }
  if (!(await flushPendingSave())) {
    return;
  }
  errorMessage = null;
  const tab = prepareTargetTab();
  if (!tab) {
    return;
  }
  tab.sql = sql;
  tab.connection = selectedConnection;
  tab.schema = activeSchema;
  await executeTab(tab);
};

/// タブに記録された SQL を同じ接続で再実行する
const rerunTab = async (id: number) => {
  const tab = resultTabs.find((t) => t.id === id);
  if (!tab || tab.running || !tab.sql.trim()) {
    return;
  }
  errorMessage = null;
  // 接続のアクティブスキーマは実行時点から変わっている可能性があるため、
  // 表示が実際の実行先とずれないよう再取得する (失敗しても実行は続ける)
  try {
    tab.schema = (await api.getActiveSchema(tab.connection)) ?? tab.schema;
  } catch {
    // 取得失敗時は記録済みのスキーマ表示を維持する
  }
  await executeTab(tab);
};

const selectResultTab = (id: number) => {
  if (resultTabs.some((t) => t.id === id)) {
    activeTabId = id;
  }
};

const closeResultTab = (id: number) => {
  const index = resultTabs.findIndex((t) => t.id === id);
  if (index < 0) {
    return;
  }
  // 実行中のタブを閉じると in-flight のクエリ状態が UI から消えてしまうため拒否する
  // (閉じるボタンも disabled にしているが、防御的にここでもガードする)
  if (resultTabs[index].running) {
    toast.warning("Cannot close a tab while its query is running.");
    return;
  }
  resultTabs = resultTabs.filter((t) => t.id !== id);
  if (activeTabId === id) {
    // 閉じたタブの右隣 (無ければ左隣) をアクティブにする
    const neighbor = resultTabs[index] ?? resultTabs[index - 1] ?? null;
    activeTabId = neighbor?.id ?? null;
  }
};

const toggleResultTabPin = (id: number) => {
  const tab = resultTabs.find((t) => t.id === id);
  if (tab) {
    tab.pinned = !tab.pinned;
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
  get resultTabs() {
    return resultTabs;
  },
  get activeTabId() {
    return activeTabId;
  },
  /// アクティブな結果タブ (無ければ null)
  get activeResultTab() {
    return resultTabs.find((t) => t.id === activeTabId) ?? null;
  },
  get errorMessage() {
    return errorMessage;
  },
  /// いずれかのタブでクエリ実行中なら true (Run ボタンの無効化などに使う)
  get running() {
    return resultTabs.some((t) => t.running);
  },
  get loadingConnections() {
    return loadingConnections;
  },
  get dirty() {
    return dirty;
  },
  get schemas() {
    return schemas;
  },
  get activeSchema() {
    return activeSchema;
  },
  loadConnections,
  reloadConnections,
  selectConnection,
  changeActiveSchema,
  selectFile,
  createFile,
  deleteFile,
  saveCurrentFile,
  updateEditorContent,
  runQuery,
  rerunTab,
  selectResultTab,
  closeResultTab,
  toggleResultTabPin,
};
