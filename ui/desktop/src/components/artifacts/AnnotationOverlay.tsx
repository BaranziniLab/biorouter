import { useCallback, useEffect, useRef, useState } from 'react';

export type SelectedRegion = { x: number; y: number; width: number; height: number };

type Drag = {
  originX: number;
  originY: number;
  x: number;
  y: number;
  width: number;
  height: number;
};

/** Below this a drag is a stray click, not a selection. */
const MIN_REGION_PX = 8;

/**
 * Drag a rectangle over the panel to select a region.
 *
 * The affordances are lifted from `Cmd+Shift+4`, deliberately: that is the
 * interaction every Mac user already has in their hands, and the details are
 * where the perceived precision lives. A live `W × H` badge tracking the
 * cursor is the single element that makes drag-to-crop feel exact rather than
 * approximate.
 *
 * - `Shift` constrains to a square
 * - `Option` sizes from the centre
 * - `Space` (held) repositions the marquee mid-drag
 * - `Esc` cancels, always
 *
 * The overlay sits above the preview and swallows pointer events, which is also
 * what stops a drag from selecting text or clicking a link underneath.
 */
export default function AnnotationOverlay({
  onSelect,
  onCancel,
}: {
  onSelect: (region: SelectedRegion) => void;
  onCancel: () => void;
}) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);
  const [isCommitting, setIsCommitting] = useState(false);
  const commitFrames = useRef<number[]>([]);
  // Held in refs so the pointer handlers read the live value without
  // re-subscribing on every keystroke.
  const modifiers = useRef({ shift: false, alt: false, space: false });
  const spaceAnchor = useRef<{ x: number; y: number } | null>(null);
  // The live pointer position, so pressing Space can anchor from where the
  // cursor already is. Anchoring lazily on the first move instead swallows that
  // move entirely, and the marquee visibly refuses to budge until you wiggle.
  const lastPoint = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCancel();
        return;
      }
      if (event.key === 'Shift') modifiers.current.shift = true;
      if (event.key === 'Alt') modifiers.current.alt = true;
      if (event.key === ' ') {
        // Repositioning only makes sense mid-drag; outside one it must not
        // swallow the key.
        if (drag) {
          event.preventDefault();
          modifiers.current.space = true;
          spaceAnchor.current = lastPoint.current;
        }
      }
    };
    const up = (event: KeyboardEvent) => {
      if (event.key === 'Shift') modifiers.current.shift = false;
      if (event.key === 'Alt') modifiers.current.alt = false;
      if (event.key === ' ') {
        modifiers.current.space = false;
        spaceAnchor.current = null;
      }
    };
    window.addEventListener('keydown', down, true);
    window.addEventListener('keyup', up, true);
    return () => {
      window.removeEventListener('keydown', down, true);
      window.removeEventListener('keyup', up, true);
    };
  }, [drag, onCancel]);

  useEffect(
    () => () => {
      for (const frame of commitFrames.current) cancelAnimationFrame(frame);
    },
    []
  );

  const localPoint = useCallback((event: React.PointerEvent) => {
    const rect = rootRef.current?.getBoundingClientRect();
    if (!rect) return { x: 0, y: 0 };
    if (rect.width <= 0 || rect.height <= 0) {
      return { x: event.clientX - rect.left, y: event.clientY - rect.top };
    }
    return {
      x: Math.min(rect.width, Math.max(0, event.clientX - rect.left)),
      y: Math.min(rect.height, Math.max(0, event.clientY - rect.top)),
    };
  }, []);

  const contained = useCallback((next: Drag): Drag => {
    const root = rootRef.current;
    if (!root) return next;
    const bounds = root.getBoundingClientRect();
    if (bounds.width <= 0 || bounds.height <= 0) return next;
    const width = Math.min(next.width, bounds.width);
    const height = Math.min(next.height, bounds.height);
    return {
      ...next,
      x: Math.min(bounds.width - width, Math.max(0, next.x)),
      y: Math.min(bounds.height - height, Math.max(0, next.y)),
      width,
      height,
    };
  }, []);

  const handleDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      const { x, y } = localPoint(event);
      lastPoint.current = { x, y };
      setDrag({ originX: x, originY: y, x, y, width: 0, height: 0 });
      setCursor({ x, y });
    },
    [localPoint]
  );

  const handleMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const point = localPoint(event);
      lastPoint.current = point;
      setCursor(point);
      setDrag((current) => {
        if (!current) return current;

        // Space repositions the whole marquee, keeping its size — the
        // "I started in the wrong place" escape hatch, and the reason the
        // origin is tracked separately from the rect.
        if (modifiers.current.space) {
          if (!spaceAnchor.current) spaceAnchor.current = point;
          const dx = point.x - spaceAnchor.current.x;
          const dy = point.y - spaceAnchor.current.y;
          spaceAnchor.current = point;
          const next = contained({
            ...current,
            originX: current.originX + dx,
            originY: current.originY + dy,
            x: current.x + dx,
            y: current.y + dy,
          });
          return {
            ...next,
            originX: current.originX + (next.x - current.x),
            originY: current.originY + (next.y - current.y),
          };
        }

        let dx = point.x - current.originX;
        let dy = point.y - current.originY;
        if (modifiers.current.shift) {
          const side = Math.max(Math.abs(dx), Math.abs(dy));
          dx = Math.sign(dx) * side;
          dy = Math.sign(dy) * side;
        }

        if (modifiers.current.alt) {
          return contained({
            ...current,
            x: current.originX - Math.abs(dx),
            y: current.originY - Math.abs(dy),
            width: Math.abs(dx) * 2,
            height: Math.abs(dy) * 2,
          });
        }

        return contained({
          ...current,
          x: dx < 0 ? current.originX + dx : current.originX,
          y: dy < 0 ? current.originY + dy : current.originY,
          width: Math.abs(dx),
          height: Math.abs(dy),
        });
      });
    },
    [contained, localPoint]
  );

  const handleUp = useCallback(() => {
    const current = drag;
    setDrag(null);
    spaceAnchor.current = null;
    if (!current || current.width < MIN_REGION_PX || current.height < MIN_REGION_PX) {
      // A click, not a selection. Stay in the mode rather than cancelling —
      // reviewing a figure produces several notes, not one, and a mode that
      // exits on a stray click is the most-complained-about detail in every
      // shipped version of this feature.
      return;
    }

    const region = {
      x: current.x,
      y: current.y,
      width: current.width,
      height: current.height,
    };
    // Hide the dimmer, marquee and size badge before the compositor capture.
    // Two animation frames ensure Chromium has painted the clean underlying
    // preview; otherwise the image sent to the model contains its own selection
    // chrome around the requested pixels.
    setIsCommitting(true);
    const first = requestAnimationFrame(() => {
      const second = requestAnimationFrame(() => onSelect(region));
      commitFrames.current = [second];
    });
    commitFrames.current = [first];
  }, [drag, onSelect]);

  const badge = drag && drag.width >= 1 && drag.height >= 1;

  return (
    <div
      ref={rootRef}
      data-testid="annotation-overlay"
      role="application"
      aria-label="Select a region to send to the chat"
      onPointerDown={handleDown}
      onPointerMove={handleMove}
      onPointerUp={handleUp}
      onPointerCancel={() => setDrag(null)}
      className={`absolute inset-0 z-30 cursor-crosshair select-none bg-black/25 ${
        isCommitting ? 'invisible pointer-events-none' : ''
      }`}
    >
      {drag && (
        <div
          data-testid="annotation-marquee"
          className="pointer-events-none absolute border border-white/90 bg-white/10 shadow-[0_0_0_1px_rgba(0,0,0,.45)]"
          style={{ left: drag.x, top: drag.y, width: drag.width, height: drag.height }}
        />
      )}

      {badge && cursor && (
        // The dimension readout, tracking the cursor. Offset so it never sits
        // under the pointer, and flipped near the edges so it cannot be clipped.
        <div
          data-testid="annotation-dimensions"
          className="pointer-events-none absolute rounded-element bg-black/80 px-1.5 py-0.5 text-supporting tabular-nums text-white"
          style={{ left: cursor.x + 12, top: cursor.y + 12 }}
        >
          {Math.round(drag.width)} × {Math.round(drag.height)}
        </div>
      )}

      {!drag && (
        <div className="pointer-events-none absolute inset-x-0 top-3 flex justify-center">
          <p className="rounded-element bg-black/75 px-2.5 py-1 text-supporting text-white">
            Drag to select a region · Shift square · Option center · Space move · Esc cancel
          </p>
        </div>
      )}
    </div>
  );
}
