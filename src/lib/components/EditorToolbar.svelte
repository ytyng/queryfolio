<script lang="ts">
  import { toast } from "svelte-sonner";
  import appStore from "$lib/stores/app.svelte";

  interface Props {
    engine: string | null;
    readonly: boolean;
  }

  let { engine, readonly }: Props = $props();

  const isSqlite = $derived(
    ["sqlite", "sqlite3"].includes((engine ?? "").toLowerCase()),
  );

  /// AI 生成のインライン入力欄の表示状態と入力内容
  let showAiInput = $state(false);
  let aiInstruction = $state("");
  let aiInputEl: HTMLInputElement | undefined = $state();

  const aiConfigured = $derived(appStore.aiInfo?.configured ?? false);

  /// AI ボタンの title (未設定・エラー時は設定方法を案内する)
  const aiButtonTitle = $derived(
    aiConfigured
      ? `Generate SQL with AI (${appStore.aiInfo?.model})`
      : appStore.aiError
        ? `AI is unavailable: ${appStore.aiError}`
        : "AI is not configured. Add an 'ai:' section (provider: openai, " +
          "api_key: ...) to config.yml or the connection YAML.",
  );

  // 入力欄を開いたらフォーカスする
  $effect(() => {
    if (showAiInput) {
      aiInputEl?.focus();
    }
  });

  const submitAiInstruction = async (e: SubmitEvent) => {
    e.preventDefault();
    if (appStore.aiGenerating) {
      return;
    }
    if (await appStore.generateSql(aiInstruction)) {
      aiInstruction = "";
      showAiInput = false;
    }
  };

  const onAiInputKeydown = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !appStore.aiGenerating) {
      showAiInput = false;
    }
  };

  const onSchemaChange = async (e: Event) => {
    const select = e.currentTarget as HTMLSelectElement;
    const schema = select.value;
    const previous = appStore.activeSchema;
    if (await appStore.changeActiveSchema(schema)) {
      if (schema !== previous) {
        toast.success(`Switched to ${schema}`);
      }
    } else {
      toast.error("Failed to switch the database", {
        description: appStore.errorMessage ?? undefined,
      });
      // 失敗したら表示を元に戻す
      select.value = previous ?? "";
    }
  };
</script>

<div
  class="flex shrink-0 items-center gap-2 border-b border-zinc-700 bg-zinc-900 px-3 py-1"
>
  {#if engine}
    <span
      class="rounded bg-zinc-800 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-zinc-400"
      data-annotate="text-editor-engine"
    >
      {engine}
    </span>
  {/if}

  {#if readonly}
    <span
      class="rounded bg-yellow-500/15 px-1.5 py-0.5 text-[10px] tracking-wide text-yellow-400"
      title="Write statements are rejected (readonly: true in config)"
      data-annotate="badge-editor-readonly"
    >
      read-only
    </span>
  {/if}

  <span class="text-xs text-zinc-500">Database:</span>
  {#if isSqlite || appStore.schemas.length <= 1}
    <span class="font-mono text-xs text-zinc-300" data-annotate="text-active-schema">
      {appStore.activeSchema ?? "(default)"}
    </span>
  {:else}
    <select
      class="max-w-64 rounded border border-zinc-600 bg-zinc-800 px-1.5 py-0.5 font-mono text-xs text-zinc-200 outline-none focus:border-blue-400"
      data-annotate="select-active-schema"
      value={appStore.activeSchema ?? ""}
      onchange={onSchemaChange}
    >
      {#if appStore.activeSchema && !appStore.schemas.includes(appStore.activeSchema)}
        <option value={appStore.activeSchema}>{appStore.activeSchema}</option>
      {/if}
      {#each appStore.schemas as schema (schema)}
        <option value={schema}>{schema}</option>
      {/each}
    </select>
  {/if}

  <div class="ml-auto flex min-w-0 items-center gap-2">
    {#if showAiInput}
      <form
        class="flex min-w-0 items-center gap-1"
        onsubmit={submitAiInstruction}
      >
        <input
          bind:this={aiInputEl}
          bind:value={aiInstruction}
          class="w-72 max-w-full rounded border border-zinc-600 bg-zinc-800 px-2 py-0.5 text-xs text-zinc-200 outline-none placeholder:text-zinc-500 focus:border-blue-400"
          data-annotate="input-ai-instruction"
          placeholder="Describe the query in natural language..."
          disabled={appStore.aiGenerating}
          onkeydown={onAiInputKeydown}
        />
        <button
          type="submit"
          class="flex items-center gap-1 rounded border border-blue-500/50 bg-blue-500/15 px-2 py-0.5 text-xs text-blue-300 hover:bg-blue-500/25 disabled:cursor-not-allowed disabled:opacity-50"
          data-annotate="button-ai-generate"
          disabled={appStore.aiGenerating || !aiInstruction.trim()}
        >
          {#if appStore.aiGenerating}
            <!-- 生成中スピナー -->
            <span
              class="inline-block size-3 animate-spin rounded-full border-2 border-blue-300 border-t-transparent"
              data-annotate="spinner-ai-generating"
            ></span>
            Generating...
          {:else}
            Generate
          {/if}
        </button>
        <button
          type="button"
          class="rounded px-1.5 py-0.5 text-xs text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-50"
          data-annotate="button-ai-close"
          title="Close (Esc)"
          disabled={appStore.aiGenerating}
          onclick={() => {
            showAiInput = false;
          }}
        >
          ✕
        </button>
      </form>
    {:else}
      <button
        type="button"
        class="rounded border border-zinc-600 bg-zinc-800 px-2 py-0.5 text-xs text-zinc-300 hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50"
        data-annotate="button-ai-toggle"
        title={aiButtonTitle}
        disabled={!aiConfigured}
        onclick={() => {
          showAiInput = true;
        }}
      >
        ✨ AI
      </button>
    {/if}
  </div>
</div>
