<script lang="ts">
  import { onMount } from "svelte";
  import appStore from "$lib/stores/app.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import ConnectionsPane from "$lib/components/ConnectionsPane.svelte";
  import FilesPane from "$lib/components/FilesPane.svelte";
  import SqlEditor from "$lib/components/SqlEditor.svelte";
  import ResultsPane from "$lib/components/ResultsPane.svelte";
  import ConfigInfoModal from "$lib/components/ConfigInfoModal.svelte";

  let showSettings = $state(false);
  let editor: SqlEditor | undefined = $state();

  const selectedEngine = $derived(
    appStore.connections.find((c) => c.name === appStore.selectedConnection)
      ?.engine ?? null,
  );

  onMount(() => {
    void appStore.loadConnections();
  });
</script>

<div class="flex h-screen flex-col bg-zinc-950 text-zinc-200">
  <Toolbar
    onRunCurrent={() => editor?.runCurrentStatement()}
    onOpenSettings={() => {
      showSettings = true;
    }}
  />

  <div class="flex min-h-0 flex-1">
    <ConnectionsPane />
    <FilesPane />

    <div class="flex min-w-0 flex-1 flex-col">
      <div class="min-h-0 flex-[3] border-b border-zinc-700">
        {#if appStore.selectedFile}
          <SqlEditor
            bind:this={editor}
            content={appStore.editorContent}
            engine={selectedEngine}
            onChange={(content) => appStore.updateEditorContent(content)}
            onRun={(sql) => appStore.runQuery(sql)}
          />
        {:else}
          <div class="flex h-full items-center justify-center">
            <p class="text-sm text-zinc-600">
              クエリファイルを選択または作成してください
            </p>
          </div>
        {/if}
      </div>
      <div class="min-h-0 flex-[2]">
        <ResultsPane />
      </div>
    </div>
  </div>
</div>

{#if showSettings}
  <ConfigInfoModal
    onClose={() => {
      showSettings = false;
    }}
  />
{/if}
