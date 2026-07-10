<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { EditorState, Compartment, StateEffect } from "@codemirror/state";
  import {
    EditorView,
    Decoration,
    ViewPlugin,
    ViewUpdate,
    keymap,
    lineNumbers,
    drawSelection,
    highlightActiveLineGutter,
  } from "@codemirror/view";
  import type { DecorationSet } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { syntaxTree } from "@codemirror/language";
  import { autocompletion, completionKeymap } from "@codemirror/autocomplete";
  import { sql, MySQL, PostgreSQL, SQLite } from "@codemirror/lang-sql";
  import type { SQLNamespace } from "@codemirror/lang-sql";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { formatSql } from "$lib/sqlFormat";

  interface Props {
    content: string;
    engine: string | null;
    /// スキーマベース補完用のテーブル名 → カラム名リスト (未取得なら null)
    schemaMap: Record<string, string[]> | null;
    onChange: (content: string) => void;
    onRun: (sql: string) => void;
  }

  let { content, engine, schemaMap, onChange, onRun }: Props = $props();

  let editorElement: HTMLDivElement;
  let view: EditorView | null = null;
  const languageCompartment = new Compartment();

  /// スキーマ補完に使うテーブル数の上限。超えた場合はカラムを渡さず
  /// テーブル名のみにする (補完候補の構築コストとメモリの抑制)
  const MAX_SCHEMA_TABLES_WITH_COLUMNS = 2000;

  const dialectFor = (engineName: string | null) => {
    switch ((engineName ?? "").toLowerCase()) {
      case "mysql":
      case "mariadb":
        return MySQL;
      case "postgres":
      case "postgresql":
        return PostgreSQL;
      case "sqlite":
      case "sqlite3":
        return SQLite;
      default:
        return PostgreSQL;
    }
  };

  // スキーママップを lang-sql の schema オプションへ変換する。
  // Record<string, string[]> はそのまま SQLNamespace として渡せる
  // (PostgreSQL の "schema.table" のようなドット付きキーは lang-sql 側で
  // 階層に分解される)。巨大スキーマではカラムを省いてテーブル名のみにする。
  const schemaNamespace = (
    map: Record<string, string[]> | null,
  ): SQLNamespace | undefined => {
    if (!map) {
      return undefined;
    }
    const tables = Object.keys(map);
    if (tables.length <= MAX_SCHEMA_TABLES_WITH_COLUMNS) {
      return map;
    }
    return Object.fromEntries(tables.map((table) => [table, []]));
  };

  // languageCompartment に入れる SQL 言語拡張 (方言 + スキーマ補完)
  const languageExtension = (
    engineName: string | null,
    map: Record<string, string[]> | null,
  ) => sql({ dialect: dialectFor(engineName), schema: schemaNamespace(map) });

  // カーソル位置を含む Statement ノードの範囲を返す。
  // カーソルが文と文の間にある場合は直前の文を返す (DataGrip と同様の挙動)。
  const statementRangeAt = (
    state: EditorState,
    pos: number,
  ): { from: number; to: number } | null => {
    const top = syntaxTree(state).topNode;
    let previous: { from: number; to: number } | null = null;
    for (
      let node = top.firstChild;
      node !== null;
      node = node.nextSibling
    ) {
      if (node.name !== "Statement") {
        continue;
      }
      if (pos >= node.from && pos <= node.to) {
        return { from: node.from, to: node.to };
      }
      if (node.to < pos) {
        previous = { from: node.from, to: node.to };
      }
    }
    return previous;
  };

  // ある行の trim 済みテキストの範囲を返す (空行なら null)
  const trimmedLineRange = (
    state: EditorState,
    pos: number,
  ): { from: number; to: number } | null => {
    const line = state.doc.lineAt(pos);
    const text = state.sliceDoc(line.from, line.to);
    const leading = text.length - text.trimStart().length;
    const trailing = text.length - text.trimEnd().length;
    if (leading + trailing >= text.length) {
      return null;
    }
    return { from: line.from + leading, to: line.to - trailing };
  };

  // 実行対象の範囲を返す。**ハイライトと実行は必ずこの同じ範囲を使うこと**
  // (別ロジックにすると表示と実行される SQL が乖離するバグになる)。
  //
  // 注意: lezer のエラー回復は "\d" を ⚠(バックスラッシュ) + Statement("d")
  // とパースするため、Statement 範囲をそのまま使うとバックスラッシュが
  // 欠落した SQL が実行される。以下のルールで補正する:
  // - カーソル行が \ 始まりで、カーソルを含む Statement が無い、または
  //   その Statement の開始行も \ 始まり (メタ行由来の誤パース) の場合は、
  //   カーソル行の trim 範囲を実行対象にする
  // - それ以外は Statement 範囲を使うが、範囲の直前 (行頭から Statement
  //   開始まで) が空白とバックスラッシュのみなら範囲を行頭側へ拡張して
  //   バックスラッシュを含める (直前の文フォールバックがメタ行を返す場合)
  const executionTargetRange = (
    state: EditorState,
    pos: number,
  ): { from: number; to: number } | null => {
    const range = statementRangeAt(state, pos);
    const cursorLine = trimmedLineRange(state, pos);
    const cursorLineText = cursorLine
      ? state.sliceDoc(cursorLine.from, cursorLine.to)
      : "";

    if (cursorLineText.startsWith("\\")) {
      const inStatement =
        range !== null && pos >= range.from && pos <= range.to;
      if (!inStatement) {
        return cursorLine;
      }
      const firstLine = trimmedLineRange(state, range.from);
      const firstLineText = firstLine
        ? state.sliceDoc(firstLine.from, firstLine.to)
        : "";
      if (firstLineText.startsWith("\\")) {
        return cursorLine;
      }
    }

    if (!range) {
      return null;
    }

    // Statement 直前のバックスラッシュ (エラートークン) を範囲に含める
    const startLine = state.doc.lineAt(range.from);
    const beforeStatement = state.sliceDoc(startLine.from, range.from);
    const match = beforeStatement.match(/^(\s*)\\+$/);
    if (match) {
      return { from: startLine.from + match[1].length, to: range.to };
    }
    return range;
  };

  const currentStatementText = (state: EditorState): string => {
    const range = executionTargetRange(state, state.selection.main.head);
    if (!range) {
      return "";
    }
    return state.sliceDoc(range.from, range.to);
  };

  // カーソル位置の文の行に枠線・背景を付けるハイライトプラグイン
  const statementHighlight = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;

      constructor(view: EditorView) {
        this.decorations = this.build(view);
      }

      update(update: ViewUpdate) {
        if (update.docChanged || update.selectionSet) {
          this.decorations = this.build(update.view);
        }
      }

      build(view: EditorView): DecorationSet {
        // 実行対象と同じ範囲をハイライトする (表示と実行の乖離を防ぐ)
        const range = executionTargetRange(
          view.state,
          view.state.selection.main.head,
        );
        if (!range) {
          return Decoration.none;
        }
        const decorations = [];
        const firstLine = view.state.doc.lineAt(range.from);
        const lastLine = view.state.doc.lineAt(range.to);
        for (let n = firstLine.number; n <= lastLine.number; n++) {
          const line = view.state.doc.line(n);
          let className = "cm-active-statement";
          if (n === firstLine.number) {
            className += " cm-active-statement-first";
          }
          if (n === lastLine.number) {
            className += " cm-active-statement-last";
          }
          decorations.push(
            Decoration.line({ class: className }).range(line.from),
          );
        }
        return Decoration.set(decorations);
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );

  // ツールバーの Run ボタンから呼ぶための公開メソッド
  export function runCurrentStatement() {
    const statement = getCurrentStatement();
    if (statement.trim()) {
      onRun(statement);
    }
  }

  // カーソル位置の文 (実行対象と同じ範囲) を返す公開メソッド。
  // ツールバーの Explain ボタンが EXPLAIN の対象文を取るのに使う
  export function getCurrentStatement(): string {
    return view ? currentStatementText(view.state) : "";
  }

  // カーソル位置の文を整形して、その範囲を整形結果で置換する公開メソッド。
  // 整形できない (未対応構文・壊す恐れ) 場合は formatSql が原文を返すため
  // 変化がなく、何もしない。
  export function formatCurrentStatement() {
    if (!view) {
      return;
    }
    const state = view.state;
    const range = executionTargetRange(state, state.selection.main.head);
    if (!range) {
      return;
    }
    const original = state.sliceDoc(range.from, range.to);
    const formatted = formatSql(original);
    if (formatted === original) {
      return;
    }
    view.dispatch({
      changes: { from: range.from, to: range.to, insert: formatted },
      selection: { anchor: range.from + formatted.length },
    });
    view.focus();
  }

  const runKeymap = keymap.of([
    {
      key: "Mod-Enter",
      run: (editorView) => {
        const statement = currentStatementText(editorView.state);
        if (statement.trim()) {
          onRun(statement);
        }
        return true;
      },
    },
  ]);

  const editorTheme = EditorView.theme({
    "&": {
      height: "100%",
      fontSize: "13px",
    },
    ".cm-scroller": {
      fontFamily:
        "ui-monospace, SFMono-Regular, Menlo, Monaco, monospace",
    },
    ".cm-active-statement": {
      backgroundColor: "rgba(96, 165, 250, 0.08)",
      borderLeft: "2px solid rgba(96, 165, 250, 0.6)",
      borderRight: "1px solid rgba(96, 165, 250, 0.25)",
    },
    ".cm-active-statement-first": {
      borderTop: "1px solid rgba(96, 165, 250, 0.25)",
    },
    ".cm-active-statement-last": {
      borderBottom: "1px solid rgba(96, 165, 250, 0.25)",
    },
  });

  onMount(() => {
    view = new EditorView({
      state: EditorState.create({
        doc: content,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          drawSelection(),
          history(),
          autocompletion(),
          // Mod-Enter を defaultKeymap より先に評価させる
          runKeymap,
          keymap.of([
            ...defaultKeymap,
            ...historyKeymap,
            ...completionKeymap,
            indentWithTab,
          ]),
          languageCompartment.of(languageExtension(engine, schemaMap)),
          oneDark,
          editorTheme,
          statementHighlight,
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              onChange(update.state.doc.toString());
            }
          }),
        ],
      }),
      parent: editorElement,
    });
  });

  onDestroy(() => {
    view?.destroy();
    view = null;
  });

  // ファイル切り替え等で外部から content が変わった時にエディタへ反映する
  $effect(() => {
    const nextContent = content;
    if (!view) {
      return;
    }
    const currentDoc = view.state.doc.toString();
    if (nextContent !== currentDoc) {
      view.dispatch({
        changes: { from: 0, to: currentDoc.length, insert: nextContent },
      });
    }
  });

  // エンジン・スキーママップ変更時に SQL 方言と補完スキーマを差し替える
  $effect(() => {
    // view のガードより先に評価し、両方をリアクティブ依存として追跡させる
    const extension = languageExtension(engine, schemaMap);
    if (!view) {
      return;
    }
    view.dispatch({
      effects: languageCompartment.reconfigure(extension),
    });
  });
</script>

<div
  bind:this={editorElement}
  class="sql-editor-host h-full min-h-0 overflow-hidden"
  data-annotate="editor-sql"
></div>

<style>
  /* oneDark テーマの背景指定は CodeMirror のテーマ優先順位で
     EditorView.theme の上書きに勝つことがあるため、CSS で確実に上書きする */
  .sql-editor-host :global(.cm-editor),
  .sql-editor-host :global(.cm-gutters) {
    background-color: #111111 !important;
  }
</style>
