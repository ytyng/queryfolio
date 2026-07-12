<script lang="ts">
  import appStore from "$lib/stores/app.svelte";
  import type { ConnectionInfo } from "$lib/api";

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

  const isSqlite = (engine: string): boolean =>
    engine.toLowerCase() === "sqlite" || engine.toLowerCase() === "sqlite3";

  /// ツールチップに並べる詳細行を接続情報から組み立てる。
  /// 値が無い項目は省く。SSH トンネル情報は機密を含まない host/port/user のみ。
  const detailRows = (c: ConnectionInfo): { label: string; value: string }[] => {
    const rows: { label: string; value: string }[] = [];
    rows.push({ label: "Engine", value: engineLabel(c.engine) });
    if (isSqlite(c.engine)) {
      if (c.schema) rows.push({ label: "File", value: c.schema });
    } else {
      if (c.host) {
        rows.push({
          label: "Host",
          value: c.port != null ? `${c.host}:${c.port}` : c.host,
        });
      } else if (c.port != null) {
        rows.push({ label: "Port", value: String(c.port) });
      }
      if (c.user) rows.push({ label: "User", value: c.user });
      if (c.schema) rows.push({ label: "Database", value: c.schema });
    }
    if (c.ssh_tunnel) {
      const t = c.ssh_tunnel;
      rows.push({ label: "SSH", value: `${t.user}@${t.host}:${t.port}` });
    }
    if (c.description) rows.push({ label: "Description", value: c.description });
    if (c.readonly) rows.push({ label: "Access", value: "read-only" });
    return rows;
  };

  /// ホバー中の接続とツールチップ表示位置 (viewport 座標)。
  let hovered = $state<ConnectionInfo | null>(null);
  let tipX = $state(0);
  let tipY = $state(0);

  const showTip = (c: ConnectionInfo, e: MouseEvent) => {
    hovered = c;
    positionTip(c, e);
  };

  const positionTip = (c: ConnectionInfo, e: MouseEvent) => {
    // カーソルの右下に少しずらして出す。画面右端で溢れる場合は左側に、
    // 画面下端で溢れる場合はカーソルの上側に反転させる (下部の項目でも
    // ツールチップ全体が収まるように)。
    const margin = 16;
    const estWidth = 320;
    // ヘッダ + 詳細行数からツールチップの高さを概算する
    const estHeight = 40 + detailRows(c).length * 18;

    let x = e.clientX + margin;
    if (x + estWidth > window.innerWidth) {
      x = Math.max(margin, e.clientX - margin - estWidth);
    }
    tipX = x;

    let y = e.clientY + margin;
    if (y + estHeight > window.innerHeight) {
      y = Math.max(margin, e.clientY - margin - estHeight);
    }
    tipY = y;
  };

  const hideTip = () => {
    hovered = null;
  };
</script>

<div class="flex h-full w-full flex-col border-r border-zinc-700 bg-zinc-900">
  <div class="flex items-center justify-between border-b border-zinc-700 px-3 py-2">
    <span class="text-xs font-semibold tracking-wide text-zinc-400">CONNECTIONS</span>
    <button
      class="rounded px-1.5 py-0.5 text-xs text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200"
      title="Reload connections"
      aria-label="Reload connections"
      data-annotate="button-reload-connections"
      onclick={() => appStore.reloadConnections()}
    >
      <i class="bi bi-arrow-clockwise" aria-hidden="true"></i>
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
          onmouseenter={(e) => showTip(connection, e)}
          onmousemove={(e) => hovered && positionTip(connection, e)}
          onmouseleave={hideTip}
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

{#if hovered}
  <div
    class="pointer-events-none fixed z-50 max-w-sm rounded border border-zinc-600 bg-zinc-800 px-3 py-2 shadow-lg"
    style="left: {tipX}px; top: {tipY}px;"
    data-annotate="tooltip-connection-{hovered.name}"
    role="tooltip"
  >
    <div class="mb-1 text-sm font-semibold text-zinc-100 break-all">{hovered.name}</div>
    <dl class="grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 text-xs">
      {#each detailRows(hovered) as row}
        <dt class="text-zinc-400">{row.label}</dt>
        <dd class="text-zinc-200 break-all whitespace-pre-wrap">{row.value}</dd>
      {/each}
    </dl>
  </div>
{/if}
