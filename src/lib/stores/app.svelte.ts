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

/// 1 つの開いているエディタタブ。タブはグローバル (全接続横断) に一列で並び、
/// 各タブが自分の接続を保持する。タブをアクティブにすると、その接続へ切り替わる
/// (engine / スキーマ / 補完 / 実行はすべてアクティブタブの接続で駆動される)。
export interface EditorTab {
  id: number;
  connection: string;
  file: string;
  content: string;
  /// 未保存の編集があるか (自動保存で false に戻る)
  dirty: boolean;
}

let connections = $state<ConnectionInfo[]>([]);
let selectedConnection = $state<string | null>(null);
/// Writable スイッチ。false (既定) の間は SELECT/SHOW 等の副作用の無い
/// 文しか実行できない (バックエンドが強制)。事故防止のためセッションごとに
/// OFF から始め、永続化しない (再起動で勝手に書き込み可にはしない)。
let writable = $state(false);
let files = $state<string[]>([]);
let editorTabs = $state<EditorTab[]>([]);
let activeEditorTabId = $state<number | null>(null);
let resultTabs = $state<ResultTab[]>([]);
let activeTabId = $state<number | null>(null);
let errorMessage = $state<string | null>(null);
let loadingConnections = $state(false);
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
/// AI で選択 SQL を解説中 (ボタンのスピナー表示・二重送信防止)
let aiExplaining = $state(false);
/// AI による選択 SQL 解説の Markdown (モーダル表示中のみ非 null)
let aiExplanation = $state<string | null>(null);
/// SQL 補完用のテーブル名 → カラム名リストのマップ (未取得・取得失敗は null)
let schemaMap = $state<Record<string, string[]> | null>(null);
/// 危険な文 (allow_dangerous_statements 有効な接続) の実行前確認ダイアログ。
/// 非 null の間モーダルを表示し、ユーザーの応答を resolve へ渡す
let dangerousConfirm = $state<{
  reason: string;
  resolve: (ok: boolean) => void;
} | null>(null);

// 結果タブ ID の連番 (セッション内で一意なら十分なので永続化しない)
let nextTabId = 1;
// エディタタブ ID の連番
let nextEditorTabId = 1;
/// 接続ごとに最後にアクティブだったエディタタブ ID を覚え、接続へ戻った時に復元する
const lastActiveTabByConnection = new Map<string, number>();

const getActiveEditorTab = (): EditorTab | null =>
  editorTabs.find((t) => t.id === activeEditorTabId) ?? null;

/// 実行中の loadSchemaMap の世代番号。接続・スキーマの連続切替で
/// 古い応答が後から解決しても、最新の要求の結果だけを反映するために使う
let schemaMapGeneration = 0;

/// 実行中の applyConnectionContext の世代番号。接続の連続切替で、遅い接続の
/// 応答が後から解決して新しい接続の files / schemas / activeSchema を上書き
/// しないよう、コミット前に最新世代かを検査する (schemaMapGeneration と同趣旨)
let connectionContextGeneration = 0;

let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
/// 自動保存が予約されているエディタタブ ID (デバウンス中の対象)
let autoSavePendingTabId: number | null = null;

const toErrorMessage = (e: unknown): string =>
  typeof e === "string" ? e : e instanceof Error ? e.message : String(e);

/// 指定接続に対する実効 Writable。Writable スイッチはツールバーに 1 つで、
/// 現在選択中の接続の状態を表すため、別接続 (別接続タブの再実行など) では
/// 常に false (読み取り専用) になる。実行ガードと危険文確認の両方でこの値を使い、
/// 「トグルが示す接続にだけ書き込みを許可する」意味を一貫させる。
const effectiveWritable = (connection: string): boolean =>
  connection === selectedConnection && writable;

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
  // タブを破棄するので、pending だけでなく全ての未保存タブを先に保存する
  if (!(await saveAllDirtyTabs())) {
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
  // 設定リロードで接続が入れ替わるため、Writable も安全側 (OFF) へ戻す
  writable = false;
  files = [];
  // 設定が丸ごと入れ替わるため、開いているエディタタブを全て破棄する
  if (autoSaveTimer) {
    clearTimeout(autoSaveTimer);
    autoSaveTimer = null;
  }
  autoSavePendingTabId = null;
  // reset 前後に接続切替 (applyConnectionContext) が in-flight でも、その古い
  // 応答が後から commit して stale な接続を復活させないよう世代を進める
  connectionContextGeneration++;
  editorTabs = [];
  activeEditorTabId = null;
  lastActiveTabByConnection.clear();
  // 設定が丸ごと入れ替わるため、ピン留め含め全タブを破棄する
  resultTabs = [];
  activeTabId = null;
  schemas = [];
  activeSchema = null;
  aiInfo = null;
  aiError = null;
  aiAnalysis = null;
  aiExplanation = null;
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
  const pendingId = autoSavePendingTabId;
  autoSavePendingTabId = null;
  // デバウンス予約のタブ (= 直近まで編集していたタブ) だけを確定させる。
  // 他タブの保存失敗でナビゲーションを巻き込まないよう、対象は 1 タブに限る
  // (エディタタブは接続をまたいで残るため、切替で内容が失われることはない)。
  if (pendingId == null) {
    return true;
  }
  const tab = editorTabs.find((t) => t.id === pendingId);
  if (tab && tab.dirty) {
    return saveEditorTab(tab);
  }
  return true;
};

/// 全ての dirty なエディタタブを保存する (best-effort)。全て成功したら true。
/// タブを破棄する前 (reloadConnections) に呼び、未保存の編集を失わないようにする。
/// 自動保存に失敗して pending が外れた dirty タブもここで確実に対象になる。
const saveAllDirtyTabs = async (): Promise<boolean> => {
  if (autoSaveTimer) {
    clearTimeout(autoSaveTimer);
    autoSaveTimer = null;
  }
  autoSavePendingTabId = null;
  let ok = true;
  for (const tab of editorTabs) {
    if (tab.dirty) {
      if (!(await saveEditorTab(tab))) {
        ok = false;
      }
    }
  }
  return ok;
};

/// 指定した接続のファイル一覧・スキーマ・補完マップを読み込み、接続コンテキストを
/// 切り替える (エディタタブには触れない)。接続選択・タブアクティブ化の両方から使う。
///
/// 重要: `selectedConnection` は読み込みを終えてから **最後にまとめて** 反映する。
/// 先に `selectedConnection = name` すると、await 中は「接続は新 (name) だがエディタは
/// まだ旧タブの SQL を表示中」というズレが生じ、その窓で Run すると旧 SQL が新接続で
/// 走ってしまう (DB クライアントとして致命的)。呼び出し側の `activeEditorTabId` 反映は
/// この関数の resolve 直後の同一マイクロタスクで行われるため、コミット〜タブ反映の間に
/// ユーザー操作 (マクロタスク) は割り込めず、接続とタブは常に整合する。
///
/// コミットできたら true。連続切替でより新しい要求に追い越された場合は、何も反映せず
/// false を返す (呼び出し側は activeEditorTabId を触らずに中断する)。
const applyConnectionContext = async (name: string): Promise<boolean> => {
  const generation = ++connectionContextGeneration;
  const defaultSchema = connections.find((c) => c.name === name)?.schema ?? null;
  let loadedFiles: string[] = [];
  let filesError: string | null = null;
  try {
    loadedFiles = await api.listQueryFiles(name);
  } catch (e) {
    filesError = toErrorMessage(e);
  }
  // await の間により新しい切替が始まっていたら、この応答は捨てる (上書き防止)
  if (generation !== connectionContextGeneration) {
    return false;
  }
  // スキーマ一覧の取得は接続確立を伴うため、失敗しても選択自体は成立させる
  // (エラーは結果ペインに出さず、プルダウンを現在値のみにする)
  let schema = defaultSchema;
  let loadedSchemas: string[] = [];
  try {
    schema = (await api.getActiveSchema(name)) ?? schema;
    loadedSchemas = await api.listSchemas(name);
  } catch {
    loadedSchemas = schema ? [schema] : [];
  }
  if (generation !== connectionContextGeneration) {
    return false;
  }
  // ここから resolve まで await を挟まず、接続コンテキストを一括反映する。
  // 別の接続へ切り替わったら Writable を安全側 (OFF) へ戻す。ある接続で
  // 書き込みを許可したまま別接続 (本番など) に移り、誤って書き込む事故を防ぐ。
  // 同一接続の再選択 (同接続のエディタタブ切替など) では維持する。
  if (selectedConnection !== name) {
    writable = false;
  }
  selectedConnection = name;
  errorMessage = filesError;
  files = filesError ? [] : loadedFiles;
  activeSchema = schema;
  schemas = loadedSchemas;
  // 補完候補は新接続のものを取り直す (世代も進め、古い取得の後追い書き込みを防ぐ)
  schemaMapGeneration++;
  schemaMap = null;
  // SQL 補完用のスキーママップをバックグラウンドで取得する (待たない)
  void loadSchemaMap();
  return true;
};

/// 接続に紐づくエディタタブのうち、アクティブに復元すべきものを選ぶ。
/// 直近にアクティブだったタブを優先し、無ければ最後に開いたタブ、無ければ null。
const pickTabForConnection = (name: string): number | null => {
  const remembered = lastActiveTabByConnection.get(name);
  if (
    remembered != null &&
    editorTabs.some((t) => t.id === remembered && t.connection === name)
  ) {
    return remembered;
  }
  for (let i = editorTabs.length - 1; i >= 0; i--) {
    if (editorTabs[i].connection === name) {
      return editorTabs[i].id;
    }
  }
  return null;
};

const selectConnection = async (name: string) => {
  if (name === selectedConnection) {
    // 現在の接続を再選択したら、進行中の別接続への切替 (applyConnectionContext は
    // commit まで selectedConnection を変えないため、その最中は現接続が選択中に
    // 見える) をキャンセルする。世代を進めておくと in-flight の切替は commit されず、
    // 「今の接続に留まる」という操作の意図どおりになる。
    connectionContextGeneration++;
    return;
  }
  // 未保存タブは best-effort で保存するが、保存失敗でも切替は止めない。
  // エディタタブは接続をまたいで残るため、切替で内容が失われることはない
  // (書込不可などで保存に失敗しても、dirty のままタブに保持される)。
  await flushPendingSave();
  // 結果タブ・エディタタブは接続をまたいで残す (接続切替では破棄しない)。
  // より新しい切替に追い越されたら、タブ選択を触らず中断する。
  if (!(await applyConnectionContext(name))) {
    return;
  }
  // この接続で最後に開いていたタブを復元する (無ければエディタは空表示)
  activeEditorTabId = pickTabForConnection(name);
};

/// エディタタブをアクティブにする。タブの接続が現在の接続と違えば、その接続へ
/// 切り替える (files / スキーマ / 補完もタブの接続のものに揃える)。
const activateEditorTab = async (id: number) => {
  if (id === activeEditorTabId) {
    return;
  }
  const tab = editorTabs.find((t) => t.id === id);
  if (!tab) {
    return;
  }
  // 未保存タブは best-effort で保存するが、保存失敗でもアクティブ化は止めない。
  // (書込不可などで保存に失敗しても未保存 SQL を閲覧・コピーできるようにする。
  //  タブは残るので内容は失われない)
  await flushPendingSave();
  if (tab.connection !== selectedConnection) {
    // より新しい切替に追い越されたら、このタブをアクティブにしない
    if (!(await applyConnectionContext(tab.connection))) {
      return;
    }
  }
  activeEditorTabId = id;
  lastActiveTabByConnection.set(tab.connection, id);
};

// アクティブスキーマ (database) を切り替える。成功したら true。
const changeActiveSchema = async (schema: string): Promise<boolean> => {
  const connection = selectedConnection;
  if (!connection || schema === activeSchema) {
    return true;
  }
  // 未保存タブは best-effort で保存 (失敗してもスキーマ切替は止めない)
  await flushPendingSave();
  try {
    await api.setActiveSchema(connection, schema);
    // 切替中に別接続へ移っていたら、そのスキーマ表示を新接続に適用しない
    if (selectedConnection !== connection) {
      return false;
    }
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

/// ファイルを開く。既に開いているタブがあればアクティブにし、無ければ
/// 内容を読み込んで新しいタブを作りアクティブにする (FilesPane から呼ばれる)。
const selectFile = async (fileName: string) => {
  // 読み込み先の接続を await 前に固定する。読込中に接続が切り替わっても、
  // タブは必ず「内容を読んだ接続」に紐づける (誤った接続への実行/保存を防ぐ)。
  const connection = selectedConnection;
  if (!connection) {
    return;
  }
  const existing = editorTabs.find(
    (t) => t.connection === connection && t.file === fileName,
  );
  if (existing) {
    await activateEditorTab(existing.id);
    return;
  }
  // 未保存タブは best-effort で保存 (失敗してもファイルオープンは止めない)
  await flushPendingSave();
  try {
    const content = await api.readQueryFile(connection, fileName);
    // 読込中に別接続へ切り替わっていたら、この読込結果は捨てる
    // (ユーザーはもうその接続を見ていないので、開かない)
    if (selectedConnection !== connection) {
      return;
    }
    const tab: EditorTab = {
      id: nextEditorTabId++,
      connection,
      file: fileName,
      content,
      dirty: false,
    };
    editorTabs = [...editorTabs, tab];
    activeEditorTabId = tab.id;
    lastActiveTabByConnection.set(connection, tab.id);
    errorMessage = null;
  } catch (e) {
    errorMessage = toErrorMessage(e);
  }
};

/// エディタタブを閉じる。未保存なら閉じる前に保存する (best-effort)。
/// アクティブタブを閉じたら右隣 (無ければ左隣) をアクティブにする。
const removeEditorTab = async (id: number, save: boolean) => {
  const tab = editorTabs.find((t) => t.id === id);
  if (!tab) {
    return;
  }
  // このタブの自動保存予約が残っていれば解除する (閉じた後に走らせない)
  if (autoSavePendingTabId === id) {
    if (autoSaveTimer) {
      clearTimeout(autoSaveTimer);
      autoSaveTimer = null;
    }
    autoSavePendingTabId = null;
  }
  if (save && tab.dirty) {
    // 保存に失敗したら閉じない (未保存内容をメモリごと失わないため)
    if (!(await saveEditorTab(tab))) {
      return;
    }
    // 保存 await の間にさらに編集された場合、saveEditorTab は dirty を残す。
    // その編集を失わないよう close を中断する (自動保存タイマーが後で確定させる)
    if (tab.dirty) {
      return;
    }
  }
  // 保存 await 中に配列が変わっている可能性があるため、位置は取り直す
  const index = editorTabs.findIndex((t) => t.id === id);
  if (index < 0) {
    return;
  }
  editorTabs = editorTabs.filter((t) => t.id !== id);
  if (lastActiveTabByConnection.get(tab.connection) === id) {
    lastActiveTabByConnection.delete(tab.connection);
  }
  if (activeEditorTabId === id) {
    // filter 後、元 index の位置には右隣タブが繰り上がっている
    const neighbor = editorTabs[index] ?? editorTabs[index - 1] ?? null;
    activeEditorTabId = null;
    if (neighbor) {
      await activateEditorTab(neighbor.id);
    }
  }
};

const closeEditorTab = (id: number) => {
  void removeEditorTab(id, true);
};

const createFile = async (fileName: string) => {
  const connection = selectedConnection;
  if (!connection) {
    return;
  }
  try {
    const normalized = await api.createQueryFile(connection, fileName);
    // 作成中に別接続へ切り替わっていたら、新接続の一覧を汚さず開かない
    if (selectedConnection !== connection) {
      return;
    }
    files = await api.listQueryFiles(connection);
    await selectFile(normalized);
  } catch (e) {
    errorMessage = toErrorMessage(e);
  }
};

const deleteFile = async (fileName: string) => {
  const connection = selectedConnection;
  if (!connection) {
    return;
  }
  try {
    await api.deleteQueryFile(connection, fileName);
    // 削除したファイルの開いているタブを閉じる (ファイルは消えたので保存しない)
    const victims = editorTabs.filter(
      (t) => t.connection === connection && t.file === fileName,
    );
    for (const v of victims) {
      await removeEditorTab(v.id, false);
    }
    // タブを閉じる過程で接続が切り替わっていなければ一覧を更新する
    if (selectedConnection === connection) {
      files = await api.listQueryFiles(connection);
    }
  } catch (e) {
    errorMessage = toErrorMessage(e);
  }
};

// ファイルをリネームする。成功したら正規化後の新ファイル名、失敗したら null。
// 対象ファイルを開いているタブがあればリネーム前に保存し、成功後は追従する。
const renameFile = async (
  oldName: string,
  newName: string,
): Promise<string | null> => {
  // await をまたぐ間に接続が切り替わっても、リネームは開始時の接続に対して
  // 行う (flushPendingSave 中の接続切替による取り違えを防ぐ)
  const connection = selectedConnection;
  if (!connection) {
    return null;
  }
  // 対象ファイルを開いているタブがあれば未保存内容を先に確定させる
  const opened = editorTabs.some(
    (t) => t.connection === connection && t.file === oldName,
  );
  if (opened && !(await flushPendingSave())) {
    return null;
  }
  try {
    const normalized = await api.renameQueryFile(connection, oldName, newName);
    // リネーム中に接続が切り替わっていたら、旧接続の一覧で上書きしない
    if (selectedConnection === connection) {
      files = await api.listQueryFiles(connection);
    }
    // 開いているタブのファイル名を追従させる
    for (const t of editorTabs) {
      if (t.connection === connection && t.file === oldName) {
        t.file = normalized;
      }
    }
    errorMessage = null;
    return normalized;
  } catch (e) {
    errorMessage = toErrorMessage(e);
    return null;
  }
};

// エディタタブの内容を保存する。成功したら true。
// 失敗時は dirty を保持したまま errorMessage を設定する。
const saveEditorTab = async (tab: EditorTab): Promise<boolean> => {
  // 書き込み中にさらに編集された場合、その古い保存完了で新しい編集の dirty を
  // 消してはならない (lost update 防止)。保存した内容を控え、完了時に内容が
  // 変わっていない時だけ dirty を下ろす。
  const saved = tab.content;
  try {
    await api.writeQueryFile(tab.connection, tab.file, saved);
    if (tab.content === saved) {
      tab.dirty = false;
    }
    return true;
  } catch (e) {
    errorMessage = `Failed to save the file: ${toErrorMessage(e)}`;
    return false;
  }
};

// アクティブなエディタタブを保存する (Toolbar 等から明示保存する場合用)。
const saveCurrentFile = async (): Promise<boolean> => {
  const tab = getActiveEditorTab();
  if (!tab) {
    return true;
  }
  return saveEditorTab(tab);
};

/// エディタからの変更通知。アクティブタブの内容を更新し、自動保存を
/// デバウンスして予約する。予約は編集中のタブを対象にする。
const updateEditorContent = (content: string) => {
  const tab = getActiveEditorTab();
  if (!tab || content === tab.content) {
    return;
  }
  tab.content = content;
  tab.dirty = true;
  autoSavePendingTabId = tab.id;
  if (autoSaveTimer) {
    clearTimeout(autoSaveTimer);
  }
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null;
    const id = autoSavePendingTabId;
    autoSavePendingTabId = null;
    const target = editorTabs.find((t) => t.id === id);
    if (target && target.dirty) {
      void saveEditorTab(target);
    }
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
  const tab = getActiveEditorTab();
  if (!tab) {
    toast.warning("Select or create a query file first");
    return false;
  }
  const trimmed = tab.content.replace(/\s+$/, "");
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
  if (!getActiveEditorTab()) {
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

/// セル編集を適用中 (run_statements 実行中) の接続。クエリ実行と同様に
/// 同一接続の並列実行を抑止するため isConnectionRunning に含める。
let applyingConnections = $state(new Set<string>());

/// 指定した接続でクエリ実行中のタブがあるかを返す。
/// バックエンドのキャンセルレジストリは接続単位で最後の実行しか
/// 管理しないため、同一接続の並列実行はフロント側で抑止する
/// (許すとキャンセル対象の取り違えや取りこぼしが起きる)。
/// セル編集の適用中 (applyingConnections) も実行中として扱う。
const isConnectionRunning = (connection: string): boolean =>
  resultTabs.some((t) => t.running && t.connection === connection) ||
  applyingConnections.has(connection);

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
    result = await api.runQuery(
      tab.connection,
      tab.sql,
      undefined,
      effectiveWritable(tab.connection),
    );
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
  // \c でアクティブスキーマが切り替わっていたら表示を追従させる
  // (切替自体はバックエンドで完了済み)
  if (result?.switched_schema) {
    applySwitchedSchema(tab.connection, result.switched_schema);
  }
};

/// `\c` によるスキーマ切替をフロントの状態へ反映する。
/// スキーマブラウザは activeSchema の変化を購読しているので自動で追従し、
/// SQL 補完のスキーママップはここで取り直す。
const applySwitchedSchema = (connection: string, schema: string) => {
  // 実行中に別接続へ移っていたら、そのスキーマ表示を新接続に適用しない
  if (selectedConnection !== connection || activeSchema === schema) {
    return;
  }
  activeSchema = schema;
  // 補完候補は切替先のものを取り直す (待たない)
  void loadSchemaMap();
  toast.success(`Switched to ${schema}`);
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

/// 危険な文の実行確認を求め、ユーザーの応答 (true=実行) を待つ。
/// 直前の未応答の確認が残っていれば却下してから差し替える
const requestDangerousConfirm = (reason: string): Promise<boolean> =>
  new Promise((resolve) => {
    if (dangerousConfirm) {
      dangerousConfirm.resolve(false);
    }
    dangerousConfirm = { reason, resolve };
  });

/// 確認ダイアログの応答 (モーダルから呼ぶ)。ok=true で実行を続行する
const resolveDangerousConfirm = (ok: boolean) => {
  if (!dangerousConfirm) {
    return;
  }
  const { resolve } = dangerousConfirm;
  dangerousConfirm = null;
  resolve(ok);
};

/// allow_dangerous_statements が有効な接続で、危険な文なら実行前に確認を出す。
/// 実行してよければ true、キャンセルなら false を返す。
/// 無効な接続では常に true を返し (バックエンドの run_query が拒否する)、
/// 危険判定の呼び出し失敗時も true を返して実行に委ねる (allow が意図のため)。
const confirmIfDangerous = async (
  connection: string,
  sql: string,
): Promise<boolean> => {
  const info = connections.find((c) => c.name === connection);
  // 読み取り専用が効いている間 (config readonly、またはこの接続に対する実効
  // Writable が OFF) は、書き込み系の文をバックエンドが Read-only として拒否する。
  // 破壊的操作の確認は実際に実行され得る文にだけ意味があるため、ここでは確認を
  // 出さず実行へ委ねる (バックエンドが明快な Read-only エラーを返す)。実効 Writable
  // を使うので、別接続のタブ再実行 (常に読み取り専用扱い) でも無駄な確認を出さない。
  if (info?.readonly || !effectiveWritable(connection)) {
    return true;
  }
  if (!info?.allow_dangerous_statements) {
    return true;
  }
  let reason: string | null = null;
  try {
    reason = await api.checkDangerousStatement(connection, sql);
  } catch (e) {
    // 判定に失敗しても実行に委ねる (allow が意図)。ただし本当のバグを
    // 握り潰さないよう、失敗自体はコンソールに残す
    console.warn("checkDangerousStatement failed; running without confirm", e);
    return true;
  }
  if (!reason) {
    return true;
  }
  return await requestDangerousConfirm(reason);
};

const runQuery = async (sql: string) => {
  // 実行先の接続を await 前に固定する。以降の await (保存・危険文の確認モーダル) の
  // 間に接続が切り替わっても、確認した接続と実行する接続が食い違わないようにする
  // (旧 SQL を新 DB で実行してしまう事故を防ぐ)。
  const connection = selectedConnection;
  // 実行前ガードの通知は、既存の結果タブを覆わないよう toast で出す
  if (!connection) {
    toast.warning("Select a connection first");
    return;
  }
  if (!sql.trim()) {
    toast.warning("There is no SQL statement to run");
    return;
  }
  // 同一接続の並列実行を抑止する (別タブで実行中でも拒否)
  if (isConnectionRunning(connection)) {
    toast.warning(
      "A query is already running on this connection. Cancel it or wait for it to finish.",
    );
    return;
  }
  // 未保存タブは best-effort で保存 (失敗しても実行は止めない。SQL はメモリ上の値)
  await flushPendingSave();
  // 危険な文 (WHERE 無し UPDATE/DELETE、DROP/TRUNCATE) は、実行を許可した
  // 接続でも実行前に確認する。キャンセルされたら何もしない
  if (!(await confirmIfDangerous(connection, sql))) {
    return;
  }
  // 確認モーダルの間に接続が切り替わっていたら、別 DB で実行しないよう中止する
  if (selectedConnection !== connection) {
    return;
  }
  errorMessage = null;
  const tab = prepareTargetTab();
  if (!tab) {
    return;
  }
  tab.sql = sql;
  tab.connection = connection;
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
  // 変数名は module 直下の explainSql アクションと被らないよう別名にする
  let explainStatement: string;
  try {
    explainStatement = await api.buildExplainSql(selectedConnection, sql);
  } catch (e) {
    // 対象外の文 (DML 等) や不明エンジン。実行前の断りなので warning にする
    toast.warning(toErrorMessage(e));
    return;
  }
  await runQuery(explainStatement);
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

/// カーソル位置の SQL 文を AI に平易に解説させ、Markdown をモーダルに
/// 表示する (実行はしない)。LLM に送るのは SQL とスキーマ情報のみで、
/// クエリの結果データは送らない (バックエンドの ai_explain_sql 参照)
const explainSql = async (sql: string) => {
  if (!selectedConnection) {
    toast.warning("Select a connection first");
    return;
  }
  if (!sql.trim()) {
    toast.warning("There is no SQL statement to explain");
    return;
  }
  // 二重実行防止 (ボタンも disabled にしているが防御的にガードする)
  if (aiExplaining) {
    return;
  }
  aiExplaining = true;
  try {
    const text = await api.aiExplainSql(selectedConnection, sql);
    if (!text.trim()) {
      toast.warning("The AI returned an empty response");
      return;
    }
    aiExplanation = text;
  } catch (e) {
    toast.error("Failed to explain the SQL statement", {
      description: toErrorMessage(e),
    });
  } finally {
    aiExplaining = false;
  }
};

/// AI による選択 SQL 解説のモーダルを閉じる
const closeAiExplanation = () => {
  aiExplanation = null;
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
  // 再実行でも危険な文は確認する (通常実行と同じガード)
  if (!(await confirmIfDangerous(tab.connection, tab.sql))) {
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

/// 結果グリッドのセル編集 (UPDATE 群) を 1 トランザクションで適用する。
/// 適用は「アクティブ接続かつ Writable ON」の時だけ (UI でも抑止しているが、
/// 別接続タブの誤適用や無駄な往復を防ぐためここでもガードする)。成功したら
/// 表示値を実際の DB 状態に合わせるため再実行し、true を返す。
const submitCellEdits = async (
  tabId: number,
  statements: string[],
): Promise<boolean> => {
  const tab = resultTabs.find((t) => t.id === tabId);
  if (!tab || statements.length === 0) {
    return false;
  }
  if (!effectiveWritable(tab.connection)) {
    toast.warning(
      "Turn on the Writable switch for this connection to apply edits.",
    );
    return false;
  }
  // 生成 UPDATE はスキーマ未修飾で「接続の現在のアクティブスキーマ」に走るため、
  // 編集後にスキーマを切り替えていると別スキーマの同名テーブルを更新し得る。
  // タブの実行時スキーマと現在のアクティブスキーマが違えば適用しない
  // (UI の canEditActiveConnection は新規編集のみ抑止するので、Submit 側でも防ぐ)。
  if (tab.schema !== activeSchema) {
    toast.warning(
      "The active schema changed since these edits were made. Cancel them and re-run the query.",
    );
    return false;
  }
  // 同一接続で実行中 (クエリまたは別の適用) なら適用しない
  // (キャンセル対象の取り違え防止・Submit 連打防止。runQuery と同じ不変条件)
  if (isConnectionRunning(tab.connection)) {
    toast.warning(
      "A query is already running on this connection. Wait for it to finish.",
    );
    return false;
  }
  const connection = tab.connection;
  // 適用中は接続を「実行中」に登録し、並列実行・二重 Submit を抑止する
  applyingConnections = new Set(applyingConnections).add(connection);
  let affected: number;
  try {
    affected = await api.runStatements(
      connection,
      statements,
      effectiveWritable(connection),
    );
  } catch (e) {
    toast.error("Failed to apply the changes", {
      description: toErrorMessage(e),
    });
    return false;
  } finally {
    // rerunTab は isConnectionRunning を見て早期 return するため、再取得の前に外す
    const next = new Set(applyingConnections);
    next.delete(connection);
    applyingConnections = next;
  }
  toast.success(`Applied ${affected} row change${affected === 1 ? "" : "s"}`);
  // 表示を DB の実際の値に合わせるため再取得する (適用自体は成功済み)
  await rerunTab(tabId);
  return true;
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
  /// Writable スイッチの状態。false の間は書き込み系の文を実行できない
  get writable() {
    return writable;
  },
  /// 選択中の接続が config で readonly: true か (スイッチでは解除できない)。
  /// 接続未選択なら false。トグルのロック表示に使う
  get selectedConnectionReadonly() {
    return (
      connections.find((c) => c.name === selectedConnection)?.readonly ?? false
    );
  },
  /// Writable スイッチを切り替える。config readonly 接続では書き込みは
  /// バックエンドが拒否するが、状態自体は接続横断のセッション設定として保持する
  toggleWritable() {
    writable = !writable;
  },
  get files() {
    return files;
  },
  /// 開いているエディタタブ (全接続横断・多段表示)
  get editorTabs() {
    return editorTabs;
  },
  get activeEditorTabId() {
    return activeEditorTabId;
  },
  /// アクティブなエディタタブのファイル名 (無ければ null)
  get selectedFile() {
    return getActiveEditorTab()?.file ?? null;
  },
  /// アクティブなエディタタブの内容 (無ければ空文字)
  get editorContent() {
    return getActiveEditorTab()?.content ?? "";
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
  /// アクティブなエディタタブに未保存の編集があるか
  get dirty() {
    return getActiveEditorTab()?.dirty ?? false;
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
  get aiExplaining() {
    return aiExplaining;
  },
  /// AI による選択 SQL 解説の Markdown (モーダル表示中のみ非 null)
  get aiExplanation() {
    return aiExplanation;
  },
  /// SQL 補完用のテーブル名 → カラム名リストのマップ (未取得なら null)
  get schemaMap() {
    return schemaMap;
  },
  /// 危険な文の実行確認ダイアログに表示する理由 (非表示中は null)
  get dangerousConfirmReason() {
    return dangerousConfirm?.reason ?? null;
  },
  /// 確認ダイアログで「実行する」を選んだ
  confirmDangerous: () => resolveDangerousConfirm(true),
  /// 確認ダイアログで「キャンセル」を選んだ
  cancelDangerous: () => resolveDangerousConfirm(false),
  loadConnections,
  loadAiInfo,
  generateSql,
  loadSchemaMap,
  reloadConnections,
  selectConnection,
  changeActiveSchema,
  selectFile,
  activateEditorTab,
  closeEditorTab,
  createFile,
  deleteFile,
  renameFile,
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
  explainSql,
  closeAiExplanation,
  cancelQuery,
  rerunTab,
  submitCellEdits,
  selectResultTab,
  closeResultTab,
  toggleResultTabPin,
};
