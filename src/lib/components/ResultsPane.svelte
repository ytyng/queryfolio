<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import appStore from "$lib/stores/app.svelte";
  import type { ResultTab } from "$lib/stores/app.svelte";
  import { toCsv, toJson, toTsv } from "$lib/export";

  let copiedFormat = $state<string | null>(null);

  const activeTab = $derived(appStore.activeResultTab);

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
</script>

<div class="flex h-full min-h-0 flex-col bg-zinc-900">
  <!-- タブバー -->
  <div
    class="flex shrink-0 items-center gap-3 border-b border-zinc-700 px-3 py-1 text-xs text-zinc-400"
  >
    <span class="font-semibold tracking-wide">RESULTS</span>
    {#if appStore.resultTabs.length > 0}
      <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
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
              data-annotate="button-result-tab-pin-{tab.id}"
              onclick={() => appStore.toggleResultTabPin(tab.id)}
            >
              <svg
                class="h-3 w-3"
                viewBox="0 0 24 24"
                fill={tab.pinned ? "currentColor" : "none"}
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M12 17v5" />
                <path
                  d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"
                />
              </svg>
            </button>
            <button
              class="rounded px-0.5 text-zinc-500 hover:bg-zinc-700 hover:text-zinc-200 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-zinc-500"
              title={tab.running
                ? "Cannot close while the query is running"
                : "Close this tab"}
              data-annotate="button-result-tab-close-{tab.id}"
              disabled={tab.running}
              onclick={() => appStore.closeResultTab(tab.id)}
            >
              ×
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
        <button
          class="rounded border border-zinc-700 px-1.5 py-0.5 hover:bg-zinc-700 hover:text-zinc-200 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent"
          title="Run this tab's SQL again on {activeTab.connection}"
          data-annotate="button-rerun-tab"
          disabled={activeTab.running}
          onclick={() => appStore.rerunTab(activeTab.id)}
        >
          ↻ Re-run
        </button>
        {#if activeTab.result && activeTab.result.columns.length > 0}
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

  <div class="min-h-0 flex-1 overflow-auto">
    {#if appStore.errorMessage}
      <pre
        class="whitespace-pre-wrap px-3 py-2 font-mono text-xs text-red-400"
        data-annotate="text-error-message">{appStore.errorMessage}</pre>
    {:else if activeTab?.running}
      <p class="px-3 py-2 text-xs text-blue-400">Running...</p>
    {:else if activeTab?.error}
      <pre
        class="whitespace-pre-wrap px-3 py-2 font-mono text-xs text-red-400"
        data-annotate="text-error-message">{activeTab.error}</pre>
    {:else if activeTab?.result && activeTab.result.columns.length > 0}
      {@const result = activeTab.result}
      <table class="min-w-full border-collapse font-mono text-xs">
        <thead class="sticky top-0 bg-zinc-800">
          <tr>
            <th
              class="border-b border-r border-zinc-700 px-2 py-1 text-right font-normal text-zinc-500"
            >
              #
            </th>
            {#each result.columns as column, i (i)}
              <th
                class="border-b border-r border-zinc-700 px-2 py-1 text-left font-semibold text-zinc-300"
              >
                {column}
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each result.rows as row, rowIndex (rowIndex)}
            <tr class="hover:bg-zinc-800/60">
              <td
                class="border-b border-r border-zinc-800 px-2 py-0.5 text-right text-zinc-600"
              >
                {rowIndex + 1}
              </td>
              {#each row as value, colIndex (colIndex)}
                <td
                  class="max-w-96 truncate border-b border-r border-zinc-800 px-2 py-0.5 {value ===
                  null
                    ? 'italic text-zinc-600'
                    : 'text-zinc-200'}"
                  title={cellText(value)}
                >
                  {cellText(value)}
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
</div>
