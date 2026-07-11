<script lang="ts">
  // ペイン間のドラッグ可能な区切り線。
  // 透明のヒットエリアを既存の border (可視線) に重ねる想定なので、
  // 負マージンで両隣に 2px ずつ食い込ませている。
  // ドラッグは Pointer Events + setPointerCapture で追跡するため、
  // カーソルが区切り線から外れても (CodeMirror や webview の上でも) 追従する。
  interface Props {
    /// vertical = 縦線 (左右にドラッグ) / horizontal = 横線 (上下にドラッグ)
    direction: "vertical" | "horizontal";
    /// ドラッグ中の移動量 (px)。vertical は X、horizontal は Y
    onDrag: (delta: number) => void;
    /// ドラッグ終了時 (サイズの永続化用)
    onDragEnd?: () => void;
    /// data-annotate 識別子
    annotate: string;
  }

  let { direction, onDrag, onDragEnd, annotate }: Props = $props();

  let dragging = $state(false);
  let lastPos = 0;

  function position(e: PointerEvent): number {
    return direction === "vertical" ? e.clientX : e.clientY;
  }

  function handlePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    dragging = true;
    lastPos = position(e);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function handlePointerMove(e: PointerEvent) {
    if (!dragging) return;
    const pos = position(e);
    onDrag(pos - lastPos);
    lastPos = pos;
  }

  function handlePointerUp(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    onDragEnd?.();
  }
</script>

<div
  data-annotate={annotate}
  role="separator"
  aria-orientation={direction}
  class="relative z-10 shrink-0 select-none transition-colors {direction ===
  'vertical'
    ? '-mx-[3px] w-[6px] cursor-col-resize'
    : '-my-[3px] h-[6px] cursor-row-resize'} {dragging
    ? 'bg-blue-500/60'
    : 'bg-transparent hover:bg-blue-500/40'}"
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onpointercancel={handlePointerUp}
></div>
