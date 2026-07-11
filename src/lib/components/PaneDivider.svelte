<script lang="ts">
  // ペイン間のドラッグ可能な区切り線。
  // 透明のヒットエリアを既存の border (可視線) に重ねる想定なので、
  // 負マージンで両隣に 2px ずつ食い込ませている。
  // ドラッグは Pointer Events + setPointerCapture で追跡するため、
  // カーソルが区切り線から外れても (CodeMirror や webview の上でも) 追従する。
  interface Props {
    /// vertical = 縦線 (左右にドラッグ) / horizontal = 横線 (上下にドラッグ)
    direction: "vertical" | "horizontal";
    /// ドラッグ開始時。親はここで基準サイズをスナップショットする
    onDragStart?: () => void;
    /// ドラッグ開始位置からの累積移動量 (px)。vertical は X、horizontal は Y。
    /// 相対 delta ではなく累積にすることで、クランプで飽和しても基準サイズは
    /// 動かず、上下限を超えて戻したときにポインタとペイン端がずれない。
    onDrag: (totalDelta: number) => void;
    /// ドラッグ終了時 (サイズの永続化用)
    onDragEnd?: () => void;
    /// data-annotate 識別子
    annotate: string;
  }

  let { direction, onDragStart, onDrag, onDragEnd, annotate }: Props = $props();

  let dragging = $state(false);
  let startPos = 0;

  function position(e: PointerEvent): number {
    return direction === "vertical" ? e.clientX : e.clientY;
  }

  function handlePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    dragging = true;
    startPos = position(e);
    onDragStart?.();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function handlePointerMove(e: PointerEvent) {
    if (!dragging) return;
    onDrag(position(e) - startPos);
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
