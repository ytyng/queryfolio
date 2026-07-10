<script lang="ts">
  import { toast } from "svelte-sonner";
  import appStore from "$lib/stores/app.svelte";

  interface Props {
    engine: string | null;
  }

  let { engine }: Props = $props();

  const isSqlite = $derived(
    ["sqlite", "sqlite3"].includes((engine ?? "").toLowerCase()),
  );

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
</div>
