/**
 * AD-17 (docs/design/copilot-agent-desktop.md): the gesture behind `browser_drag`, free of
 * Electron so it can be tested against a stand-in CDP and run unchanged by the driver.
 *
 * Chromium's drag interception is on for the gesture. A page that starts a native HTML drag
 * answers with `Input.dragIntercepted`, and the drag is finished with `Input.dispatchDragEvent`
 * at the destination with the intercepted data; a page that drags with pointer events gets the
 * mouse sequence alone. Either way the press is released at the destination, interception is
 * switched off again, and the system pointer never moves: no OS drag session starts and nothing
 * from the disk can be dropped.
 *
 * The sequence is the one a person makes, because real drag libraries depend on its parts:
 *
 * - **Hover, press, then a short lift** of 2, 5 and 10 px toward the destination, one animation
 *   frame apart. That passes every common start threshold: jQuery UI's 1 px `distance`,
 *   Chromium's 4 px native drag threshold, and the 5 to 10 px activation distance pointer
 *   libraries use, before the pointer travels.
 * - **The destination is found again after the lift.** A page often reflows when it picks the
 *   source up (it hides it, or puts a placeholder in its place), so a drop point measured
 *   before the press can land on the wrong item.
 * - **The travel is many small moves**, one frame apart (one per 20 CSS px by default), so a
 *   sortable sees the pointer cross each item on the way, as with a person's hand.
 * - **A native drag** gets `dragEnter` where it started, `dragOver` along the same path, then
 *   several `dragOver`s at the destination over a few frames before the drop. A page that
 *   reorders on `dragover`, or that only accepts drags after a tick (SortableJS does both),
 *   needs those, and the last `dragOver` decides whether Chromium lets the drop through.
 * - **A drag that starts late** (a page that sets `draggable` on press, or starts a native
 *   drag partway through) is still finished as a native drag, never left open: an open drag
 *   would make the page ignore every later mouse event.
 */
import type { CdpEvent } from './cdp';

export type Point = { x: number; y: number };

/** The two things the gesture needs from a tab's CDP client (`TabCdp` has both). */
export type DragCdp = {
  send(method: string, params?: Record<string, unknown>): Promise<unknown>;
  on(listener: (event: CdpEvent) => void): () => void;
};

export type DragMechanism = 'pointer events' | 'an HTML drag';

export type DragOptions = {
  from: Point;
  to: Point;
  /** Moves of the travel, 1 to 50. Absent: one per 20 CSS px, at least 10. */
  steps?: number | null;
  /**
   * Where the destination is now, asked once the source is picked up. Null keeps `to` (a drop
   * at a point, or a destination that left the visible page).
   */
  locateTarget?: () => Promise<Point | null>;
  signal?: AbortSignal;
};

/** One animation frame, the pace a person's pointer reports at. */
export const FRAME_MS = 16;
/** The lift, in CSS px from the press toward the destination. */
export const LIFT_PX: readonly number[] = [2, 5, 10];
/** `dragOver`s at the destination before a native drop. */
export const HOVER_FRAMES = 4;
const MAX_STEPS = 50;
const MIN_AUTO_STEPS = 10;
const PX_PER_STEP = 20;

function wait(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(new Error('stopped'));
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener(
      'abort',
      () => {
        clearTimeout(timer);
        reject(new Error('stopped'));
      },
      { once: true }
    );
  });
}

function mouse(
  cdp: DragCdp,
  type: 'mouseMoved' | 'mousePressed' | 'mouseReleased',
  at: Point,
  pressed: boolean
): Promise<unknown> {
  // `buttons` is the state after the event, as a browser reports it: held from the press until
  // the release. `button` names the held button on a move, which jQuery UI reads as `which`.
  const held = type === 'mousePressed' || (type === 'mouseMoved' && pressed);
  return cdp.send('Input.dispatchMouseEvent', {
    type,
    x: at.x,
    y: at.y,
    button: type === 'mouseMoved' && !pressed ? 'none' : 'left',
    buttons: held ? 1 : 0,
    clickCount: type === 'mouseMoved' ? 0 : 1,
  });
}

/** How many moves the travel from `from` to `to` takes. */
export function travelSteps(from: Point, to: Point, steps?: number | null): number {
  if (typeof steps === 'number' && Number.isFinite(steps) && steps >= 1) {
    return Math.min(MAX_STEPS, Math.max(1, Math.round(steps)));
  }
  const distance = Math.hypot(to.x - from.x, to.y - from.y);
  return Math.min(MAX_STEPS, Math.max(MIN_AUTO_STEPS, Math.ceil(distance / PX_PER_STEP)));
}

/** The points of the travel, `from` excluded and `to` included. */
export function travelPath(from: Point, to: Point, moves: number): Point[] {
  const path: Point[] = [];
  for (let step = 1; step <= moves; step += 1) {
    path.push({
      x: from.x + ((to.x - from.x) * step) / moves,
      y: from.y + ((to.y - from.y) * step) / moves,
    });
  }
  return path;
}

/** The lift's points: short moves from `from` toward `to`, never past it. */
export function liftPath(from: Point, to: Point): Point[] {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const distance = Math.hypot(dx, dy);
  // A drop on the press point itself still has to leave it to start a drag: lift downward and
  // let the travel come back.
  const [ux, uy] = distance >= 1 ? [dx / distance, dy / distance] : [0, 1];
  const reach = distance >= 1 ? distance : Infinity;
  const points: Point[] = [];
  for (const px of LIFT_PX) {
    const d = Math.min(px, reach);
    const point = { x: from.x + ux * d, y: from.y + uy * d };
    const last = points[points.length - 1];
    if (!last || last.x !== point.x || last.y !== point.y) points.push(point);
  }
  return points;
}

/** Press at `from`, lift, travel to the destination and drop there. */
export async function performDrag(cdp: DragCdp, options: DragOptions): Promise<DragMechanism> {
  const { from, signal } = options;
  let to = options.to;
  let at = from;
  let intercepted: Record<string, unknown> | null = null;
  /** The drop of an intercepted drag went out, so the page's drag is over. */
  let dropSent = false;
  const stopListening = cdp.on((event) => {
    if (event.method === 'Input.dragIntercepted' && !event.sessionId)
      intercepted = (event.params.data as Record<string, unknown>) ?? null;
  });
  const dropped = (): Record<string, unknown> | null => intercepted;

  const relocate = async () => {
    if (!options.locateTarget) return;
    const found = await options.locateTarget().catch(() => null);
    if (found) to = found;
  };

  const finishHtmlDrag = async (data: Record<string, unknown>): Promise<DragMechanism> => {
    // The page's own dragstart follow-ups (a hidden source, a placeholder, a library that
    // accepts drags only after a tick) run before the destination is measured.
    await wait(FRAME_MS * 2, signal);
    await relocate();
    await cdp.send('Input.dispatchDragEvent', { type: 'dragEnter', x: at.x, y: at.y, data });
    await wait(FRAME_MS, signal);
    for (const point of travelPath(at, to, travelSteps(at, to, options.steps))) {
      await cdp.send('Input.dispatchDragEvent', { type: 'dragOver', x: point.x, y: point.y, data });
      at = point;
      await wait(FRAME_MS, signal);
    }
    for (let frame = 0; frame < HOVER_FRAMES; frame += 1) {
      await cdp.send('Input.dispatchDragEvent', { type: 'dragOver', x: to.x, y: to.y, data });
      await wait(FRAME_MS, signal);
    }
    await cdp.send('Input.dispatchDragEvent', { type: 'drop', x: to.x, y: to.y, data });
    dropSent = true;
    await mouse(cdp, 'mouseReleased', to, true);
    return 'an HTML drag';
  };

  try {
    await cdp.send('Input.setInterceptDrags', { enabled: true });
    await mouse(cdp, 'mouseMoved', from, false);
    await wait(FRAME_MS, signal);
    await mouse(cdp, 'mousePressed', from, true);
    await wait(FRAME_MS, signal);
    for (const point of liftPath(from, to)) {
      await mouse(cdp, 'mouseMoved', point, true);
      at = point;
      await wait(FRAME_MS, signal);
      if (dropped()) break;
    }
    // Room for a drag that starts on the last lift move, and for the page's lift handlers.
    if (!dropped()) await wait(FRAME_MS * 2, signal);
    let data = dropped();
    if (data) return await finishHtmlDrag(data);

    await relocate();
    for (const point of travelPath(at, to, travelSteps(at, to, options.steps))) {
      await mouse(cdp, 'mouseMoved', point, true);
      at = point;
      await wait(FRAME_MS, signal);
      data = dropped();
      if (data) return await finishHtmlDrag(data);
    }
    // Dwell on the destination: a page that tracks the pointer in animation frames sees it there.
    await wait(FRAME_MS * 2, signal);
    await mouse(cdp, 'mouseMoved', to, true);
    await wait(100, signal);
    data = dropped();
    if (data) return await finishHtmlDrag(data);
    await mouse(cdp, 'mouseReleased', to, true);
    return 'pointer events';
  } catch (error) {
    // Never leave the page holding a pressed button or an open drag.
    const data = dropped();
    if (data && !dropSent)
      await cdp
        .send('Input.dispatchDragEvent', { type: 'dragCancel', x: at.x, y: at.y, data })
        .catch(() => undefined);
    await mouse(cdp, 'mouseReleased', at, true).catch(() => undefined);
    throw error;
  } finally {
    await cdp.send('Input.setInterceptDrags', { enabled: false }).catch(() => undefined);
    stopListening();
  }
}
