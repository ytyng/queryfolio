<script lang="ts">
  import { onMount } from "svelte";
  import { getSettings, saveSettings, type AppSettings } from "$lib/api";
  import appStore from "$lib/stores/app.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  let configYamlPath = $state("");
  let configYamlGetterCommand = $state("");
  let sqlfilesDir = $state("");
  let saveError = $state<string | null>(null);
  let loaded = $state(false);

  onMount(async () => {
    try {
      const settings = await getSettings();
      configYamlPath = settings.config_yaml_path ?? "";
      configYamlGetterCommand = settings.config_yaml_getter_command ?? "";
      sqlfilesDir = settings.sqlfiles_dir ?? "";
    } catch (e) {
      saveError = String(e);
    } finally {
      loaded = true;
    }
  });

  const submit = async () => {
    const settings: AppSettings = {
      config_yaml_path: configYamlPath.trim() || null,
      config_yaml_getter_command: configYamlGetterCommand.trim() || null,
      sqlfiles_dir: sqlfilesDir.trim() || null,
    };
    try {
      await saveSettings(settings);
    } catch (e) {
      saveError = String(e);
      return;
    }
    onClose();
    await appStore.reloadConnections();
  };
</script>

<div
  class="fixed inset-0 z-10 flex items-center justify-center bg-black/60"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) {
      onClose();
    }
  }}
>
  <div
    class="flex w-[520px] flex-col gap-3 rounded-lg border border-zinc-700 bg-zinc-900 p-4 shadow-xl"
  >
    <h2 class="text-sm font-semibold text-zinc-200">設定</h2>

    {#if loaded}
      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        接続設定 YAML の getter command (config_yaml_path より優先)
        <input
          class="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 font-mono text-xs text-zinc-200 outline-none focus:border-blue-400"
          placeholder={'op read "op://development/queryfolio/config-yaml"'}
          data-annotate="input-getter-command"
          bind:value={configYamlGetterCommand}
        />
      </label>

      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        接続設定 YAML ファイルのパス (デフォルト: ~/.config/queryfolio/config.yaml)
        <input
          class="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 font-mono text-xs text-zinc-200 outline-none focus:border-blue-400"
          placeholder="~/.config/queryfolio/config.yaml"
          data-annotate="input-config-yaml-path"
          bind:value={configYamlPath}
        />
      </label>

      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        クエリファイル保存ディレクトリ (デフォルト: ~/.config/queryfolio/sqlfiles)
        <input
          class="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 font-mono text-xs text-zinc-200 outline-none focus:border-blue-400"
          placeholder="~/.config/queryfolio/sqlfiles"
          data-annotate="input-sqlfiles-dir"
          bind:value={sqlfilesDir}
        />
      </label>

      {#if saveError}
        <p class="text-xs text-red-400">{saveError}</p>
      {/if}

      <div class="flex justify-end gap-2">
        <button
          class="rounded border border-zinc-600 px-3 py-1 text-xs text-zinc-300 hover:bg-zinc-800"
          data-annotate="button-settings-cancel"
          onclick={onClose}
        >
          キャンセル
        </button>
        <button
          class="rounded bg-blue-600 px-3 py-1 text-xs text-white hover:bg-blue-500"
          data-annotate="button-settings-save"
          onclick={submit}
        >
          保存して再読込
        </button>
      </div>
    {:else}
      <p class="text-xs text-zinc-500">読込中...</p>
    {/if}
  </div>
</div>
