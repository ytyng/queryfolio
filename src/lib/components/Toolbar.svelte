<script lang="ts">
  import appStore from "$lib/stores/app.svelte";

  interface Props {
    onRunCurrent: () => void;
    onOpenSettings: () => void;
  }

  let { onRunCurrent, onOpenSettings }: Props = $props();

  // 実行中は Run ボタンを Cancel ボタンに切り替え、
  // 実行中のタブ (接続単位で 1 つ) のクエリをキャンセルする
  const cancelRunningQuery = () => {
    const running = appStore.resultTabs.find((t) => t.running);
    if (running) {
      void appStore.cancelQuery(running.id);
    }
  };
</script>

<div
  class="flex shrink-0 items-center gap-2 border-b border-zinc-700 bg-zinc-900 px-3 py-1.5"
>
  <span class="text-sm font-semibold text-zinc-200">QueryFolio</span>
  {#if appStore.selectedConnection}
    <span class="text-xs text-zinc-500">
      {appStore.selectedConnection}
      {#if appStore.selectedFile}
        / {appStore.selectedFile}
      {/if}
    </span>
  {/if}

  <span class="ml-auto flex items-center gap-2">
    {#if appStore.running}
      <button
        class="rounded bg-red-800 px-3 py-1 text-xs text-white hover:bg-red-700"
        title="Cancel the running query"
        data-annotate="button-cancel-query-toolbar"
        onclick={cancelRunningQuery}
      >
        ■ Cancel
      </button>
    {:else}
      <button
        class="rounded bg-green-700 px-3 py-1 text-xs text-white hover:bg-green-600 disabled:opacity-40"
        title="Run the statement under the cursor (Cmd+Enter)"
        data-annotate="button-run-query"
        disabled={!appStore.selectedConnection}
        onclick={onRunCurrent}
      >
        ▶ Run
      </button>
    {/if}
    <button
      class="rounded border border-zinc-600 px-2 py-1 text-xs text-zinc-300 hover:bg-zinc-800"
      title="Settings"
      data-annotate="button-open-settings"
      onclick={onOpenSettings}
    >
      ⚙
    </button>
  </span>
</div>
