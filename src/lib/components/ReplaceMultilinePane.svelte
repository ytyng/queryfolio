<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import {
    generateLineReplace,
    countLineReplaceResults,
    PLACEHOLDER,
  } from "$lib/lineReplace";

  interface Props {
    /// 開いた時点でエディタで選択されていた行 (Lines 欄の初期値)
    initialLines: string;
    /// 生成結果をエディタの選択範囲へ差し込む
    onReplace: (result: string) => void;
    /// ペインを閉じる
    onClose: () => void;
  }

  let { initialLines, onReplace, onClose }: Props = $props();

  // テンプレートは localStorage に保存し、開き直しても保持する。
  // 既定はタスクの主目的である KILL 文の例にしておく
  const TEMPLATE_KEY = "queryfolio.replaceMultiline.template";
  const loadTemplate = (): string => {
    try {
      return localStorage.getItem(TEMPLATE_KEY) ?? "KILL %%%;";
    } catch {
      return "KILL %%%;";
    }
  };

  let template = $state(loadTemplate());
  // 開いた時点の選択行で初期化する。親は #key で再マウントするため、
  // prop の初期値をそのまま使えばよい (以後は編集可能なローカル状態)
  // svelte-ignore state_referenced_locally
  let linesText = $state(initialLines);
  let copied = $state(false);

  $effect(() => {
    try {
      localStorage.setItem(TEMPLATE_KEY, template);
    } catch {
      // localStorage が使えなくても動作は継続する
    }
  });

  const output = $derived(generateLineReplace(linesText, template));
  const resultCount = $derived(countLineReplaceResults(linesText, template));

  const copy = async () => {
    if (output === "") {
      return;
    }
    await writeText(output);
    copied = true;
    setTimeout(() => {
      copied = false;
    }, 1500);
  };

  const replace = () => {
    // 空出力 (全行スキップ) では何もしない。ボタンの disabled と挙動を揃え、
    // Cmd+Enter で選択が空文字置換 (= 削除) されるのを防ぐ
    if (output === "") {
      return;
    }
    onReplace(output);
  };

  // Cmd+Enter で差し込み (エディタと同じ実行系ショートカットに寄せる)
  const onKeydown = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      replace();
    }
  };
</script>

<div
  class="flex h-full min-h-0 flex-col bg-zinc-900 text-xs text-zinc-300"
  data-annotate="pane-replace-multiline"
>
  <!-- ヘッダ -->
  <div
    class="flex shrink-0 items-center gap-2 border-b border-zinc-700 px-3 py-1.5"
  >
    <span class="font-semibold tracking-wide text-zinc-400">
      REPLACE MULTILINE
    </span>
    <button
      type="button"
      class="ml-auto rounded px-1 text-zinc-500 hover:bg-zinc-700 hover:text-zinc-200"
      title="Close"
      aria-label="Close"
      data-annotate="button-replace-multiline-close"
      onclick={onClose}
    >
      <i class="bi bi-x-lg" aria-hidden="true"></i>
    </button>
  </div>

  <div class="flex min-h-0 flex-1 flex-col gap-2 overflow-auto p-3">
    <!-- テンプレート -->
    <label class="flex flex-col gap-1">
      <span class="text-zinc-500">
        Template (use <code class="text-sky-400">{PLACEHOLDER}</code> as the placeholder)
      </span>
      <input
        type="text"
        class="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 font-mono text-zinc-200 outline-none focus:border-blue-400"
        data-annotate="input-replace-multiline-template"
        placeholder="KILL %%%;"
        bind:value={template}
        onkeydown={onKeydown}
      />
    </label>

    <!-- 入力行 (選択行で初期化。編集可) -->
    <label class="flex min-h-0 flex-1 flex-col gap-1">
      <span class="text-zinc-500">
        Lines (empty lines and lines starting with # or // are skipped)
      </span>
      <textarea
        class="min-h-24 flex-1 resize-none rounded border border-zinc-600 bg-zinc-800 px-2 py-1 font-mono text-zinc-200 outline-none focus:border-blue-400"
        data-annotate="textarea-replace-multiline-lines"
        spellcheck="false"
        bind:value={linesText}
        onkeydown={onKeydown}
      ></textarea>
    </label>

    <!-- 生成結果プレビュー -->
    <div class="flex min-h-0 flex-1 flex-col gap-1">
      <span class="text-zinc-500">Result ({resultCount} lines)</span>
      <textarea
        class="min-h-24 flex-1 resize-none rounded border border-zinc-700 bg-zinc-950 px-2 py-1 font-mono text-emerald-300 outline-none"
        data-annotate="text-replace-multiline-output"
        readonly
        spellcheck="false"
        value={output}
      ></textarea>
    </div>
  </div>

  <!-- フッタ操作 -->
  <div
    class="flex shrink-0 items-center gap-2 border-t border-zinc-700 px-3 py-2"
  >
    <button
      type="button"
      class="rounded border border-blue-500/50 bg-blue-500/15 px-2 py-0.5 text-blue-300 hover:bg-blue-500/25 disabled:cursor-not-allowed disabled:opacity-50"
      title="Replace the selected text in the editor with the result (Cmd+Enter)"
      data-annotate="button-replace-multiline-apply"
      disabled={output === ""}
      onclick={replace}
    >
      <i class="bi bi-arrow-left-right" aria-hidden="true"></i> Replace selection
    </button>
    <button
      type="button"
      class="rounded border border-zinc-600 bg-zinc-800 px-2 py-0.5 text-zinc-300 hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50"
      title="Copy the result to the clipboard"
      data-annotate="button-replace-multiline-copy"
      disabled={output === ""}
      onclick={copy}
    >
      {copied ? "Copied!" : "Copy"}
    </button>
  </div>
</div>
