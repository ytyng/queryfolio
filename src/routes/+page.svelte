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
  import EditorTabs from "$lib/components/EditorTabs.svelte";
  import ReplaceMultilinePane from "$lib/components/ReplaceMultilinePane.svelte";
  import ResultsPane from "$lib/components/ResultsPane.svelte";
  import ConfigInfoModal from "$lib/components/ConfigInfoModal.svelte";
  import AiAnalysisModal from "$lib/components/AiAnalysisModal.svelte";
  import DangerousConfirmModal from "$lib/components/DangerousConfirmModal.svelte";
  import PaneDivider from "$lib/components/PaneDivider.svelte";

  let showSettings = $state(false);
  let editor: SqlEditor | undefined = $state();

  // Replace Multiline: エディタの複数行選択状態と、右側の置換ペインの表示。
  // ペインを開いた時点の選択範囲を snapshot し、差し込み時に範囲がズレて
  // いないか照合してから置換する (ファイル切替・編集での誤挿入を防ぐ)
  let hasMultilineSelection = $state(false);
  let showReplacePane = $state(false);
  let replaceInitialLines = $state("");
  let replaceSnapshot: { from: number; to: number; text: string } | null = null;
  // ペインを開き直すたびに増やし、#key で再マウントして Lines を作り直す
  let replaceOpenToken = $state(0);

  function openReplacePane() {
    const snap = editor?.getMainSelection();
    if (!snap) {
      return;
    }
    replaceSnapshot = snap;
    replaceInitialLines = snap.text;
    replaceOpenToken += 1;
    showReplacePane = true;
  }

  function applyReplace(result: string) {
    const snap = replaceSnapshot;
    const ok =
      snap != null &&
      (editor?.replaceRangeIfMatches(
        snap.from,
        snap.to,
        snap.text,
        result,
      ) ??
        false);
    if (ok) {
      showReplacePane = false;
    } else {
      // 選択範囲がズレた (ファイル切替や編集) 場合は破壊せずに知らせる
      toast.error("The editor selection changed — nothing was replaced.", {
        description: "Use Copy to grab the result instead.",
      });
    }
  }

  // タブ切替・クローズで選択追跡状態と置換ペインをリセットする。
  // 依存はアクティブタブ ID にする: 同名ファイルを別接続で開いている場合、
  // selectedFile (ファイル名) は変わらないままタブだけ切り替わり得るため、
  // selectedFile 依存だと stale なスナップショットが残り新タブへ誤適用される
  $effect(() => {
    void appStore.activeEditorTabId;
    hasMultilineSelection = false;
    showReplacePane = false;
    replaceSnapshot = null;
  });
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
          showReplaceMultiline={hasMultilineSelection &&
            appStore.selectedFile !== null}
          onReplaceMultiline={openReplacePane}
        />
      {/if}
      <div
        class="flex min-h-0 basis-0 flex-col border-b border-zinc-700"
        style="flex-grow: {editorFrac}"
        bind:this={editorPaneEl}
      >
        {#if appStore.editorTabs.length > 0}
          <EditorTabs />
        {/if}
        <div class="min-h-0 flex-1">
          {#if appStore.selectedFile}
            <!-- エディタと Replace Multiline ペインを横並びにする -->
            <div class="flex h-full min-h-0">
              <div class="min-w-0 flex-1">
                <!-- タブ切替でエディタを作り直し、タブ間で undo 履歴・
                     カーソルが混ざらないようにする -->
                {#key appStore.activeEditorTabId}
                  <SqlEditor
                    bind:this={editor}
                    content={appStore.editorContent}
                    engine={selectedEngine}
                    schemaMap={appStore.schemaMap}
                    onChange={(content) => appStore.updateEditorContent(content)}
                    onRun={(sql) => appStore.runQuery(sql)}
                    onSelectionChange={(info) => {
                      hasMultilineSelection = info.hasMultilineSelection;
                    }}
                  />
                {/key}
              </div>
              {#if showReplacePane}
                <div class="w-96 shrink-0 border-l border-zinc-700">
                  {#key replaceOpenToken}
                    <ReplaceMultilinePane
                      initialLines={replaceInitialLines}
                      onReplace={applyReplace}
                      onClose={() => {
                        showReplacePane = false;
                      }}
                    />
                  {/key}
                </div>
              {/if}
            </div>
          {:else}
            <div class="flex h-full items-center justify-center">
              <p class="text-sm text-zinc-600">
                Select or create a query file
              </p>
            </div>
          {/if}
        </div>
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
