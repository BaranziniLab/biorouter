import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  FN_DOUBLE_TAP_GAP_MS,
  FN_GESTURE_IDLE,
  FN_HOLD_MS,
  FnGestureDriver,
  nextFnGesture,
  type FnGestureEvent,
  type FnGestureState,
  type FnIntent,
} from './fnGesture';

/** Runs `events` through the reducer from idle and returns every intent, in order. */
function run(events: FnGestureEvent[], start: FnGestureState = FN_GESTURE_IDLE) {
  let state = start;
  const intents: FnIntent[] = [];
  for (const event of events) {
    const result = nextFnGesture(state, event);
    state = result.state;
    intents.push(...result.intents);
  }
  return { state, intents };
}

describe('the fn gesture reducer', () => {
  it('a hold starts push to talk at the threshold and the release ends it', () => {
    const pressed = nextFnGesture(FN_GESTURE_IDLE, { type: 'fn-down', at: 1000 });
    expect(pressed.intents).toEqual([]);
    expect(pressed.deadline).toBe(1000 + FN_HOLD_MS);
    const early = nextFnGesture(pressed.state, { type: 'tick', at: 1000 + FN_HOLD_MS - 1 });
    expect(early.intents).toEqual([]);
    const held = nextFnGesture(early.state, { type: 'tick', at: 1000 + FN_HOLD_MS });
    expect(held.intents).toEqual(['start-hold']);
    expect(held.state.kind).toBe('holding');
    const released = nextFnGesture(held.state, { type: 'fn-up', at: 4000 });
    expect(released.intents).toEqual(['end-hold']);
    expect(released.state).toEqual(FN_GESTURE_IDLE);
  });

  it('a single short tap does nothing, and the wait for a second tap ends', () => {
    const { state, intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'fn-up', at: 120 },
    ]);
    expect(intents).toEqual([]);
    expect(state.kind).toBe('tapped');
    const after = run([{ type: 'tick', at: 120 + FN_DOUBLE_TAP_GAP_MS }], state);
    expect(after.intents).toEqual([]);
    expect(after.state).toEqual(FN_GESTURE_IDLE);
  });

  it('a double-tap toggles on the second release', () => {
    const { state, intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'fn-up', at: 100 },
      { type: 'fn-down', at: 100 + FN_DOUBLE_TAP_GAP_MS },
    ]);
    expect(intents).toEqual([]);
    const done = run([{ type: 'fn-up', at: 600 }], state);
    expect(done.intents).toEqual(['toggle']);
    expect(done.state).toEqual(FN_GESTURE_IDLE);
  });

  it('a second press that comes too late is a first press again', () => {
    const { state, intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'fn-up', at: 100 },
      { type: 'fn-down', at: 100 + FN_DOUBLE_TAP_GAP_MS + 1 },
      { type: 'fn-up', at: 600 },
    ]);
    expect(intents).toEqual([]);
    expect(state.kind).toBe('tapped');
  });

  it('a tap and then a hold is a hold', () => {
    const { intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'fn-up', at: 100 },
      { type: 'fn-down', at: 250 },
      { type: 'tick', at: 250 + FN_HOLD_MS },
      { type: 'fn-up', at: 2000 },
    ]);
    expect(intents).toEqual(['start-hold', 'end-hold']);
  });

  it('a press longer than the threshold is never a tap, even with a late timer', () => {
    const { intents, state } = run([
      { type: 'fn-down', at: 0 },
      { type: 'fn-up', at: 50 },
      { type: 'fn-down', at: 200 },
      { type: 'fn-up', at: 200 + FN_HOLD_MS },
    ]);
    expect(intents).toEqual([]);
    expect(state).toEqual(FN_GESTURE_IDLE);
  });

  it('fn with another key before the threshold never starts dictation', () => {
    const { intents, state } = run([
      { type: 'fn-down', at: 0 },
      { type: 'other-key', at: 80 },
      { type: 'tick', at: FN_HOLD_MS },
      { type: 'tick', at: 5000 },
    ]);
    expect(intents).toEqual([]);
    expect(state.kind).toBe('spoiled');
    expect(run([{ type: 'fn-up', at: 6000 }], state).state).toEqual(FN_GESTURE_IDLE);
  });

  it('fn with another key after a hold started cancels that recording', () => {
    const { intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'tick', at: FN_HOLD_MS },
      { type: 'other-key', at: 900 },
      { type: 'fn-up', at: 1000 },
    ]);
    expect(intents).toEqual(['start-hold', 'cancel-hold']);
  });

  it('fn with another key on the second press drops the double-tap', () => {
    const { intents, state } = run([
      { type: 'fn-down', at: 0 },
      { type: 'fn-up', at: 100 },
      { type: 'fn-down', at: 200 },
      { type: 'other-key', at: 250 },
      { type: 'fn-up', at: 300 },
    ]);
    expect(intents).toEqual([]);
    expect(state).toEqual(FN_GESTURE_IDLE);
  });

  it('a spoiled press followed by a tap is not a double-tap', () => {
    const { intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'other-key', at: 50 },
      { type: 'fn-up', at: 100 },
      { type: 'fn-down', at: 200 },
      { type: 'fn-up', at: 260 },
    ]);
    expect(intents).toEqual([]);
  });

  it('repeated downs and stray ups change nothing', () => {
    const { intents, state } = run([
      { type: 'fn-up', at: 0 },
      { type: 'fn-down', at: 10 },
      { type: 'fn-down', at: 20 },
    ]);
    expect(intents).toEqual([]);
    expect(state).toEqual({ kind: 'pressed', downAt: 10, second: false });
  });

  it('a reset ends a hold as its release would, and forgets anything else', () => {
    expect(nextFnGesture({ kind: 'holding' }, { type: 'reset' }).intents).toEqual(['end-hold']);
    for (const state of [
      FN_GESTURE_IDLE,
      { kind: 'pressed', downAt: 0, second: true } as FnGestureState,
      { kind: 'tapped', upAt: 0 } as FnGestureState,
      { kind: 'spoiled' } as FnGestureState,
    ]) {
      const result = nextFnGesture(state, { type: 'reset' });
      expect(result.intents, state.kind).toEqual([]);
      expect(result.state).toEqual(FN_GESTURE_IDLE);
    }
  });

  it('a step that leaves the state alone keeps its deadline', () => {
    const pressed = nextFnGesture(FN_GESTURE_IDLE, { type: 'fn-down', at: 0 });
    expect(nextFnGesture(pressed.state, { type: 'tick', at: 10 }).deadline).toBe(FN_HOLD_MS);
    const tapped = run([
      { type: 'fn-down', at: 0 },
      { type: 'fn-up', at: 50 },
    ]).state;
    expect(nextFnGesture(tapped, { type: 'tick', at: 60 }).deadline).toBe(
      50 + FN_DOUBLE_TAP_GAP_MS
    );
  });
});

describe('the gesture driver', () => {
  let now = 0;
  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
  });
  afterEach(() => vi.useRealTimers());

  const advance = (ms: number) => {
    now += ms;
    vi.advanceTimersByTime(ms);
  };

  it('starts push to talk from its own timer while fn stays down', () => {
    const intents: FnIntent[] = [];
    const driver = new FnGestureDriver(
      (intent) => intents.push(intent),
      () => now
    );
    driver.feed({ type: 'fn-down', at: now });
    advance(FN_HOLD_MS - 1);
    expect(intents).toEqual([]);
    advance(1);
    expect(intents).toEqual(['start-hold']);
    advance(2000);
    driver.feed({ type: 'fn-up', at: now });
    expect(intents).toEqual(['start-hold', 'end-hold']);
  });

  it('applies an overdue tick before the next event, so a late timer cannot lose a hold', () => {
    const intents: FnIntent[] = [];
    const driver = new FnGestureDriver(
      (intent) => intents.push(intent),
      () => now
    );
    driver.feed({ type: 'fn-down', at: 0 });
    // The timer has not run (the event loop was busy), and fn comes up after the threshold.
    driver.feed({ type: 'fn-up', at: FN_HOLD_MS + 500 });
    expect(intents).toEqual(['start-hold', 'end-hold']);
  });

  it('a double-tap toggles, and a single tap leaves no timer behind', () => {
    const intents: FnIntent[] = [];
    const driver = new FnGestureDriver(
      (intent) => intents.push(intent),
      () => now
    );
    driver.feed({ type: 'fn-down', at: now });
    advance(80);
    driver.feed({ type: 'fn-up', at: now });
    advance(150);
    driver.feed({ type: 'fn-down', at: now });
    advance(80);
    driver.feed({ type: 'fn-up', at: now });
    expect(intents).toEqual(['toggle']);
    driver.feed({ type: 'fn-down', at: now });
    advance(80);
    driver.feed({ type: 'fn-up', at: now });
    advance(FN_DOUBLE_TAP_GAP_MS + 10);
    expect(driver.current).toEqual(FN_GESTURE_IDLE);
    expect(vi.getTimerCount()).toBe(0);
    expect(intents).toEqual(['toggle']);
  });

  it('dispose ends a hold and clears its timer', () => {
    const intents: FnIntent[] = [];
    const driver = new FnGestureDriver(
      (intent) => intents.push(intent),
      () => now
    );
    driver.feed({ type: 'fn-down', at: now });
    advance(FN_HOLD_MS);
    driver.dispose();
    expect(intents).toEqual(['start-hold', 'end-hold']);
    expect(vi.getTimerCount()).toBe(0);
  });
});
