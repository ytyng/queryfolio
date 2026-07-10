import { toast } from "svelte-sonner";
import * as api from "$lib/api";
import type { AiInfo, ConnectionInfo, QueryResult } from "$lib/api";

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
  /// AI にエラー修正案を問い合わせ中 (スピナー表示・二重実行防止)
  fixing: boolean;
  /// AI が返した修正案の SQL (無ければ null)。自動実行はせず、
  /// ユーザーの Apply でエディタに挿入する
  fixSuggestion: string | null;
}

/// タブの SQL が EXPLAIN 由来かを判定する (Analyze with AI ボタンの表示用)。
/// Explain ボタンで組み立てた SQL は必ず EXPLAIN で始まるため、
/// 先頭キーワードの一致で判定する (手入力の EXPLAIN も対象になる)
export const isExplainSql = (sql: string): boolean =>
  sql.trimStart().toLowerCase().startsWith("explain");

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
/// AI 設定の情報 (未取得・取得失敗時は null)
let aiInfo = $state<AiInfo | null>(null);
/// AI 設定の解決エラー (不明 provider 等。ボタンの title で案内する)
let aiError = $state<string | null>(null);
/// AI で SQL 生成中 (ボタンのスピナー表示・二重送信防止)
let aiGenerating = $state(false);
/// AI で実行計画を解説中 (ボタンのスピナー表示・二重送信防止)
let aiAnalyzing = $state(false);
/// AI による実行計画解説の Markdown (モーダル表示中のみ非 null)
let aiAnalysis = $state<string | null>(null);
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
    // 接続設定の解決結果はバックエンドにキャッシュ済みなので、
    // AI 設定の取得はここでは軽い (取得コマンドの再実行は起きない)
    await loadAiInfo();
  } catch (e) {
    errorMessage = toErrorMessage(e);
    connections = [];
  } finally {
    loadingConnections = false;
  }
};

/// AI 設定の情報 (configured / model) を取得する。
/// 未設定は configured: false で返り、設定の解決エラー
/// (不明 provider 等) は aiError に入れて AI ボタンの title で案内する。
const loadAiInfo = async () => {
  try {
    aiInfo = await api.getAiInfo();
    aiError = null;
  } catch (e) {
    aiInfo = null;
    aiError = toErrorMessage(e);
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
  aiInfo = null;
  aiError = null;
  aiAnalysis = null;
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
/// 置換ではなく追記にする)。実行はしない。挿入できたら true を返す。
/// エディタへの反映は SqlEditor 側の $effect が行う。
const insertSqlSnippet = (sql: string): boolean => {
  if (!selectedConnection) {
    toast.warning("Select a connection first");
    return false;
  }
  if (!selectedFile) {
    toast.warning("Select or create a query file first");
    return false;
  }
  const trimmed = editorContent.replace(/\s+$/, "");
  updateEditorContent(trimmed ? `${trimmed}\n\n${sql}\n` : `${sql}\n`);
  return true;
};

/// 自然言語の指示から AI で SQL を生成し、エディタに挿入する
/// (自動実行はしない。ユーザーが内容を確認してから実行する)。
/// 成功したら true を返す (入力欄を閉じる判定に使う)。
const generateSql = async (instruction: string): Promise<boolean> => {
  if (!selectedConnection) {
    toast.warning("Select a connection first");
    return false;
  }
  if (!selectedFile) {
    toast.warning("Select or create a query file first");
    return false;
  }
  if (!instruction.trim()) {
    toast.warning("Enter an instruction for the SQL to generate");
    return false;
  }
  if (aiGenerating) {
    return false;
  }
  aiGenerating = true;
  try {
    const sql = await api.aiGenerateSql(selectedConnection, instruction);
    if (!sql.trim()) {
      toast.warning("The AI returned an empty response");
      return false;
    }
    // 生成中に接続・ファイルの選択が外れた場合は挿入されない
    // (insertSqlSnippet が warning を出す) ため、成功時のみ通知する
    if (!insertSqlSnippet(sql)) {
      return false;
    }
    toast.success("Generated SQL inserted into the editor");
    return true;
  } catch (e) {
    toast.error("Failed to generate SQL", {
      description: toErrorMessage(e),
    });
    return false;
  } finally {
    aiGenerating = false;
  }
};

/// タブに記録された SQL とエラーメッセージから、AI に修正案を問い合わせて
/// タブへ書き込む (自動実行はしない。ユーザーが Apply でエディタに挿入する)。
const fixSqlWithAi = async (tabId: number) => {
  const tab = resultTabs.find((t) => t.id === tabId);
  if (!tab || !tab.error || !tab.sql.trim()) {
    return;
  }
  // 二重実行防止 (ボタンも disabled にしているが防御的にガードする)
  if (tab.fixing) {
    return;
  }
  // 問い合わせ中に Re-run されて別の実行結果になった場合に、
  // 古いエラーへの修正案を書き込まないよう実行時刻を控えておく
  const requestedExecutedAt = tab.executedAt;
  tab.fixing = true;
  try {
    const fixed = await api.aiFixSql(tab.connection, tab.sql, tab.error);
    // 問い合わせ中に設定再読込などでタブが破棄された・再実行で
    // 結果が入れ替わった場合は、古い修正案を捨てる
    const current = resultTabs.find((t) => t.id === tabId);
    if (!current || current.executedAt !== requestedExecutedAt) {
      return;
    }
    if (!fixed.trim()) {
      toast.warning("The AI returned an empty response");
      return;
    }
    tab.fixSuggestion = fixed;
  } catch (e) {
    toast.error("Failed to get a fix suggestion", {
      description: toErrorMessage(e),
    });
  } finally {
    tab.fixing = false;
  }
};

/// AI の修正案をエディタに挿入して提案表示を閉じる (実行はしない)。
/// 挿入できなかった場合 (接続・ファイル未選択・接続の切替) は提案を残す。
const applyFixSuggestion = (tabId: number) => {
  const tab = resultTabs.find((t) => t.id === tabId);
  if (!tab?.fixSuggestion) {
    return;
  }
  // 結果タブは接続をまたいで残るため、提案表示中に接続を切り替えると
  // 別接続 (別方言) のファイルに挿入されてしまう。誤挿入を防ぐ
  if (selectedConnection !== tab.connection) {
    toast.warning(
      `This suggestion is for '${tab.connection}'. Switch back to that connection to apply it.`,
    );
    return;
  }
  if (insertSqlSnippet(tab.fixSuggestion)) {
    toast.success("Fixed SQL inserted into the editor");
    tab.fixSuggestion = null;
  }
};

/// AI の修正案を破棄して提案表示を閉じる。
const dismissFixSuggestion = (tabId: number) => {
  const tab = resultTabs.find((t) => t.id === tabId);
  if (tab) {
    tab.fixSuggestion = null;
  }
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
    fixing: false,
    fixSuggestion: null,
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
  // 前回エラーへの修正案は再実行で古くなるため破棄する
  tab.fixSuggestion = null;
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

/// カーソル位置の文にエンジン別の EXPLAIN プレフィックスを付けて実行する。
/// プレフィックスの組み立てと対象判定 (SELECT / WITH のみ) はバックエンドの
/// build_explain_sql が行い、対象外の文は toast で断る。
const explainQuery = async (sql: string) => {
  if (!selectedConnection) {
    toast.warning("Select a connection first");
    return;
  }
  if (!sql.trim()) {
    toast.warning("There is no SQL statement to explain");
    return;
  }
  let explainSql: string;
  try {
    explainSql = await api.buildExplainSql(selectedConnection, sql);
  } catch (e) {
    // 対象外の文 (DML 等) や不明エンジン。実行前の断りなので warning にする
    toast.warning(toErrorMessage(e));
    return;
  }
  await runQuery(explainSql);
};

/// EXPLAIN 結果を AI に渡すテキストに整形する (ヘッダ + タブ区切り行)。
/// 渡すのは実行計画テキストのみ (EXPLAIN 出力なので結果データではない)
const formatPlanText = (result: QueryResult): string => {
  const cellText = (value: unknown): string =>
    value === null || value === undefined
      ? "NULL"
      : typeof value === "object"
        ? JSON.stringify(value)
        : String(value);
  const lines = result.rows.map((row) => row.map(cellText).join("\t"));
  return [result.columns.join("\t"), ...lines].join("\n");
};

/// EXPLAIN 結果のタブを AI に解説させ、Markdown をモーダルに表示する
const analyzeExplainTab = async (id: number) => {
  const tab = resultTabs.find((t) => t.id === id);
  if (!tab || !tab.result || aiAnalyzing) {
    return;
  }
  aiAnalyzing = true;
  try {
    const text = await api.aiExplainPlan(
      tab.connection,
      tab.sql,
      formatPlanText(tab.result),
    );
    if (!text.trim()) {
      toast.warning("The AI returned an empty response");
      return;
    }
    aiAnalysis = text;
  } catch (e) {
    toast.error("Failed to analyze the execution plan", {
      description: toErrorMessage(e),
    });
  } finally {
    aiAnalyzing = false;
  }
};

/// AI 解説モーダルを閉じる
const closeAiAnalysis = () => {
  aiAnalysis = null;
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
  get aiInfo() {
    return aiInfo;
  },
  get aiError() {
    return aiError;
  },
  get aiGenerating() {
    return aiGenerating;
  },
  get aiAnalyzing() {
    return aiAnalyzing;
  },
  /// AI による実行計画解説の Markdown (モーダル表示中のみ非 null)
  get aiAnalysis() {
    return aiAnalysis;
  },
  /// SQL 補完用のテーブル名 → カラム名リストのマップ (未取得なら null)
  get schemaMap() {
    return schemaMap;
  },
  loadConnections,
  loadAiInfo,
  generateSql,
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
  fixSqlWithAi,
  applyFixSuggestion,
  dismissFixSuggestion,
  isConnectionRunning,
  runQuery,
  explainQuery,
  analyzeExplainTab,
  closeAiAnalysis,
  cancelQuery,
  rerunTab,
  selectResultTab,
  closeResultTab,
  toggleResultTabPin,
};
