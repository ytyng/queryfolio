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
  import AiAnalysisModal from "$lib/components/AiAnalysisModal.svelte";
  import DangerousConfirmModal from "$lib/components/DangerousConfirmModal.svelte";
  import PaneDivider from "$lib/components/PaneDivider.svelte";

  let showSettings = $state(false);
  let editor: SqlEditor | undefined = $state();
  /// 左ペイン 2 列目のタブ (クエリファイル一覧 / クエリ履歴 / テーブル一覧)
  let leftPaneTab = $state<"files" | "history" | "tables">("files");

  // ペインのレイアウト。区切り線のドラッグで変更し localStorage に保存する
  const LAYOUT_PREFIX = "queryfolio.layout.";
  const SIDEBAR_MIN = 140;
  const SIDEBAR_MAX = 500;
  const EDITOR_FRAC_MIN = 0.15;
  const EDITOR_FRAC_MAX = 0.85;

  function loadLayoutValue(key: string, fallback: number): number {
    try {
      const raw = localStorage.getItem(LAYOUT_PREFIX + key);
      if (raw === null) return fallback;
      const n = Number(raw);
      return Number.isFinite(n) ? n : fallback;
    } catch {
      return fallback;
    }
  }

  function saveLayoutValue(key: string, value: number) {
    try {
      localStorage.setItem(LAYOUT_PREFIX + key, String(value));
    } catch {
      // localStorage が使えなくてもレイアウト変更自体は機能させる
    }
  }

  function clamp(value: number, min: number, max: number): number {
    return Math.min(max, Math.max(min, value));
  }

  /// 接続一覧ペインの幅 (px)。デフォルトは従来の w-56 = 224px
  let connectionsWidth = $state(
    clamp(loadLayoutValue("connectionsWidth", 224), SIDEBAR_MIN, SIDEBAR_MAX),
  );
  /// 2 列目 (Files / History / Tables) ペインの幅 (px)
  let sidebarWidth = $state(
    clamp(loadLayoutValue("sidebarWidth", 224), SIDEBAR_MIN, SIDEBAR_MAX),
  );
  /// エディタが占める縦の割合。デフォルトは従来の flex 3:2 = 0.6
  let editorFrac = $state(
    clamp(loadLayoutValue("editorFrac", 0.6), EDITOR_FRAC_MIN, EDITOR_FRAC_MAX),
  );
  // editorFrac の px 換算用。列全体 (ツールバー込み) ではなく
  // 分割対象 2 ペインの実高さの合計を使うと、ドラッグがカーソルに正確に追従する
  let editorPaneEl: HTMLDivElement | undefined = $state();
  let resultsPaneEl: HTMLDivElement | undefined = $state();

  // ドラッグ開始時の基準サイズ。PaneDivider は開始位置からの累積 delta を
  // 渡すので、基準 + delta で計算するとクランプ飽和後もポインタと同期する
  let dragBaseConnections = 0;
  let dragBaseSidebar = 0;
  let dragBaseEditorFrac = 0;

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
    <div class="shrink-0" style="width: {connectionsWidth}px">
      <ConnectionsPane />
    </div>
    <PaneDivider
      direction="vertical"
      annotate="pane-divider-connections"
      onDragStart={() => {
        dragBaseConnections = connectionsWidth;
      }}
      onDrag={(delta) => {
        connectionsWidth = clamp(
          dragBaseConnections + delta,
          SIDEBAR_MIN,
          SIDEBAR_MAX,
        );
      }}
      onDragEnd={() => saveLayoutValue("connectionsWidth", connectionsWidth)}
    />
    <div class="shrink-0" style="width: {sidebarWidth}px">
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
    </div>
    <PaneDivider
      direction="vertical"
      annotate="pane-divider-sidebar"
      onDragStart={() => {
        dragBaseSidebar = sidebarWidth;
      }}
      onDrag={(delta) => {
        sidebarWidth = clamp(dragBaseSidebar + delta, SIDEBAR_MIN, SIDEBAR_MAX);
      }}
      onDragEnd={() => saveLayoutValue("sidebarWidth", sidebarWidth)}
    />

    <div class="flex min-w-0 flex-1 flex-col">
      {#if appStore.selectedConnection}
        <EditorToolbar
          engine={selectedEngine}
          readonly={selectedConnectionInfo?.readonly ?? false}
          onExplain={() =>
            appStore.explainQuery(editor?.getCurrentStatement() ?? "")}
          onExplainSql={() =>
            appStore.explainSql(editor?.getCurrentStatement() ?? "")}
          onFormat={() => editor?.formatCurrentStatement()}
        />
      {/if}
      <div
        class="min-h-0 basis-0 border-b border-zinc-700"
        style="flex-grow: {editorFrac}"
        bind:this={editorPaneEl}
      >
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
      <PaneDivider
        direction="horizontal"
        annotate="pane-divider-results"
        onDragStart={() => {
          dragBaseEditorFrac = editorFrac;
        }}
        onDrag={(delta) => {
          const height =
            (editorPaneEl?.clientHeight ?? 0) +
            (resultsPaneEl?.clientHeight ?? 0);
          if (height <= 0) return;
          editorFrac = clamp(
            dragBaseEditorFrac + delta / height,
            EDITOR_FRAC_MIN,
            EDITOR_FRAC_MAX,
          );
        }}
        onDragEnd={() => saveLayoutValue("editorFrac", editorFrac)}
      />
      <div
        class="min-h-0 basis-0"
        style="flex-grow: {1 - editorFrac}"
        bind:this={resultsPaneEl}
      >
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

<!-- AI による選択 SQL 解説のモーダル (EXPLAIN 解説モーダルを見出し違いで再利用) -->
{#if appStore.aiExplanation !== null}
  <AiAnalysisModal
    title="AI SQL Explanation"
    text={appStore.aiExplanation}
    onClose={() => appStore.closeAiExplanation()}
  />
{/if}

<!-- 危険な文 (allow_dangerous_statements 有効な接続) の実行前確認モーダル -->
{#if appStore.dangerousConfirmReason !== null}
  <DangerousConfirmModal
    reason={appStore.dangerousConfirmReason}
    onConfirm={() => appStore.confirmDangerous()}
    onCancel={() => appStore.cancelDangerous()}
  />
{/if}
