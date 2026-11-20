import { describe, expect, it } from 'vitest';
import type { CdpEvent } from './cdp';
import { performDrag, type DragCdp } from './drag';

/** A stand-in CDP: records every command, and starts a native drag on the first pressed move. */
function fakeCdp(options: { htmlDrag: boolean; failOn?: string }) {
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  const listeners = new Set<(event: CdpEvent) => void>();
  const cdp: DragCdp = {
    async send(method, params = {}) {
      calls.push({ method, params });
      if (options.failOn && (params as { type?: string }).type === options.failOn)
        throw new Error('the page went away');
      if (
        options.htmlDrag &&
        method === 'Input.dispatchMouseEvent' &&
        params.type === 'mouseMoved' &&
        params.buttons === 1
      ) {
        for (const listener of listeners)
          listener({
            method: 'Input.dragIntercepted',
            params: { data: { items: [], dragOperationsMask: 1 } },
          });
      }
      return {};
    },
    on(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  const kinds = () =>
    calls.map((call) =>
      call.method === 'Input.setInterceptDrags'
        ? `intercept:${call.params.enabled}`
        : `${String(call.params.type)}`
    );
  return { cdp, calls, kinds, listeners };
}

describe('performDrag (AD-17)', () => {
  it('finishes a native HTML drag with drag events at the destination', async () => {
    const fake = fakeCdp({ htmlDrag: true });
    const mechanism = await performDrag(fake.cdp, { x: 10, y: 10 }, { x: 110, y: 60 }, 10);
    expect(mechanism).toBe('an HTML drag');
    expect(fake.kinds()).toEqual([
      'intercept:true',
      'mouseMoved',
      'mousePressed',
      'mouseMoved',
      'dragEnter',
      'dragOver',
      'drop',
      'mouseReleased',
      'intercept:false',
    ]);
    const drop = fake.calls.find((call) => call.params.type === 'drop');
    expect(drop?.params).toMatchObject({ x: 110, y: 60, data: { items: [] } });
    expect(fake.listeners.size).toBe(0);
  });

  it('moves in steps with the button held for a pointer-events drag', async () => {
    const fake = fakeCdp({ htmlDrag: false });
    const mechanism = await performDrag(fake.cdp, { x: 0, y: 0 }, { x: 40, y: 0 }, 4);
    expect(mechanism).toBe('pointer events');
    const moves = fake.calls.filter(
      (call) => call.params.type === 'mouseMoved' && call.params.buttons === 1
    );
    expect(moves.map((call) => call.params.x)).toEqual([10, 20, 30, 40, 40]);
    expect(moves.every((call) => call.params.button === 'left')).toBe(true);
    expect(fake.kinds().slice(-2)).toEqual(['mouseReleased', 'intercept:false']);
    expect(fake.kinds()).not.toContain('drop');
  });

  it('cancels an open drag and releases the button when a step fails', async () => {
    const fake = fakeCdp({ htmlDrag: true, failOn: 'drop' });
    await expect(performDrag(fake.cdp, { x: 0, y: 0 }, { x: 5, y: 5 }, 2)).rejects.toThrow(
      'the page went away'
    );
    const tail = fake.kinds().slice(-3);
    expect(tail).toEqual(['dragCancel', 'mouseReleased', 'intercept:false']);
  });

  it('stops when the operation is stopped, still releasing the button', async () => {
    const fake = fakeCdp({ htmlDrag: false });
    const stop = new AbortController();
    stop.abort();
    await expect(
      performDrag(fake.cdp, { x: 0, y: 0 }, { x: 5, y: 5 }, 3, stop.signal)
    ).rejects.toThrow();
    expect(fake.kinds().slice(-2)).toEqual(['mouseReleased', 'intercept:false']);
  });
});
