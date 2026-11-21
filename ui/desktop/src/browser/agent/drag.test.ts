import { describe, expect, it } from 'vitest';
import type { CdpEvent } from './cdp';
import { HOVER_FRAMES, liftPath, performDrag, travelPath, travelSteps, type DragCdp } from './drag';

type Call = { method: string; params: Record<string, unknown> };

/**
 * A stand-in CDP: records every command, and starts a native drag on the pressed move that
 * `startsDragAt` accepts (never, for a page that drags with pointer events).
 */
function fakeCdp(
  options: {
    startsDragAt?: (params: Record<string, unknown>) => boolean;
    failOn?: string;
  } = {}
) {
  const calls: Call[] = [];
  const listeners = new Set<(event: CdpEvent) => void>();
  let started = false;
  const cdp: DragCdp = {
    async send(method, params = {}) {
      calls.push({ method, params });
      if (options.failOn && (params as { type?: string }).type === options.failOn)
        throw new Error('the page went away');
      if (
        !started &&
        options.startsDragAt &&
        method === 'Input.dispatchMouseEvent' &&
        params.type === 'mouseMoved' &&
        params.buttons === 1 &&
        options.startsDragAt(params)
      ) {
        started = true;
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
  const ofType = (type: string) => calls.filter((call) => call.params.type === type);
  return { cdp, calls, kinds, ofType, listeners };
}

/** Collapse runs of the same kind: ['a', 'b', 'b'] -> ['a', 'b*2']. */
function runs(kinds: string[]): string[] {
  const out: string[] = [];
  for (const kind of kinds) {
    const last = out[out.length - 1];
    const match = last?.match(/^(.*?)(?:\*(\d+))?$/);
    if (match && match[1] === kind) out[out.length - 1] = `${kind}*${Number(match[2] ?? 1) + 1}`;
    else out.push(kind);
  }
  return out;
}

describe('the drag path (AD-17)', () => {
  it('lifts past every common start threshold before travelling, never past the target', () => {
    expect(liftPath({ x: 0, y: 0 }, { x: 100, y: 0 })).toEqual([
      { x: 2, y: 0 },
      { x: 5, y: 0 },
      { x: 10, y: 0 },
    ]);
    expect(liftPath({ x: 0, y: 0 }, { x: 0, y: 4 })).toEqual([
      { x: 0, y: 2 },
      { x: 0, y: 4 },
    ]);
    // A drop on the press point still leaves it, so a drag can start.
    const lift = liftPath({ x: 5, y: 5 }, { x: 5, y: 5 });
    expect(lift[lift.length - 1]).toEqual({ x: 5, y: 15 });
  });

  it('travels in a move per 20 CSS px by default, or the steps asked for', () => {
    expect(travelSteps({ x: 0, y: 0 }, { x: 0, y: 50 })).toBe(10);
    expect(travelSteps({ x: 0, y: 0 }, { x: 0, y: 400 })).toBe(20);
    expect(travelSteps({ x: 0, y: 0 }, { x: 0, y: 5_000 })).toBe(50);
    expect(travelSteps({ x: 0, y: 0 }, { x: 0, y: 400 }, 3)).toBe(3);
    expect(travelSteps({ x: 0, y: 0 }, { x: 0, y: 400 }, 99)).toBe(50);
    expect(travelPath({ x: 0, y: 0 }, { x: 30, y: 0 }, 3).map((p) => p.x)).toEqual([10, 20, 30]);
  });
});

describe('performDrag (AD-17)', () => {
  it('finishes a native HTML drag with drag events along the path and a hover before the drop', async () => {
    const fake = fakeCdp({ startsDragAt: () => true });
    const mechanism = await performDrag(fake.cdp, {
      from: { x: 10, y: 10 },
      to: { x: 10, y: 110 },
      steps: 4,
    });
    expect(mechanism).toBe('an HTML drag');
    expect(runs(fake.kinds())).toEqual([
      'intercept:true',
      'mouseMoved',
      'mousePressed',
      // The first lift move starts the native drag; the page gets no more mouse moves.
      'mouseMoved',
      'dragEnter',
      `dragOver*${4 + HOVER_FRAMES}`,
      'drop',
      'mouseReleased',
      'intercept:false',
    ]);
    // dragEnter where the drag started, then the travel, ending on the destination.
    expect(fake.ofType('dragEnter')[0].params).toMatchObject({ x: 10, y: 12 });
    const overs = fake.ofType('dragOver').map((call) => call.params.y);
    expect(overs.slice(0, 4)).toEqual([36.5, 61, 85.5, 110]);
    expect(fake.ofType('drop')[0].params).toMatchObject({ x: 10, y: 110, data: { items: [] } });
    expect(fake.listeners.size).toBe(0);
  });

  it('measures the destination again once the source is picked up', async () => {
    const fake = fakeCdp({ startsDragAt: () => true });
    let asked = 0;
    await performDrag(fake.cdp, {
      from: { x: 10, y: 10 },
      to: { x: 10, y: 200 },
      locateTarget: async () => {
        asked += 1;
        // The page hid the source on dragstart: the destination moved up.
        return { x: 10, y: 150 };
      },
    });
    expect(asked).toBe(1);
    expect(fake.ofType('drop')[0].params).toMatchObject({ x: 10, y: 150 });
    const overs = fake.ofType('dragOver');
    expect(overs[overs.length - 1].params).toMatchObject({ x: 10, y: 150 });
  });

  it('presses, lifts and travels with the button held for a pointer-events drag', async () => {
    const fake = fakeCdp();
    const mechanism = await performDrag(fake.cdp, {
      from: { x: 0, y: 0 },
      to: { x: 50, y: 0 },
      steps: 4,
      locateTarget: async () => null,
    });
    expect(mechanism).toBe('pointer events');
    const pressed = fake.ofType('mousePressed')[0].params;
    expect(pressed).toMatchObject({ button: 'left', buttons: 1, clickCount: 1 });
    const held = fake.calls.filter(
      (call) => call.params.type === 'mouseMoved' && call.params.buttons === 1
    );
    // The lift (2, 5, 10), the travel from the lift in four moves, and a dwell on the target.
    expect(held.map((call) => call.params.x)).toEqual([2, 5, 10, 20, 30, 40, 50, 50]);
    expect(held.every((call) => call.params.button === 'left')).toBe(true);
    expect(fake.ofType('mouseReleased')[0].params).toMatchObject({ x: 50, buttons: 0 });
    expect(fake.kinds().slice(-2)).toEqual(['mouseReleased', 'intercept:false']);
    expect(fake.kinds()).not.toContain('drop');
  });

  it('finishes a native drag the page starts partway through, never leaving it open', async () => {
    const fake = fakeCdp({ startsDragAt: (params) => Number(params.x) >= 30 });
    const mechanism = await performDrag(fake.cdp, {
      from: { x: 0, y: 0 },
      to: { x: 60, y: 0 },
      steps: 5,
    });
    expect(mechanism).toBe('an HTML drag');
    expect(fake.ofType('dragEnter')[0].params).toMatchObject({ x: 30 });
    expect(fake.ofType('drop')[0].params).toMatchObject({ x: 60 });
    expect(fake.kinds().slice(-3)).toEqual(['drop', 'mouseReleased', 'intercept:false']);
  });

  it('cancels an open drag and releases the button when a step fails', async () => {
    const fake = fakeCdp({ startsDragAt: () => true, failOn: 'drop' });
    await expect(
      performDrag(fake.cdp, { from: { x: 0, y: 0 }, to: { x: 5, y: 5 }, steps: 2 })
    ).rejects.toThrow('the page went away');
    expect(fake.kinds().slice(-3)).toEqual(['dragCancel', 'mouseReleased', 'intercept:false']);
  });

  it('stops when the operation is stopped, still releasing the button', async () => {
    const fake = fakeCdp();
    const stop = new AbortController();
    stop.abort();
    await expect(
      performDrag(fake.cdp, {
        from: { x: 0, y: 0 },
        to: { x: 5, y: 5 },
        steps: 3,
        signal: stop.signal,
      })
    ).rejects.toThrow();
    expect(fake.kinds().slice(-2)).toEqual(['mouseReleased', 'intercept:false']);
  });
});
