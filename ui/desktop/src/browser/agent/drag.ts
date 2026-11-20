/**
 * AD-17 (docs/design/copilot-agent-desktop.md): the gesture behind `browser_drag`, free of
 * Electron so it can be tested against a stand-in CDP and run unchanged by the driver.
 *
 * Chromium's drag interception is on for the gesture. A page that starts a native HTML drag
 * answers with `Input.dragIntercepted`, and the drag is finished with `Input.dispatchDragEvent`
 * (`dragEnter`, `dragOver`, `drop`) at the destination with the intercepted data; a page that
 * drags with pointer events gets the mouse sequence alone. Either way the press is released at
 * the destination, interception is switched off again, and the system pointer never moves: no
 * OS drag session starts and nothing from the disk can be dropped.
 */
import type { CdpEvent } from './cdp';

export type Point = { x: number; y: number };

/** The two things the gesture needs from a tab's CDP client (`TabCdp` has both). */
export type DragCdp = {
  send(method: string, params?: Record<string, unknown>): Promise<unknown>;
  on(listener: (event: CdpEvent) => void): () => void;
};

export type DragMechanism = 'pointer events' | 'an HTML drag';

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
  return cdp.send('Input.dispatchMouseEvent', {
    type,
    x: at.x,
    y: at.y,
    button: type === 'mouseMoved' && !pressed ? 'none' : 'left',
    buttons: pressed && type === 'mouseMoved' ? 1 : 0,
    clickCount: type === 'mouseMoved' ? 0 : 1,
  });
}

/** Press at `from`, move to `to` in `steps` moves, and drop there. */
export async function performDrag(
  cdp: DragCdp,
  from: Point,
  to: Point,
  steps: number,
  signal?: AbortSignal
): Promise<DragMechanism> {
  const moves = Math.min(50, Math.max(1, Math.round(steps) || 10));
  let intercepted: Record<string, unknown> | null = null;
  const stopListening = cdp.on((event) => {
    if (event.method === 'Input.dragIntercepted' && !event.sessionId)
      intercepted = (event.params.data as Record<string, unknown>) ?? null;
  });
  const dropped = (): Record<string, unknown> | null => intercepted;
  try {
    await cdp.send('Input.setInterceptDrags', { enabled: true });
    await mouse(cdp, 'mouseMoved', from, false);
    await mouse(cdp, 'mousePressed', from, true);
    for (let step = 1; step <= moves && !dropped(); step += 1) {
      const at = {
        x: from.x + ((to.x - from.x) * step) / moves,
        y: from.y + ((to.y - from.y) * step) / moves,
      };
      await mouse(cdp, 'mouseMoved', at, true);
      await wait(16, signal);
    }
    const data = dropped();
    let mechanism: DragMechanism = 'pointer events';
    if (data) {
      mechanism = 'an HTML drag';
      for (const type of ['dragEnter', 'dragOver'] as const)
        await cdp.send('Input.dispatchDragEvent', { type, x: to.x, y: to.y, data });
      await wait(50, signal);
      await cdp.send('Input.dispatchDragEvent', { type: 'drop', x: to.x, y: to.y, data });
    } else {
      await mouse(cdp, 'mouseMoved', to, true);
      await wait(100, signal);
    }
    await mouse(cdp, 'mouseReleased', to, true);
    return mechanism;
  } catch (error) {
    // Never leave the page holding a pressed button or an open drag.
    const data = dropped();
    if (data)
      await cdp
        .send('Input.dispatchDragEvent', { type: 'dragCancel', x: to.x, y: to.y, data })
        .catch(() => undefined);
    await mouse(cdp, 'mouseReleased', to, true).catch(() => undefined);
    throw error;
  } finally {
    await cdp.send('Input.setInterceptDrags', { enabled: false }).catch(() => undefined);
    stopListening();
  }
}
