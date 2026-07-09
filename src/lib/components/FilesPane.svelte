<script lang="ts">
  import appStore from "$lib/stores/app.svelte";

  let creating = $state(false);
  let newFileName = $state("");
  let deleteCandidate = $state<string | null>(null);

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
      title="新規クエリファイル"
      data-annotate="button-create-file"
      disabled={!appStore.selectedConnection}
      onclick={() => {
        creating = true;
      }}
    >
      +
    </button>
  </div>
  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if !appStore.selectedConnection}
      <p class="px-3 py-2 text-xs text-zinc-500">接続先を選択してください</p>
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
            placeholder="ファイル名"
            data-annotate="input-new-file-name"
            autofocus
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
          + でクエリファイルを作成できます
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
              <span class="text-zinc-500" title="未保存">*</span>
            {/if}
          </button>
          <button
            class="hidden shrink-0 rounded px-1 text-xs group-hover:block {deleteCandidate ===
            fileName
              ? 'bg-red-800 text-red-200'
              : 'text-zinc-500 hover:text-red-400'}"
            title={deleteCandidate === fileName
              ? "もう一度クリックで削除"
              : "削除"}
            data-annotate="button-delete-file-{fileName}"
            onclick={() => confirmDelete(fileName)}
            onblur={() => {
              deleteCandidate = null;
            }}
          >
            {deleteCandidate === fileName ? "削除?" : "×"}
          </button>
        </div>
      {/each}
    {/if}
  </div>
</div>
