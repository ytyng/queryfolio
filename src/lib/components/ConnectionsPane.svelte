<script lang="ts">
  import appStore from "$lib/stores/app.svelte";

  const engineLabel = (engine: string): string => {
    switch (engine.toLowerCase()) {
      case "mysql":
      case "mariadb":
        return "MySQL";
      case "postgres":
      case "postgresql":
        return "PostgreSQL";
      case "sqlite":
      case "sqlite3":
        return "SQLite";
      default:
        return engine;
    }
  };
</script>

<div class="flex h-full w-56 shrink-0 flex-col border-r border-zinc-700 bg-zinc-900">
  <div class="flex items-center justify-between border-b border-zinc-700 px-3 py-2">
    <span class="text-xs font-semibold tracking-wide text-zinc-400">CONNECTIONS</span>
    <button
      class="rounded px-1.5 py-0.5 text-xs text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200"
      title="Reload connections"
      data-annotate="button-reload-connections"
      onclick={() => appStore.reloadConnections()}
    >
      ⟳
    </button>
  </div>
  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if appStore.loadingConnections}
      <p class="px-3 py-2 text-xs text-zinc-500">Loading...</p>
    {:else if appStore.connections.length === 0}
      <p class="px-3 py-2 text-xs text-zinc-500">
        No connections. Review your config file.
      </p>
    {:else}
      {#each appStore.connections as connection (connection.name)}
        <button
          class="flex w-full flex-col gap-0.5 px-3 py-2 text-left hover:bg-zinc-800 {appStore.selectedConnection ===
          connection.name
            ? 'bg-zinc-800 border-l-2 border-blue-400'
            : 'border-l-2 border-transparent'}"
          data-annotate="button-connection-{connection.name}"
          onclick={() => appStore.selectConnection(connection.name)}
        >
          <span class="truncate text-sm text-zinc-200">{connection.name}</span>
          <span class="flex items-center gap-1 text-xs text-zinc-500">
            {engineLabel(connection.engine)}
            {#if connection.has_ssh_tunnel}
              <span
                class="rounded bg-zinc-700 px-1 text-[10px] text-zinc-300"
                title="Via SSH tunnel">ssh</span
              >
            {/if}
            {#if connection.readonly}
              <span
                class="rounded bg-yellow-500/15 px-1 text-[10px] text-yellow-400"
                title="Write statements are rejected (readonly: true in config)"
                data-annotate="badge-readonly-{connection.name}">read-only</span
              >
            {/if}
          </span>
          {#if connection.description}
            <span class="truncate text-xs text-zinc-500">{connection.description}</span>
          {/if}
        </button>
      {/each}
    {/if}
  </div>
</div>
