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
  import { oneDark } from "@codemirror/theme-one-dark";

  interface Props {
    content: string;
    engine: string | null;
    onChange: (content: string) => void;
    onRun: (sql: string) => void;
  }

  let { content, engine, onChange, onRun }: Props = $props();

  let editorElement: HTMLDivElement;
  let view: EditorView | null = null;
  const languageCompartment = new Compartment();

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

  const currentStatementText = (state: EditorState): string => {
    const head = state.selection.main.head;
    const range = statementRangeAt(state, head);
    // psql 風メタコマンド (\dt など) は、カーソル行が \ 始まりの場合に
    // その行全体を実行対象にする。ただし複数行 SQL の途中の行
    // (文字列リテラル内等) を誤ってメタコマンド扱いしないよう、
    // カーソルを含む Statement が行の外にまたがる場合は SQL として扱う。
    const line = state.doc.lineAt(head);
    const lineText = state.sliceDoc(line.from, line.to).trim();
    if (lineText.startsWith("\\")) {
      // 注意: lezer のエラー回復は "\\d" を ⚠(バックスラッシュ) +
      // Statement("d") とパースし、後続の SQL と融合することもあるため、
      // 「Statement 内かどうか」ではメタコマンド行を判別できない。
      // カーソルを含む Statement の開始行も \ 始まりなら、その Statement は
      // メタコマンド行から始まった誤パースとみなして行を実行する。
      // 開始行が通常の SQL (複数行文の途中の文字列リテラル等) の場合のみ
      // Statement 全体を実行する。
      const inStatement =
        range !== null && head >= range.from && head <= range.to;
      if (!inStatement) {
        return lineText;
      }
      const statementFirstLine = state.doc.lineAt(range.from);
      const statementFirstLineText = state
        .sliceDoc(statementFirstLine.from, statementFirstLine.to)
        .trim();
      if (statementFirstLineText.startsWith("\\")) {
        return lineText;
      }
    }
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
        const range = statementRangeAt(
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
    if (!view) {
      return;
    }
    const statement = currentStatementText(view.state);
    if (statement.trim()) {
      onRun(statement);
    }
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
          languageCompartment.of(sql({ dialect: dialectFor(engine) })),
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

  // エンジン変更時に SQL 方言を差し替える
  $effect(() => {
    const dialect = dialectFor(engine);
    if (!view) {
      return;
    }
    view.dispatch({
      effects: languageCompartment.reconfigure(sql({ dialect })),
    });
  });
</script>

<div
  bind:this={editorElement}
  class="h-full min-h-0 overflow-hidden"
  data-annotate="editor-sql"
></div>
