<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { toast } from "svelte-sonner";
  import { ensureConfigFile } from "$lib/api";
  import appStore from "$lib/stores/app.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import EditorToolbar from "$lib/components/EditorToolbar.svelte";
  import ConnectionsPane from "$lib/components/ConnectionsPane.svelte";
  import FilesPane from "$lib/components/FilesPane.svelte";
  import HistoryPane from "$lib/components/HistoryPane.svelte";
  import TablesPane from "$lib/components/TablesPane.svelte";
  import SqlEditor from "$lib/components/SqlEditor.svelte";
  import ResultsPane from "$lib/components/ResultsPane.svelte";
  import ConfigInfoModal from "$lib/components/ConfigInfoModal.svelte";

  let showSettings = $state(false);
  let editor: SqlEditor | undefined = $state();
  /// 左ペイン 2 列目のタブ (クエリファイル一覧 / クエリ履歴 / テーブル一覧)
  let leftPaneTab = $state<"files" | "history" | "tables">("files");

  const selectedConnectionInfo = $derived(
    appStore.connections.find((c) => c.name === appStore.selectedConnection) ??
      null,
  );
  const selectedEngine = $derived(selectedConnectionInfo?.engine ?? null);

  onMount(() => {
    // メニューの Reload config file からの通知を受けて再読込する
    const unlistenPromise = listen("menu-reload-config", async () => {
      if (await appStore.reloadConnections()) {
        toast.success("Config reloaded");
      } else {
        toast.error("Failed to reload the config", {
          description: appStore.errorMessage ?? undefined,
        });
      }
    });

    void (async () => {
      try {
        const createdPath = await ensureConfigFile();
        if (createdPath) {
          toast.info("Created a config file", {
            description: `Edit ${createdPath} to add your connections`,
          });
        }
      } catch (e) {
        toast.error("Failed to create the config file", {
          description: String(e),
        });
      }
      await appStore.loadConnections();
    })();

    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
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
    {#if leftPaneTab === "files"}
      <FilesPane
        onShowHistory={() => {
          leftPaneTab = "history";
        }}
        onShowTables={() => {
          leftPaneTab = "tables";
        }}
      />
    {:else if leftPaneTab === "history"}
      <HistoryPane
        onShowFiles={() => {
          leftPaneTab = "files";
        }}
        onShowTables={() => {
          leftPaneTab = "tables";
        }}
      />
    {:else}
      <TablesPane
        onShowFiles={() => {
          leftPaneTab = "files";
        }}
        onShowHistory={() => {
          leftPaneTab = "history";
        }}
      />
    {/if}

    <div class="flex min-w-0 flex-1 flex-col">
      {#if appStore.selectedConnection}
        <EditorToolbar
          engine={selectedEngine}
          readonly={selectedConnectionInfo?.readonly ?? false}
          onExplain={() =>
            appStore.explainQuery(editor?.getCurrentStatement() ?? "")}
        />
      {/if}
      <div class="min-h-0 flex-[3] border-b border-zinc-700">
        {#if appStore.selectedFile}
          <SqlEditor
            bind:this={editor}
            content={appStore.editorContent}
            engine={selectedEngine}
            schemaMap={appStore.schemaMap}
            onChange={(content) => appStore.updateEditorContent(content)}
            onRun={(sql) => appStore.runQuery(sql)}
          />
        {:else}
          <div class="flex h-full items-center justify-center">
            <p class="text-sm text-zinc-600">
              Select or create a query file
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
