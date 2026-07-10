<script lang="ts">
  import appStore from "$lib/stores/app.svelte";

  interface Props {
    onRunCurrent: () => void;
    onOpenSettings: () => void;
  }

  let { onRunCurrent, onOpenSettings }: Props = $props();
</script>

<div
  class="flex shrink-0 items-center gap-2 border-b border-zinc-700 bg-zinc-900 px-3 py-1.5"
>
  <span class="text-sm font-semibold text-zinc-200">Queryfolio</span>
  {#if appStore.selectedConnection}
    <span class="text-xs text-zinc-500">
      {appStore.selectedConnection}
      {#if appStore.selectedFile}
        / {appStore.selectedFile}
      {/if}
    </span>
  {/if}

  <span class="ml-auto flex items-center gap-2">
    <button
      class="rounded bg-green-700 px-3 py-1 text-xs text-white hover:bg-green-600 disabled:opacity-40"
      title="Run the statement under the cursor (Cmd+Enter)"
      data-annotate="button-run-query"
      disabled={!appStore.selectedConnection || appStore.running}
      onclick={onRunCurrent}
    >
      {appStore.running ? "Running..." : "▶ Run"}
    </button>
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
