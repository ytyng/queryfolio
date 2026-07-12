<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import appStore, { isExplainSql } from "$lib/stores/app.svelte";
  import type { ResultTab } from "$lib/stores/app.svelte";
  import { toCsv, toCsvRange, toJson, toTsv, type CellRange } from "$lib/export";
  import CellInspector from "./CellInspector.svelte";
  import AiAnalysisModal from "./AiAnalysisModal.svelte";

  let copiedFormat = $state<string | null>(null);

  const activeTab = $derived(appStore.activeResultTab);

  /// Analyze with AI ボタンの表示条件: EXPLAIN 由来のタブに結果があり、
  /// AI が設定済みであること
  const canAnalyzePlan = $derived(
    activeTab !== null &&
      activeTab.result !== null &&
      isExplainSql(activeTab.sql) &&
      (appStore.aiInfo?.configured ?? false),
  );

  // インスペクタで表示中のセル。どのタブのセルかを tabId で覚えておき、
  // タブ切替・タブクローズ時に別タブのセルを表示し続けないようにする
  let selectedCell = $state<{
    tabId: number;
    rowIndex: number;
    colIndex: number;
  } | null>(null);

  // 結果テーブルの矩形選択。anchor から focus までを選択範囲とする。
  // mode: cell = 単一セル基点の矩形, row = 行まるごと, col = 列まるごと。
  // tabId で対象タブを覚え、タブ切替・再実行で範囲外になったら破棄する。
  let selection = $state<{
    tabId: number;
    mode: "cell" | "row" | "col";
    anchorRow: number;
    anchorCol: number;
    focusRow: number;
    focusCol: number;
  } | null>(null);

  // ドラッグ選択の状態 ($state 不要: レンダリングに使わない内部フラグ)
  let dragging = false;
  let dragMode: "cell" | "row" | "col" | null = null;
  // ドラッグでセルをまたいだか。単純クリックとの区別に使い、
  // ドラッグ後にセルインスペクタを誤って開閉しないようにする
  let dragMoved = false;

  // キーボード操作 (Cmd+C) を結果グリッドに限定するためのフォーカス先
  let gridEl = $state<HTMLDivElement | null>(null);

  // Cmd+C コピーにヘッダ行を含めるか。localStorage に保存する
  const COPY_HEADERS_KEY = "queryfolio.results.copyWithHeaders";
  let copyWithHeaders = $state(loadCopyWithHeaders());
  function loadCopyWithHeaders(): boolean {
    try {
      return localStorage.getItem(COPY_HEADERS_KEY) === "1";
    } catch {
      return false;
    }
  }
  $effect(() => {
    try {
      localStorage.setItem(COPY_HEADERS_KEY, copyWithHeaders ? "1" : "0");
    } catch {
      // localStorage が使えなくても動作は継続する
    }
  });

  // 選択範囲コピー時の一時的なフィードバック表示
  let selectionCopied = $state(false);

  // タブ切替 (クローズによる切替を含む) でインスペクタ・選択を閉じる
  $effect(() => {
    void appStore.activeTabId;
    selectedCell = null;
    selection = null;
  });

  // 選択範囲を結果サイズにクランプして正規化する (min/max を確定)。
  // アクティブタブ・結果が無い、または別タブの選択なら null。
  const selectedRange = $derived.by<CellRange | null>(() => {
    const tab = activeTab;
    if (!selection || !tab || selection.tabId !== tab.id) {
      return null;
    }
    const result = tab.result;
    if (!result) {
      return null;
    }
    const maxRow = result.rows.length - 1;
    const maxCol = result.columns.length - 1;
    if (maxRow < 0 || maxCol < 0) {
      return null;
    }
    const clampR = (n: number) => Math.min(maxRow, Math.max(0, n));
    const clampC = (n: number) => Math.min(maxCol, Math.max(0, n));
    let rowStart: number;
    let rowEnd: number;
    let colStart: number;
    let colEnd: number;
    if (selection.mode === "col") {
      rowStart = 0;
      rowEnd = maxRow;
    } else {
      rowStart = clampR(Math.min(selection.anchorRow, selection.focusRow));
      rowEnd = clampR(Math.max(selection.anchorRow, selection.focusRow));
    }
    if (selection.mode === "row") {
      colStart = 0;
      colEnd = maxCol;
    } else {
      colStart = clampC(Math.min(selection.anchorCol, selection.focusCol));
      colEnd = clampC(Math.max(selection.anchorCol, selection.focusCol));
    }
    return { rowStart, rowEnd, colStart, colEnd };
  });

  const isCellSelected = (rowIndex: number, colIndex: number): boolean => {
    const r = selectedRange;
    return (
      r !== null &&
      rowIndex >= r.rowStart &&
      rowIndex <= r.rowEnd &&
      colIndex >= r.colStart &&
      colIndex <= r.colEnd
    );
  };

  // 行ヘッダ (#) / 列ヘッダのハイライト条件
  const isRowHeaderSelected = (rowIndex: number): boolean => {
    const r = selectedRange;
    const cols = activeTab?.result?.columns.length ?? 0;
    return (
      r !== null &&
      r.colStart === 0 &&
      r.colEnd === cols - 1 &&
      rowIndex >= r.rowStart &&
      rowIndex <= r.rowEnd
    );
  };
  const isColHeaderSelected = (colIndex: number): boolean => {
    const r = selectedRange;
    const rows = activeTab?.result?.rows.length ?? 0;
    return (
      r !== null &&
      r.rowStart === 0 &&
      r.rowEnd === rows - 1 &&
      colIndex >= r.colStart &&
      colIndex <= r.colEnd
    );
  };

  const beginSelect = (
    mode: "cell" | "row" | "col",
    rowIndex: number,
    colIndex: number,
    e: PointerEvent,
  ) => {
    if (!activeTab?.result || e.button !== 0) {
      return;
    }
    dragging = true;
    dragMode = mode;
    dragMoved = false;
    selection = {
      tabId: activeTab.id,
      mode,
      anchorRow: rowIndex,
      anchorCol: colIndex,
      focusRow: rowIndex,
      focusCol: colIndex,
    };
    // ドラッグ後 (クリックが発火しない) でも Cmd+C が届くようにグリッドへフォーカス
    gridEl?.focus();
  };

  const extendSelect = (
    mode: "cell" | "row" | "col",
    rowIndex: number,
    colIndex: number,
    e: PointerEvent,
  ) => {
    if (!dragging) {
      return;
    }
    // pointerup をウインドウ外で取りこぼした場合の保険。ボタンが押されて
    // いない pointerenter (ただのホバー) が来たらドラッグを終了する
    if (e.buttons === 0) {
      endDrag();
      return;
    }
    if (dragMode !== mode || !selection) {
      return;
    }
    if (selection.focusRow !== rowIndex || selection.focusCol !== colIndex) {
      dragMoved = true;
    }
    selection = { ...selection, focusRow: rowIndex, focusCol: colIndex };
  };

  const endDrag = () => {
    dragging = false;
    dragMode = null;
  };

  const copySelection = async () => {
    const range = selectedRange;
    const result = activeTab?.result;
    if (!range || !result) {
      return;
    }
    const text = toCsvRange(result, range, copyWithHeaders);
    await writeText(text);
    selectionCopied = true;
    setTimeout(() => {
      selectionCopied = false;
    }, 1500);
  };

  // Cmd+C (Ctrl+C) で選択範囲を CSV コピー。結果グリッドにフォーカスが
  // ある時だけ処理し、SQL エディタ等からのコピーは横取りしない
  const handleWindowKeydown = (e: KeyboardEvent) => {
    if (!(e.metaKey || e.ctrlKey) || (e.key !== "c" && e.key !== "C")) {
      return;
    }
    if (!selectedRange || !gridEl || !gridEl.contains(document.activeElement)) {
      return;
    }
    e.preventDefault();
    void copySelection();
  };

  // インスペクタに渡す値。再実行などで結果が入れ替わり
  // 選択位置が範囲外になった場合は null (= 非表示) にする
  const inspectedCell = $derived.by(() => {
    const tab = activeTab;
    if (!selectedCell || !tab || selectedCell.tabId !== tab.id) {
      return null;
    }
    const result = tab.result;
    if (!result) {
      return null;
    }
    const row = result.rows[selectedCell.rowIndex];
    if (!row || selectedCell.colIndex >= result.columns.length) {
      return null;
    }
    return {
      value: row[selectedCell.colIndex],
      column: result.columns[selectedCell.colIndex],
      rowIndex: selectedCell.rowIndex,
    };
  });

  // セルクリックで選択してインスペクタを開く。選択中セルの再クリックで閉じる
  const selectCell = (rowIndex: number, colIndex: number) => {
    if (!activeTab) {
      return;
    }
    if (
      selectedCell &&
      selectedCell.tabId === activeTab.id &&
      selectedCell.rowIndex === rowIndex &&
      selectedCell.colIndex === colIndex
    ) {
      selectedCell = null;
      return;
    }
    selectedCell = { tabId: activeTab.id, rowIndex, colIndex };
  };

  // セルの click。直前がドラッグ選択だった場合はインスペクタ開閉を抑制する
  const onCellClick = (rowIndex: number, colIndex: number) => {
    if (dragMoved) {
      dragMoved = false;
      return;
    }
    // 選択中セルの再クリック (インスペクタを閉じる操作) では、
    // pointerdown で張られた 1 セルの矩形選択も一緒に解除して
    // ハイライトが残らないようにする
    const closingInspector = isSelectedCell(rowIndex, colIndex);
    selectCell(rowIndex, colIndex);
    if (closingInspector) {
      selection = null;
    }
  };

  const isSelectedCell = (rowIndex: number, colIndex: number): boolean =>
    selectedCell !== null &&
    selectedCell.tabId === activeTab?.id &&
    selectedCell.rowIndex === rowIndex &&
    selectedCell.colIndex === colIndex;

  // セル背景色: インスペクタで開いているセルを最優先で強調し、
  // 次に矩形選択の範囲を淡く強調する
  const cellBgClass = (rowIndex: number, colIndex: number): string => {
    if (isSelectedCell(rowIndex, colIndex)) {
      return "bg-sky-800/60";
    }
    if (isCellSelected(rowIndex, colIndex)) {
      return "bg-sky-900/40";
    }
    return "";
  };

  const copyAs = async (format: "csv" | "tsv" | "json") => {
    const result = activeTab?.result;
    if (!result) {
      return;
    }
    const text =
      format === "csv"
        ? toCsv(result)
        : format === "tsv"
          ? toTsv(result)
          : toJson(result);
    // navigator.clipboard は Tauri 2 で OS のパーミッションプロンプトが
    // 出ることがあるため、公式プラグイン経由で書き込む
    await writeText(text);
    copiedFormat = format;
    setTimeout(() => {
      copiedFormat = null;
    }, 1500);
  };

  const cellText = (value: unknown): string => {
    if (value === null || value === undefined) {
      return "NULL";
    }
    if (typeof value === "object") {
      return JSON.stringify(value);
    }
    return String(value);
  };

  // タブ見出し用に SQL を 1 行・短縮表示にする
  const tabLabel = (tab: ResultTab): string => {
    const compact = tab.sql.replace(/\s+/g, " ").trim();
    if (!compact) {
      return "Query";
    }
    return compact.length > 24 ? `${compact.slice(0, 24)}…` : compact;
  };

  const formatTime = (epochMs: number): string => {
    const d = new Date(epochMs);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  };

  const tabTooltip = (tab: ResultTab): string =>
    `${tab.connection}${tab.schema ? ` / ${tab.schema}` : ""} at ${formatTime(tab.executedAt)}\n${tab.sql.trim()}`;

  const aiConfigured = $derived(appStore.aiInfo?.configured ?? false);

  /// Fix with AI ボタンの title (未設定・エラー時は設定方法を案内する)。
  /// DB エラーメッセージには値が含まれ得るため、送信内容を明示する
  const aiFixButtonTitle = $derived(
    aiConfigured
      ? `Ask AI to fix this SQL (${appStore.aiInfo?.model}). ` +
          "Sends the failed SQL, the database error message, and " +
          "table/column names to the AI provider (never the query results)."
      : appStore.aiError
        ? `AI is unavailable: ${appStore.aiError}`
        : "AI is not configured. Add an 'ai:' section (provider: openai, " +
          "api_key: ...) to config.yml or the connection YAML.",
  );
</script>

<!-- ドラッグ選択はセルの pointerenter で追跡するため、
     pointer capture は使わず window で終了を拾う -->
<svelte:window
  onpointerup={endDrag}
  onpointercancel={endDrag}
  onkeydown={handleWindowKeydown}
/>

<div class="flex h-full min-h-0 flex-col bg-zinc-900">
  <!-- タブバー -->
  <div
    class="flex shrink-0 items-start gap-3 border-b border-zinc-700 px-3 py-1 text-xs text-zinc-400"
  >
    <span class="mt-0.5 font-semibold tracking-wide">RESULTS</span>
    {#if appStore.resultTabs.length > 0}
      <!-- 多段タブ: タブが増えたら折り返して複数段で表示する -->
      <div class="flex min-w-0 flex-1 flex-wrap items-center gap-1">
        {#each appStore.resultTabs as tab (tab.id)}
          <div
            class="flex shrink-0 items-center gap-0.5 rounded-t border-t border-r border-l px-1 py-0.5 {tab.id ===
            appStore.activeTabId
              ? 'border-zinc-600 bg-zinc-800 text-zinc-200'
              : 'border-transparent text-zinc-500 hover:bg-zinc-800/60 hover:text-zinc-300'}"
          >
            <button
              class="max-w-48 truncate font-mono"
              title={tabTooltip(tab)}
              data-annotate="button-result-tab-{tab.id}"
              onclick={() => appStore.selectResultTab(tab.id)}
            >
              {tabLabel(tab)}
            </button>
            <button
              class="rounded px-0.5 hover:bg-zinc-700 {tab.pinned
                ? 'text-amber-400'
                : 'text-zinc-500 hover:text-zinc-300'}"
              title={tab.pinned ? "Unpin this tab" : "Pin this tab"}
              aria-label={tab.pinned ? "Unpin this tab" : "Pin this tab"}
              data-annotate="button-result-tab-pin-{tab.id}"
              onclick={() => appStore.toggleResultTabPin(tab.id)}
            >
              <i
                class="bi {tab.pinned ? 'bi-pin-fill' : 'bi-pin-angle'}"
                aria-hidden="true"
              ></i>
            </button>
            <button
              class="rounded px-0.5 text-zinc-500 hover:bg-zinc-700 hover:text-zinc-200 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-zinc-500"
              title={tab.running
                ? "Cannot close while the query is running"
                : "Close this tab"}
              aria-label={tab.running
                ? "Cannot close while the query is running"
                : "Close this tab"}
              data-annotate="button-result-tab-close-{tab.id}"
              disabled={tab.running}
              onclick={() => appStore.closeResultTab(tab.id)}
            >
              <i class="bi bi-x" aria-hidden="true"></i>
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <!-- アクティブタブの実行情報 -->
  {#if activeTab}
    <div
      class="flex shrink-0 items-center gap-3 border-b border-zinc-700 px-3 py-1.5 text-xs text-zinc-400"
    >
      <span
        class="max-w-40 truncate font-mono text-zinc-300"
        title={activeTab.connection}
        data-annotate="text-result-connection"
      >
        {activeTab.connection}
      </span>
      {#if activeTab.schema}
        <span
          class="max-w-40 truncate font-mono"
          title={activeTab.schema}
          data-annotate="text-result-schema"
        >
          {activeTab.schema}
        </span>
      {/if}
      <span data-annotate="text-result-executed-at">
        {formatTime(activeTab.executedAt)}
      </span>
      {#if activeTab.running}
        <span class="text-blue-400">Running...</span>
      {:else if activeTab.cancelled}
        <span class="text-amber-400" data-annotate="text-result-cancelled">
          Cancelled
        </span>
      {:else if activeTab.result}
        {@const result = activeTab.result}
        {#if result.affected_rows !== null}
          <span data-annotate="text-affected-rows">
            {result.affected_rows} rows affected
          </span>
        {:else}
          <span data-annotate="text-row-count">{result.row_count} rows</span>
          {#if result.applied_limit !== null}
            <span
              class="text-zinc-500"
              title="LIMIT was added automatically (default_limit in config.yml)"
              data-annotate="text-applied-limit"
            >
              LIMIT {result.applied_limit} (auto)
            </span>
          {/if}
          {#if result.truncated}
            <span class="text-amber-400" title="Truncated at the row limit">
              (truncated)
            </span>
          {/if}
        {/if}
        <span>{result.elapsed_ms} ms</span>
      {/if}
      <span class="ml-auto flex items-center gap-1">
        {#if activeTab.running}
          <button
            class="rounded border border-red-800 bg-red-900/40 px-1.5 py-0.5 text-red-300 hover:bg-red-800 hover:text-red-100"
            title="Cancel the running query"
            aria-label="Cancel the running query"
            data-annotate="button-cancel-query"
            onclick={() => appStore.cancelQuery(activeTab.id)}
          >
            <i class="bi bi-x-circle" aria-hidden="true"></i> Cancel
          </button>
        {/if}
        <button
          class="rounded border border-zinc-700 px-1.5 py-0.5 hover:bg-zinc-700 hover:text-zinc-200 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent"
          title="Run this tab's SQL again on {activeTab.connection}"
          aria-label="Run this tab's SQL again"
          data-annotate="button-rerun-tab"
          disabled={appStore.isConnectionRunning(activeTab.connection)}
          onclick={() => appStore.rerunTab(activeTab.id)}
        >
          <i class="bi bi-arrow-repeat" aria-hidden="true"></i> Re-run
        </button>
        {#if canAnalyzePlan}
          <!-- EXPLAIN の実行計画を AI に解説させる (AI 設定済みのタブのみ) -->
          <button
            class="flex items-center gap-1 rounded border border-blue-500/50 bg-blue-500/15 px-1.5 py-0.5 text-blue-300 hover:bg-blue-500/25 disabled:cursor-not-allowed disabled:opacity-50"
            title="Explain the plan with AI ({appStore.aiInfo?.model})"
            data-annotate="button-analyze-plan"
            disabled={appStore.aiAnalyzing}
            onclick={() => appStore.analyzeExplainTab(activeTab.id)}
          >
            {#if appStore.aiAnalyzing}
              <!-- 解説中スピナー -->
              <span
                class="inline-block size-3 animate-spin rounded-full border-2 border-blue-300 border-t-transparent"
                data-annotate="spinner-ai-analyzing"
              ></span>
              Analyzing...
            {:else}
              <i class="bi bi-stars" aria-hidden="true"></i> Analyze with AI
            {/if}
          </button>
        {/if}
        {#if activeTab.result && activeTab.result.columns.length > 0}
          {#if selectionCopied}
            <span class="text-emerald-400" data-annotate="text-selection-copied">
              Copied
            </span>
          {/if}
          <label
            class="flex cursor-pointer items-center gap-1 select-none hover:text-zinc-200"
            title="Include column headers when copying a cell selection with Cmd+C (Ctrl+C)"
          >
            <input
              type="checkbox"
              class="cursor-pointer accent-sky-600"
              data-annotate="checkbox-copy-with-headers"
              bind:checked={copyWithHeaders}
            />
            Copy with headers
          </label>
          {#each ["csv", "tsv", "json"] as const as format (format)}
            <button
              class="rounded border border-zinc-700 px-1.5 py-0.5 uppercase hover:bg-zinc-700 hover:text-zinc-200"
              data-annotate="button-copy-{format}"
              onclick={() => copyAs(format)}
            >
              {copiedFormat === format ? "copied!" : format}
            </button>
          {/each}
        {/if}
      </span>
    </div>
  {/if}

  <div class="flex min-h-0 flex-1">
    <!-- tabindex/bind: セル選択後の Cmd+C コピーを結果グリッドに限定する
         (window の keydown で gridEl 内にフォーカスがある時だけ処理) -->
    <div
      class="min-h-0 flex-1 overflow-auto focus:outline-none"
      tabindex="-1"
      bind:this={gridEl}
    >
      {#if appStore.errorMessage}
        <pre
          class="whitespace-pre-wrap px-3 py-2 font-mono text-xs text-red-400"
          data-annotate="text-error-message">{appStore.errorMessage}</pre>
      {:else if activeTab?.running}
        <p class="px-3 py-2 text-xs text-blue-400">Running...</p>
      {:else if activeTab?.error}
        <div class="px-3 py-2">
          <div class="flex items-start gap-2">
            <pre
              class="min-w-0 flex-1 whitespace-pre-wrap font-mono text-xs text-red-400"
              data-annotate="text-error-message">{activeTab.error}</pre>
            <button
              class="flex shrink-0 items-center gap-1 rounded border border-blue-500/50 bg-blue-500/15 px-2 py-0.5 text-xs text-blue-300 hover:bg-blue-500/25 disabled:cursor-not-allowed disabled:opacity-50"
              title={aiFixButtonTitle}
              data-annotate="button-ai-fix"
              disabled={!aiConfigured || activeTab.fixing}
              onclick={() => appStore.fixSqlWithAi(activeTab.id)}
            >
              {#if activeTab.fixing}
                <!-- 修正案の生成中スピナー -->
                <span
                  class="inline-block size-3 animate-spin rounded-full border-2 border-blue-300 border-t-transparent"
                  data-annotate="spinner-ai-fixing"
                ></span>
                Fixing...
              {:else}
                <i class="bi bi-magic" aria-hidden="true"></i> Fix with AI
              {/if}
            </button>
          </div>

          <!-- AI の修正案 (元の SQL と並べて表示。Apply までは実行しない) -->
          {#if activeTab.fixSuggestion}
            <div
              class="mt-2 rounded border border-zinc-700 bg-zinc-800/40"
              data-annotate="panel-ai-fix-suggestion"
            >
              <div
                class="flex items-center gap-2 border-b border-zinc-700 px-2 py-1 text-xs text-zinc-400"
              >
                <span class="font-semibold">AI fix suggestion</span>
                <span class="ml-auto flex items-center gap-1">
                  <button
                    class="rounded border border-blue-500/50 bg-blue-500/15 px-1.5 py-0.5 text-blue-300 hover:bg-blue-500/25"
                    title="Insert the suggested SQL into the editor (does not run it)"
                    data-annotate="button-ai-fix-apply"
                    onclick={() => appStore.applyFixSuggestion(activeTab.id)}
                  >
                    Apply to editor
                  </button>
                  <button
                    class="rounded border border-zinc-700 px-1.5 py-0.5 hover:bg-zinc-700 hover:text-zinc-200"
                    title="Discard the suggestion"
                    data-annotate="button-ai-fix-dismiss"
                    onclick={() => appStore.dismissFixSuggestion(activeTab.id)}
                  >
                    Dismiss
                  </button>
                </span>
              </div>
              <div class="p-2 text-xs">
                <p class="mb-1 text-zinc-500">Original SQL:</p>
                <pre
                  class="mb-2 overflow-x-auto rounded bg-zinc-900 px-2 py-1 font-mono text-zinc-400"
                  data-annotate="text-ai-fix-original">{activeTab.sql.trim()}</pre>
                <p class="mb-1 text-zinc-500">Suggested SQL:</p>
                <pre
                  class="overflow-x-auto rounded bg-zinc-900 px-2 py-1 font-mono text-emerald-300"
                  data-annotate="text-ai-fix-suggested">{activeTab.fixSuggestion}</pre>
              </div>
            </div>
          {/if}
        </div>
      {:else if activeTab?.cancelled}
        <p
          class="px-3 py-2 text-xs text-amber-400"
          data-annotate="text-query-cancelled"
        >
          Query cancelled
        </p>
      {:else if activeTab?.result && activeTab.result.columns.length > 0}
        {@const result = activeTab.result}
        <table class="min-w-full border-collapse font-mono text-xs select-none">
          <thead class="sticky top-0 bg-zinc-800">
            <tr>
              <th
                class="border-b border-r border-zinc-700 px-2 py-1 text-right font-normal text-zinc-500"
              >
                #
              </th>
              {#each result.columns as column, colIndex (colIndex)}
                <!-- ヘッダクリックでその列を選択。ドラッグで複数列に拡張 -->
                <th
                  class="border-b border-r border-zinc-700 p-0 text-left font-semibold {isColHeaderSelected(
                    colIndex,
                  )
                    ? 'bg-sky-800/50 text-zinc-100'
                    : 'text-zinc-300'}"
                >
                  <button
                    class="block w-full cursor-pointer px-2 py-1 text-left"
                    title="Click to select this column (drag to select more)"
                    data-annotate="button-result-col-header-{colIndex}"
                    onpointerdown={(e) => beginSelect("col", 0, colIndex, e)}
                    onpointerenter={(e) => extendSelect("col", 0, colIndex, e)}
                  >
                    {column}
                  </button>
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each result.rows as row, rowIndex (rowIndex)}
              <tr class="hover:bg-zinc-800/60">
                <!-- 行番号クリックでその行を選択。ドラッグで複数行に拡張 -->
                <td
                  class="border-b border-r border-zinc-800 p-0 text-right {isRowHeaderSelected(
                    rowIndex,
                  )
                    ? 'bg-sky-800/50 text-zinc-300'
                    : 'text-zinc-600'}"
                >
                  <button
                    class="block w-full cursor-pointer px-2 py-0.5 text-right"
                    title="Click to select this row (drag to select more)"
                    data-annotate="button-result-row-header-{rowIndex}"
                    onpointerdown={(e) => beginSelect("row", rowIndex, 0, e)}
                    onpointerenter={(e) => extendSelect("row", rowIndex, 0, e)}
                  >
                    {rowIndex + 1}
                  </button>
                </td>
                {#each row as value, colIndex (colIndex)}
                  <!-- クリックでセルインスペクタを開く。ドラッグで矩形選択。
                       truncate のためボタンをセル全面に敷く -->
                  <td
                    class="max-w-96 border-b border-r border-zinc-800 p-0 {cellBgClass(
                      rowIndex,
                      colIndex,
                    )}"
                  >
                    <button
                      class="block w-full cursor-pointer truncate px-2 py-0.5 text-left {value ===
                      null
                        ? 'italic text-zinc-600'
                        : 'text-zinc-200'}"
                      title={cellText(value)}
                      data-annotate="button-result-cell-{rowIndex}-{colIndex}"
                      onpointerdown={(e) =>
                        beginSelect("cell", rowIndex, colIndex, e)}
                      onpointerenter={(e) =>
                        extendSelect("cell", rowIndex, colIndex, e)}
                      onclick={() => onCellClick(rowIndex, colIndex)}
                    >
                      {cellText(value)}
                    </button>
                  </td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      {:else if activeTab?.result}
        <p class="px-3 py-2 text-xs text-zinc-500">No result set</p>
      {:else}
        <p class="px-3 py-2 text-xs text-zinc-500">
          Press Cmd+Enter (Ctrl+Enter) to run the SQL statement under the cursor
        </p>
      {/if}
    </div>

    <!-- セルインスペクタ (セル選択中のみ表示) -->
    {#if inspectedCell}
      <CellInspector
        value={inspectedCell.value}
        column={inspectedCell.column}
        rowIndex={inspectedCell.rowIndex}
        onclose={() => (selectedCell = null)}
      />
    {/if}
  </div>
</div>

<!-- AI による実行計画解説のモーダル -->
{#if appStore.aiAnalysis !== null}
  <AiAnalysisModal
    text={appStore.aiAnalysis}
    onClose={() => appStore.closeAiAnalysis()}
  />
{/if}
