<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";

  interface Props {
    /// 表示対象のセル値 (result.rows の値そのまま)
    value: unknown;
    /// カラム名 (ヘッダ表示用)
    column: string;
    /// 行インデックス (0 始まり。表示は 1 始まりに変換)
    rowIndex: number;
    /// 閉じるボタン / ESC で呼ばれるコールバック
    onclose: () => void;
  }

  let { value, column, rowIndex, onclose }: Props = $props();

  let copiedKind = $state<"raw" | "pretty" | null>(null);

  // これを超えるサイズはトークン分解が重くなるためハイライトを諦める
  const HIGHLIGHT_MAX_CHARS = 200_000;

  const isNull = $derived(value === null || value === undefined);

  // セルの生テキスト表現 (テーブル表示と同じルール)
  const rawText = $derived.by(() => {
    if (value === null || value === undefined) {
      return "NULL";
    }
    if (typeof value === "object") {
      return JSON.stringify(value);
    }
    return String(value);
  });

  // JSON として解釈できる場合はパース結果 (それ以外は undefined)。
  // 数値や true 単体などのスカラーは整形する意味が無いので
  // オブジェクト / 配列のみ JSON 扱いにする
  const parsedJson = $derived.by((): unknown => {
    if (value !== null && typeof value === "object") {
      return value;
    }
    if (typeof value !== "string") {
      return undefined;
    }
    const trimmed = value.trim();
    if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) {
      return undefined;
    }
    try {
      return JSON.parse(trimmed);
    } catch {
      return undefined;
    }
  });

  const isJson = $derived(parsedJson !== undefined);

  const prettyText = $derived(
    isJson ? JSON.stringify(parsedJson, null, 2) : null,
  );

  type TokenType = "key" | "string" | "number" | "keyword" | "plain";

  interface Token {
    text: string;
    type: TokenType;
  }

  // JSON.stringify(_, null, 2) の出力を前提にした簡易トークナイザ。
  // 文字列 (キー / 値)・数値・true/false/null と、それ以外 (括弧や
  // カンマ等の構造文字) に分類する
  const JSON_TOKEN_RE =
    /("(?:\\.|[^"\\])*")(\s*:)|("(?:\\.|[^"\\])*")|\b(true|false|null)\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g;

  const tokenizeJson = (text: string): Token[] => {
    const tokens: Token[] = [];
    let last = 0;
    let match: RegExpExecArray | null;
    JSON_TOKEN_RE.lastIndex = 0;
    while ((match = JSON_TOKEN_RE.exec(text)) !== null) {
      if (match.index > last) {
        tokens.push({ text: text.slice(last, match.index), type: "plain" });
      }
      if (match[1] !== undefined) {
        // キー文字列 + 後続のコロン
        tokens.push({ text: match[1], type: "key" });
        tokens.push({ text: match[2], type: "plain" });
      } else if (match[3] !== undefined) {
        tokens.push({ text: match[3], type: "string" });
      } else if (match[4] !== undefined) {
        tokens.push({ text: match[4], type: "keyword" });
      } else {
        tokens.push({ text: match[5], type: "number" });
      }
      last = JSON_TOKEN_RE.lastIndex;
    }
    if (last < text.length) {
      tokens.push({ text: text.slice(last), type: "plain" });
    }
    return tokens;
  };

  // ハイライト用トークン列 (巨大な値は null にして無地で表示する)
  const tokens = $derived.by(() => {
    if (prettyText === null || prettyText.length > HIGHLIGHT_MAX_CHARS) {
      return null;
    }
    return tokenizeJson(prettyText);
  });

  const TOKEN_CLASSES: Record<TokenType, string> = {
    key: "text-sky-300",
    string: "text-emerald-300",
    number: "text-amber-300",
    keyword: "text-purple-300",
    plain: "text-zinc-500",
  };

  const copy = async (kind: "raw" | "pretty") => {
    const text = kind === "pretty" ? (prettyText ?? rawText) : rawText;
    // navigator.clipboard は Tauri 2 で OS のパーミッションプロンプトが
    // 出ることがあるため、公式プラグイン経由で書き込む
    await writeText(text);
    copiedKind = kind;
    setTimeout(() => {
      copiedKind = null;
    }, 1500);
  };

  // ESC でインスペクタを閉じる
  const onWindowKeydown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      onclose();
    }
  };
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div
  class="flex w-96 shrink-0 flex-col border-l border-zinc-700 bg-zinc-900"
  data-annotate="panel-cell-inspector"
>
  <!-- ヘッダ: カラム名・行番号・閉じるボタン -->
  <div
    class="flex shrink-0 items-center gap-2 border-b border-zinc-700 px-3 py-1.5 text-xs text-zinc-400"
  >
    <span
      class="min-w-0 truncate font-mono font-semibold text-zinc-300"
      title={column}
      data-annotate="text-cell-inspector-column"
    >
      {column}
    </span>
    <span class="shrink-0 text-zinc-500">Row {rowIndex + 1}</span>
    {#if isJson}
      <span
        class="shrink-0 rounded bg-zinc-700 px-1 py-px text-[10px] font-semibold text-zinc-300"
      >
        JSON
      </span>
    {/if}
    <button
      class="ml-auto shrink-0 rounded px-1 text-zinc-500 hover:bg-zinc-700 hover:text-zinc-200"
      title="Close the cell inspector (Esc)"
      data-annotate="button-cell-inspector-close"
      onclick={onclose}
    >
      ×
    </button>
  </div>

  <!-- コピー操作 -->
  <div
    class="flex shrink-0 items-center gap-1 border-b border-zinc-700 px-3 py-1 text-xs text-zinc-400"
  >
    <button
      class="rounded border border-zinc-700 px-1.5 py-0.5 hover:bg-zinc-700 hover:text-zinc-200"
      title="Copy the raw cell value"
      data-annotate="button-cell-inspector-copy-raw"
      onclick={() => copy("raw")}
    >
      {copiedKind === "raw" ? "Copied!" : "Copy raw"}
    </button>
    {#if isJson}
      <button
        class="rounded border border-zinc-700 px-1.5 py-0.5 hover:bg-zinc-700 hover:text-zinc-200"
        title="Copy the formatted JSON"
        data-annotate="button-cell-inspector-copy-pretty"
        onclick={() => copy("pretty")}
      >
        {copiedKind === "pretty" ? "Copied!" : "Copy pretty"}
      </button>
    {/if}
  </div>

  <!-- 本文: JSON は整形 + ハイライト、それ以外は折り返しテキスト -->
  <div
    class="min-h-0 flex-1 overflow-auto px-3 py-2"
    data-annotate="text-cell-inspector-value"
  >
    {#if isNull}
      <p class="font-mono text-xs text-zinc-600 italic">NULL</p>
    {:else if prettyText !== null}
      {#if tokens !== null}
        <!-- pre 内は空白がそのまま表示されるため 1 行で書く -->
        <!-- prettier-ignore -->
        <pre class="font-mono text-xs break-all whitespace-pre-wrap">{#each tokens as token, i (i)}<span class={TOKEN_CLASSES[token.type]}>{token.text}</span>{/each}</pre>
      {:else}
        <pre
          class="font-mono text-xs break-all whitespace-pre-wrap text-zinc-200">{prettyText}</pre>
      {/if}
    {:else}
      <pre
        class="font-mono text-xs break-all whitespace-pre-wrap text-zinc-200">{rawText}</pre>
    {/if}
  </div>
</div>
