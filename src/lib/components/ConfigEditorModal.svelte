<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { toast } from "svelte-sonner";
  import { EditorState } from "@codemirror/state";
  import {
    EditorView,
    keymap,
    lineNumbers,
    drawSelection,
    highlightActiveLineGutter,
  } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
  import { tags as t } from "@lezer/highlight";
  import { yaml } from "@codemirror/lang-yaml";
  import { oneDark } from "@codemirror/theme-one-dark";
  import {
    readConfigFile,
    readSqlServersSourceYaml,
    writeConfigFile,
  } from "$lib/api";
  import appStore from "$lib/stores/app.svelte";

  interface Props {
    /// "config" = config.yml を編集・保存する。
    /// "source" = sql_servers のソース宣言 command が返す YAML を読み取り専用で表示する。
    mode: "config" | "source";
    onClose: () => void;
    /// 未保存の変更の有無を親に知らせる (別のモードへ切り替える時の巻き添え破棄を防ぐ)
    onDirtyChange?: (dirty: boolean) => void;
  }

  let { mode, onClose, onDirtyChange }: Props = $props();

  const readOnly = $derived(mode === "source");
  const title = $derived(
    mode === "config" ? "Edit config.yml" : "sql_servers config yaml (Read only)",
  );

  let editorElement = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let saving = $state(false);
  let dirty = $state(false);
  /// 未保存の変更がある状態で閉じようとした時に、破棄の確認を出す
  let confirmDiscard = $state(false);

  const editorTheme = EditorView.theme({
    "&": { height: "100%", fontSize: "13px" },
    ".cm-content": { color: "#f3f5f9" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#f3f5f9" },
    ".cm-scroller": {
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, monospace",
    },
  });

  // SqlEditor と同じ、oneDark より明るい配色
  const brightHighlightStyle = HighlightStyle.define([
    { tag: [t.keyword, t.operatorKeyword, t.modifier], color: "#eac6ff" },
    { tag: [t.string, t.special(t.string)], color: "#d8f5b0" },
    { tag: [t.number, t.bool, t.null], color: "#ffd7a3" },
    { tag: [t.name, t.propertyName, t.variableName], color: "#b3ddff" },
    { tag: [t.comment], color: "#c0c7da", fontStyle: "italic" },
    { tag: [t.operator, t.punctuation, t.separator], color: "#e2e7f0" },
    { tag: [t.typeName, t.className], color: "#ffeab0" },
  ]);

  const createEditor = (doc: string) => {
    if (!editorElement) {
      return;
    }
    view = new EditorView({
      state: EditorState.create({
        doc,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          drawSelection(),
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          yaml(),
          oneDark,
          syntaxHighlighting(brightHighlightStyle),
          editorTheme,
          EditorState.readOnly.of(readOnly),
          EditorView.editable.of(!readOnly),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              dirty = true;
              onDirtyChange?.(true);
              saveError = null;
            }
          }),
        ],
      }),
      parent: editorElement,
    });
    view.focus();
  };

  const load = async () => {
    try {
      const text =
        mode === "config" ? await readConfigFile() : await readSqlServersSourceYaml();
      createEditor(text);
    } catch (e) {
      loadError = String(e);
    } finally {
      loading = false;
    }
  };

  // onMount から Promise を返すとクリーンアップ関数と誤認されるため、投げっぱなしにする
  onMount(() => {
    void load();
  });

  onDestroy(() => {
    view?.destroy();
    view = null;
  });

  const save = async () => {
    if (!view || saving) {
      return;
    }
    saving = true;
    saveError = null;
    try {
      const path = await writeConfigFile(view.state.doc.toString());
      dirty = false;
      onDirtyChange?.(false);
      // 保存しただけでは実行中の接続に反映されないため、続けて再読込する
      if (await appStore.reloadConnections()) {
        toast.success(`Saved ${path}`);
        onClose();
        return;
      }
      // 保存自体は成功しているので、失敗したのは再読込であることを明示する
      saveError = `Saved ${path}, but reloading the config failed: ${
        appStore.errorMessage ?? "unknown error"
      }`;
    } catch (e) {
      saveError = String(e);
    } finally {
      saving = false;
    }
  };

  const copyAll = async () => {
    if (!view) {
      return;
    }
    await writeText(view.state.doc.toString());
    toast.success("Copied to the clipboard");
  };

  /// 未保存の変更を巻き添えで捨てないよう、dirty なら確認を挟む
  const requestClose = () => {
    if (dirty) {
      confirmDiscard = true;
      return;
    }
    onClose();
  };

  const onWindowKeydown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      // 破棄確認を出している間の Escape は「編集に戻る」(誤って捨てない)
      if (confirmDiscard) {
        confirmDiscard = false;
        return;
      }
      requestClose();
      return;
    }
    // Cmd+S / Ctrl+S で保存
    if (!readOnly && (e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      void save();
    }
  };
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div
  class="fixed inset-0 z-10 flex items-center justify-center bg-black/60"
  role="presentation"
  data-annotate="backdrop-config-editor-modal"
  onclick={(e) => {
    if (e.target === e.currentTarget) {
      requestClose();
    }
  }}
>
  <div
    class="flex h-[80vh] w-[860px] max-w-[92vw] flex-col gap-3 rounded-lg border border-zinc-700 bg-zinc-900 p-4 shadow-xl"
  >
    <h2 class="text-sm font-semibold text-zinc-200" data-annotate="text-config-editor-title">
      {title}
    </h2>

    {#if readOnly}
      <p class="text-xs text-zinc-400">
        This YAML comes from the sql_servers command source declaration and cannot be saved
        here. Copy it and edit it where it is stored.
      </p>
    {/if}

    {#if loadError}
      <pre
        class="whitespace-pre-wrap font-mono text-xs text-red-400"
        data-annotate="text-config-editor-load-error">{loadError}</pre>
    {:else if loading}
      <p class="text-xs text-zinc-500">Loading...</p>
    {/if}

    <div
      bind:this={editorElement}
      class="config-editor-host min-h-0 flex-1 overflow-hidden rounded border border-zinc-700"
      class:hidden={loadError !== null}
      data-annotate="editor-config-yaml"
    ></div>

    {#if saveError}
      <pre
        class="max-h-24 overflow-auto whitespace-pre-wrap font-mono text-xs text-red-400"
        data-annotate="text-config-editor-save-error">{saveError}</pre>
    {/if}

    {#if confirmDiscard}
      <div class="flex items-center justify-end gap-2">
        <span class="mr-auto text-xs text-amber-400">Discard unsaved changes?</span>
        <button
          class="rounded border border-zinc-600 px-3 py-1 text-xs text-zinc-300 hover:bg-zinc-800"
          data-annotate="button-config-editor-keep-editing"
          onclick={() => (confirmDiscard = false)}
        >
          Keep editing
        </button>
        <button
          class="rounded bg-red-600 px-3 py-1 text-xs text-white hover:bg-red-500"
          data-annotate="button-config-editor-discard"
          onclick={onClose}
        >
          Discard
        </button>
      </div>
    {:else}
      <div class="flex items-center justify-end gap-2">
        {#if dirty}
          <span class="mr-auto text-xs text-zinc-500">Unsaved changes</span>
        {/if}
        <button
          class="rounded border border-zinc-600 px-3 py-1 text-xs text-zinc-300 hover:bg-zinc-800"
          data-annotate="button-config-editor-copy"
          onclick={copyAll}
        >
          Copy
        </button>
        <button
          class="rounded border border-zinc-600 px-3 py-1 text-xs text-zinc-300 hover:bg-zinc-800"
          data-annotate="button-config-editor-close"
          onclick={requestClose}
        >
          Close
        </button>
        {#if !readOnly}
          <button
            class="rounded bg-blue-600 px-3 py-1 text-xs text-white hover:bg-blue-500 disabled:opacity-50"
            data-annotate="button-config-editor-save"
            disabled={saving || loadError !== null}
            onclick={save}
          >
            {saving ? "Saving..." : "Save"}
          </button>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  /* oneDark の背景指定が EditorView.theme に勝つため、CSS で確実に上書きする */
  .config-editor-host :global(.cm-editor),
  .config-editor-host :global(.cm-gutters) {
    background-color: #111111 !important;
  }
  .config-editor-host :global(.cm-editor) {
    height: 100%;
  }
</style>
