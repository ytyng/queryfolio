<script lang="ts">
  import appStore from "$lib/stores/app.svelte";

  let creating = $state(false);
  let newFileName = $state("");
  let deleteCandidate = $state<string | null>(null);

  // デフォルトのファイル名: query-YYYYMMDD-HHMM (.sql はバックエンドが付与)。
  // 同一分内の連続作成で衝突しないよう、既存ファイルと重複する場合は
  // -2, -3 ... を付けて一意化する。
  const defaultFileName = () => {
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, "0");
    const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`;
    const time = `${pad(now.getHours())}${pad(now.getMinutes())}`;
    const base = `query-${date}-${time}`;
    if (!appStore.files.includes(`${base}.sql`)) {
      return base;
    }
    let n = 2;
    while (appStore.files.includes(`${base}-${n}.sql`)) {
      n++;
    }
    return `${base}-${n}`;
  };

  const submitNewFile = async () => {
    const name = newFileName.trim();
    if (!name) {
      creating = false;
      return;
    }
    await appStore.createFile(name);
    newFileName = "";
    creating = false;
  };

  const confirmDelete = async (fileName: string) => {
    if (deleteCandidate !== fileName) {
      deleteCandidate = fileName;
      return;
    }
    deleteCandidate = null;
    await appStore.deleteFile(fileName);
  };
</script>

<div class="flex h-full w-56 shrink-0 flex-col border-r border-zinc-700 bg-zinc-900">
  <div class="flex items-center justify-between border-b border-zinc-700 px-3 py-2">
    <span class="text-xs font-semibold tracking-wide text-zinc-400">FILES</span>
    <button
      class="rounded px-1.5 py-0.5 text-xs text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200 disabled:opacity-40"
      title="New query file"
      data-annotate="button-create-file"
      disabled={!appStore.selectedConnection}
      onclick={() => {
        newFileName = defaultFileName();
        creating = true;
      }}
    >
      +
    </button>
  </div>
  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if !appStore.selectedConnection}
      <p class="px-3 py-2 text-xs text-zinc-500">Select a connection</p>
    {:else}
      {#if creating}
        <form
          class="flex items-center gap-1 px-2 py-1.5"
          onsubmit={(e) => {
            e.preventDefault();
            void submitNewFile();
          }}
        >
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="w-full rounded border border-zinc-600 bg-zinc-800 px-1.5 py-0.5 text-xs text-zinc-200 outline-none focus:border-blue-400"
            placeholder="File name"
            data-annotate="input-new-file-name"
            autofocus
            onfocus={(e) => e.currentTarget.select()}
            bind:value={newFileName}
            onblur={() => {
              creating = false;
              newFileName = "";
            }}
          />
        </form>
      {/if}
      {#if appStore.files.length === 0 && !creating}
        <p class="px-3 py-2 text-xs text-zinc-500">
          Click + to create a query file
        </p>
      {/if}
      {#each appStore.files as fileName (fileName)}
        <div
          class="group flex items-center gap-1 pr-1 hover:bg-zinc-800 {appStore.selectedFile ===
          fileName
            ? 'bg-zinc-800 border-l-2 border-blue-400'
            : 'border-l-2 border-transparent'}"
        >
          <button
            class="min-w-0 flex-1 truncate px-3 py-1.5 text-left text-sm text-zinc-200"
            data-annotate="button-file-{fileName}"
            onclick={() => appStore.selectFile(fileName)}
          >
            {fileName}
            {#if appStore.selectedFile === fileName && appStore.dirty}
              <span class="text-zinc-500" title="Unsaved">*</span>
            {/if}
          </button>
          <button
            class="hidden shrink-0 rounded px-1 text-xs group-hover:block {deleteCandidate ===
            fileName
              ? 'bg-red-800 text-red-200'
              : 'text-zinc-500 hover:text-red-400'}"
            title={deleteCandidate === fileName
              ? "Click again to delete"
              : "Delete"}
            data-annotate="button-delete-file-{fileName}"
            onclick={() => confirmDelete(fileName)}
            onblur={() => {
              deleteCandidate = null;
            }}
          >
            {deleteCandidate === fileName ? "Delete?" : "×"}
          </button>
        </div>
      {/each}
    {/if}
  </div>
</div>
