<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import appStore from "$lib/stores/app.svelte";
  import { toCsv, toJson, toTsv } from "$lib/export";

  let copiedFormat = $state<string | null>(null);

  const copyAs = async (format: "csv" | "tsv" | "json") => {
    const result = appStore.queryResult;
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
</script>

<div class="flex h-full min-h-0 flex-col bg-zinc-900">
  <div
    class="flex shrink-0 items-center gap-3 border-b border-zinc-700 px-3 py-1.5 text-xs text-zinc-400"
  >
    <span class="font-semibold tracking-wide">RESULTS</span>
    {#if appStore.running}
      <span class="text-blue-400">Running...</span>
    {:else if appStore.queryResult}
      {@const result = appStore.queryResult}
      {#if result.affected_rows !== null}
        <span data-annotate="text-affected-rows">
          {result.affected_rows} rows affected
        </span>
      {:else}
        <span data-annotate="text-row-count">{result.row_count} rows</span>
        {#if result.truncated}
          <span class="text-amber-400" title="Truncated at the row limit">
            (truncated)
          </span>
        {/if}
      {/if}
      <span>{result.elapsed_ms} ms</span>
      {#if result.columns.length > 0}
        <span class="ml-auto flex items-center gap-1">
          {#each ["csv", "tsv", "json"] as const as format (format)}
            <button
              class="rounded border border-zinc-700 px-1.5 py-0.5 uppercase hover:bg-zinc-700 hover:text-zinc-200"
              data-annotate="button-copy-{format}"
              onclick={() => copyAs(format)}
            >
              {copiedFormat === format ? "copied!" : format}
            </button>
          {/each}
        </span>
      {/if}
    {/if}
  </div>

  <div class="min-h-0 flex-1 overflow-auto">
    {#if appStore.errorMessage}
      <pre
        class="whitespace-pre-wrap px-3 py-2 font-mono text-xs text-red-400"
        data-annotate="text-error-message">{appStore.errorMessage}</pre>
    {:else if appStore.queryResult && appStore.queryResult.columns.length > 0}
      {@const result = appStore.queryResult}
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
    {:else if appStore.queryResult}
      <p class="px-3 py-2 text-xs text-zinc-500">No result set</p>
    {:else}
      <p class="px-3 py-2 text-xs text-zinc-500">
        Press Cmd+Enter (Ctrl+Enter) to run the SQL statement under the cursor
      </p>
    {/if}
  </div>
</div>
