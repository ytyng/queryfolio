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
  /// キャンセル要求で実行が中断された (エラーとは別の見た目で表示する)
  cancelled: boolean;
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
/// SQL 補完用のテーブル名 → カラム名リストのマップ (未取得・取得失敗は null)
let schemaMap = $state<Record<string, string[]> | null>(null);

// タブ ID の連番 (セッション内で一意なら十分なので永続化しない)
let nextTabId = 1;

/// 実行中の loadSchemaMap の世代番号。接続・スキーマの連続切替で
/// 古い応答が後から解決しても、最新の要求の結果だけを反映するために使う
let schemaMapGeneration = 0;

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
  // 実行中の取得が後から古いマップを書き込まないよう世代を進めて破棄する
  schemaMapGeneration++;
  schemaMap = null;
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

/// SQL 補完用のスキーママップをバックグラウンドで再取得する。
/// 補完はあくまで補助機能のため、失敗しても通知せず補完なしで続行する。
const loadSchemaMap = async () => {
  const connection = selectedConnection;
  const generation = ++schemaMapGeneration;
  if (!connection) {
    schemaMap = null;
    return;
  }
  // 取得中に古いスキーマの候補を出さないよう先にクリアする
  schemaMap = null;
  try {
    const map = await api.getSchemaMap(connection);
    // より新しい要求が始まっていたら、古い応答は捨てる
    if (generation === schemaMapGeneration) {
      schemaMap = map;
    }
  } catch {
    // 補完なしで黙って続行 (toast も errorMessage も出さない)
  }
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
  // 接続確立を待つ間、前の接続の補完候補を出さないよう先にクリアする
  // (世代も進め、実行中の古い取得が後から書き込むのを防ぐ)
  schemaMapGeneration++;
  schemaMap = null;
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
  // SQL 補完用のスキーママップをバックグラウンドで取得する (待たない)
  void loadSchemaMap();
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
    // 切替先スキーマの補完候補をバックグラウンドで取得する (待たない)
    void loadSchemaMap();
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

/// 履歴パネル・スキーマブラウザからの SQL 断片の挿入。
/// 開いているファイルの末尾に追記する (既存の編集内容を上書きしないよう、
/// 置換ではなく追記にする)。実行はしない。
/// エディタへの反映は SqlEditor 側の $effect が行う。
const insertSqlSnippet = (sql: string) => {
  if (!selectedConnection) {
    toast.warning("Select a connection first");
    return;
  }
  if (!selectedFile) {
    toast.warning("Select or create a query file first");
    return;
  }
  const trimmed = editorContent.replace(/\s+$/, "");
  updateEditorContent(trimmed ? `${trimmed}\n\n${sql}\n` : `${sql}\n`);
};

/// 指定した接続でクエリ実行中のタブがあるかを返す。
/// バックエンドのキャンセルレジストリは接続単位で最後の実行しか
/// 管理しないため、同一接続の並列実行はフロント側で抑止する
/// (許すとキャンセル対象の取り違えや取りこぼしが起きる)。
const isConnectionRunning = (connection: string): boolean =>
  resultTabs.some((t) => t.running && t.connection === connection);

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
    cancelled: false,
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
  tab.cancelled = false;
  tab.executedAt = Date.now();
  activeTabId = tab.id;
  let result: QueryResult | null = null;
  let error: string | null = null;
  let cancelled = false;
  try {
    result = await api.runQuery(tab.connection, tab.sql);
  } catch (e) {
    const message = toErrorMessage(e);
    // キャンセルによる中断はエラーではなく「Query cancelled」として表示する
    if (message === api.CANCELLED_ERROR_MESSAGE) {
      cancelled = true;
    } else {
      error = message;
    }
  }
  tab.running = false;
  // 実行中に設定再読込などでタブが破棄されていた場合は、
  // 存在しないタブ (detached なオブジェクト) へ書き込まず結果を捨てる
  if (!resultTabs.some((t) => t.id === tab.id)) {
    return;
  }
  tab.result = result;
  tab.error = error;
  tab.cancelled = cancelled;
};

/// タブで実行中のクエリのキャンセルを要求する。
/// 実際の中断はバックエンドが行い、実行中の runQuery が
/// 「Query cancelled」で返ることで executeTab 側がタブに反映する。
const cancelQuery = async (id: number) => {
  const tab = resultTabs.find((t) => t.id === id);
  if (!tab || !tab.running) {
    return;
  }
  try {
    const requested = await api.cancelQuery(tab.connection);
    // 実行が直前に完了していた等でキャンセル対象が無かった場合の通知
    if (!requested) {
      toast.info("No running query to cancel. It may have just finished.");
    }
  } catch (e) {
    toast.error("Failed to cancel the query", {
      description: toErrorMessage(e),
    });
  }
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
  // 同一接続の並列実行を抑止する (別タブで実行中でも拒否)
  if (isConnectionRunning(selectedConnection)) {
    toast.warning(
      "A query is already running on this connection. Cancel it or wait for it to finish.",
    );
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
  // 同一接続の並列実行を抑止する (別タブで実行中でも拒否)
  if (isConnectionRunning(tab.connection)) {
    toast.warning(
      "A query is already running on this connection. Cancel it or wait for it to finish.",
    );
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
  /// SQL 補完用のテーブル名 → カラム名リストのマップ (未取得なら null)
  get schemaMap() {
    return schemaMap;
  },
  loadConnections,
  loadSchemaMap,
  reloadConnections,
  selectConnection,
  changeActiveSchema,
  selectFile,
  createFile,
  deleteFile,
  saveCurrentFile,
  updateEditorContent,
  insertSqlSnippet,
  isConnectionRunning,
  runQuery,
  cancelQuery,
  rerunTab,
  selectResultTab,
  closeResultTab,
  toggleResultTabPin,
};
