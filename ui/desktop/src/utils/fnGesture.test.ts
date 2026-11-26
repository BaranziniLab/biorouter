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
  type FnIntentEvent,
} from './fnGesture';

/** Runs `events` through the reducer from idle and returns every intent, in order. */
function run(events: FnGestureEvent[], start: FnGestureState = FN_GESTURE_IDLE) {
  let state = start;
  const steps: FnIntentEvent[] = [];
  for (const event of events) {
    const result = nextFnGesture(state, event);
    state = result.state;
    steps.push(...result.intents);
  }
  const intents: FnIntent[] = steps.map((s) => s.intent);
  return { state, intents, steps };
}

const tap = (at: number, length = 90): FnGestureEvent[] => [
  { type: 'fn-down', at },
  { type: 'fn-up', at: at + length },
];

describe('the fn gesture reducer', () => {
  it('a hold captures from fn down and is confirmed only by the helper’s held line', () => {
    const pressed = nextFnGesture(FN_GESTURE_IDLE, { type: 'fn-down', at: 1000 });
    expect(pressed.intents).toEqual([{ intent: 'prime', gesture: 1 }]);
    expect(pressed.state.kind).toBe('pressed');
    // Nothing on main's side can confirm it: no tick, and another fn down changes nothing.
    const again = nextFnGesture(pressed.state, { type: 'fn-down', at: 1000 + FN_HOLD_MS + 50 });
    expect(again.intents).toEqual([]);
    expect(again.state.kind).toBe('pressed');
    const held = nextFnGesture(again.state, { type: 'held', at: 1000 + FN_HOLD_MS });
    expect(held.intents).toEqual([{ intent: 'start-hold', gesture: 1 }]);
    expect(held.state.kind).toBe('holding');
    const released = nextFnGesture(held.state, { type: 'fn-up', at: 4000 });
    expect(released.intents).toEqual([{ intent: 'end-hold', gesture: 1 }]);
    expect(released.state).toEqual({ kind: 'idle', gesture: 1 });
  });

  it('a single short tap discards what its press captured', () => {
    const { state, intents } = run(tap(0, 120));
    expect(intents).toEqual(['prime', 'cancel-hold']);
    expect(state).toEqual({ kind: 'tapped', gesture: 1, upAt: 120 });
  });

  it('a double-tap toggles on the second release', () => {
    const { state, intents } = run([
      ...tap(0, 100),
      { type: 'fn-down', at: 100 + FN_DOUBLE_TAP_GAP_MS },
    ]);
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime']);
    // The second press's capture is not discarded: `toggle` keeps it, hands-free.
    const done = run([{ type: 'fn-up', at: 600 }], state);
    expect(done.intents).toEqual(['toggle']);
    expect(done.state.kind).toBe('idle');
  });

  it('the wait for a second tap ends by event times, with nothing pending', () => {
    const { state, intents, steps } = run([
      ...tap(0, 100),
      { type: 'fn-down', at: 100 + FN_DOUBLE_TAP_GAP_MS + 1 },
      { type: 'fn-up', at: 600 },
    ]);
    // The second press came too late, so it is a first press of a new gesture.
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'cancel-hold']);
    expect(steps.map((s) => s.gesture)).toEqual([1, 1, 2, 2]);
    expect(state).toEqual({ kind: 'tapped', gesture: 2, upAt: 600 });
    // However long the tapped state waits, a later press is judged by its own time.
    const later = run(tap(60_000), { kind: 'tapped', gesture: 2, upAt: 600 });
    expect(later.intents).toEqual(['prime', 'cancel-hold']);
    expect(later.steps[0].gesture).toBe(3);
  });

  it('one gesture id runs from the first press to the toggle, and the next gesture takes a new one', () => {
    const { steps } = run([
      ...tap(0),
      ...tap(200),
      // A hold.
      { type: 'fn-down', at: 5000 },
      { type: 'held', at: 5000 + FN_HOLD_MS },
      { type: 'fn-up', at: 6000 },
      // fn with another key.
      { type: 'fn-down', at: 8000 },
      { type: 'other-key', at: 8050 },
      { type: 'fn-up', at: 8100 },
      // A double-tap again.
      ...tap(9000),
      ...tap(9200),
    ]);
    expect(steps).toEqual([
      { intent: 'prime', gesture: 1 },
      { intent: 'cancel-hold', gesture: 1 },
      { intent: 'prime', gesture: 1 },
      { intent: 'toggle', gesture: 1 },
      { intent: 'prime', gesture: 2 },
      { intent: 'start-hold', gesture: 2 },
      { intent: 'end-hold', gesture: 2 },
      { intent: 'prime', gesture: 3 },
      { intent: 'cancel-hold', gesture: 3 },
      { intent: 'prime', gesture: 4 },
      { intent: 'cancel-hold', gesture: 4 },
      { intent: 'prime', gesture: 4 },
      { intent: 'toggle', gesture: 4 },
    ]);
  });

  it('a held line after the release changes nothing', () => {
    const { state, intents } = run([...tap(0, 100), { type: 'held', at: FN_HOLD_MS }]);
    expect(intents).toEqual(['prime', 'cancel-hold']);
    expect(state).toEqual({ kind: 'tapped', gesture: 1, upAt: 100 });
    // The double-tap still completes.
    expect(run(tap(350), state).intents).toEqual(['prime', 'toggle']);
    for (const start of [
      FN_GESTURE_IDLE,
      { kind: 'holding', gesture: 1 } as FnGestureState,
      { kind: 'spoiled', gesture: 1 } as FnGestureState,
    ]) {
      const result = nextFnGesture(start, { type: 'held', at: 5000 });
      expect(result.intents, start.kind).toEqual([]);
      expect(result.state).toEqual(start);
    }
  });

  it('a tap and then a hold is a hold', () => {
    const { intents } = run([
      ...tap(0, 100),
      { type: 'fn-down', at: 250 },
      { type: 'held', at: 250 + FN_HOLD_MS },
      { type: 'fn-up', at: 2000 },
    ]);
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'start-hold', 'end-hold']);
  });

  it('a press as long as a hold that ends before any held line still ends as a hold', () => {
    const { intents, state } = run([
      ...tap(0, 50),
      { type: 'fn-down', at: 200 },
      { type: 'fn-up', at: 200 + FN_HOLD_MS },
    ]);
    // It captured from fn down, so it is kept as a hold rather than dropped.
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'start-hold', 'end-hold']);
    expect(state.kind).toBe('idle');
  });

  it('fn with another key before the threshold discards the capture and never confirms it', () => {
    const { intents, state } = run([
      { type: 'fn-down', at: 0 },
      { type: 'other-key', at: 80 },
      { type: 'held', at: FN_HOLD_MS },
    ]);
    expect(intents).toEqual(['prime', 'cancel-hold']);
    expect(state.kind).toBe('spoiled');
    expect(run([{ type: 'fn-up', at: 6000 }], state).state.kind).toBe('idle');
  });

  it('fn with another key after a hold started cancels that recording', () => {
    const { intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'held', at: FN_HOLD_MS },
      { type: 'other-key', at: 900 },
      { type: 'fn-up', at: 1000 },
    ]);
    expect(intents).toEqual(['prime', 'start-hold', 'cancel-hold']);
  });

  it('fn with another key on the second press drops the double-tap', () => {
    const { intents, state } = run([
      ...tap(0, 100),
      { type: 'fn-down', at: 200 },
      { type: 'other-key', at: 250 },
      { type: 'fn-up', at: 300 },
    ]);
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'cancel-hold']);
    expect(state.kind).toBe('idle');
  });

  it('a spoiled press followed by a tap is not a double-tap', () => {
    const { intents } = run([
      { type: 'fn-down', at: 0 },
      { type: 'other-key', at: 50 },
      { type: 'fn-up', at: 100 },
      ...tap(200, 60),
    ]);
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'cancel-hold']);
  });

  it('a tap, another key and a tap within the gap is two taps, not a double-tap', () => {
    const { intents, state } = run([
      ...tap(0, 80),
      { type: 'other-key', at: 150 },
      ...tap(220, 80),
    ]);
    expect(intents).toEqual(['prime', 'cancel-hold', 'prime', 'cancel-hold']);
    expect(state.kind).toBe('tapped');
  });

  it('repeated downs and stray ups change nothing', () => {
    const { intents, state } = run([
      { type: 'fn-up', at: 0 },
      { type: 'fn-down', at: 10 },
      { type: 'fn-down', at: 20 },
    ]);
    expect(intents).toEqual(['prime']);
    expect(state).toEqual({ kind: 'pressed', gesture: 1, downAt: 10, second: false });
  });

  it('a reset ends a hold as its release would, discards a press, and forgets anything else', () => {
    expect(nextFnGesture({ kind: 'holding', gesture: 4 }, { type: 'reset' }).intents).toEqual([
      { intent: 'end-hold', gesture: 4 },
    ]);
    const pressed = nextFnGesture(
      { kind: 'pressed', gesture: 5, downAt: 0, second: true },
      { type: 'reset' }
    );
    expect(pressed.intents).toEqual([{ intent: 'cancel-hold', gesture: 5 }]);
    expect(pressed.state).toEqual({ kind: 'idle', gesture: 5 });
    for (const state of [
      FN_GESTURE_IDLE,
      { kind: 'tapped', gesture: 2, upAt: 0 } as FnGestureState,
      { kind: 'spoiled', gesture: 3 } as FnGestureState,
    ]) {
      const result = nextFnGesture(state, { type: 'reset' });
      expect(result.intents, state.kind).toEqual([]);
      expect(result.state).toEqual({ kind: 'idle', gesture: state.gesture });
    }
    // After a reset the wait for a second tap is gone, so the next press is a new gesture.
    expect(
      run(tap(10), nextFnGesture({ kind: 'tapped', gesture: 2, upAt: 0 }, { type: 'reset' }).state)
        .steps
    ).toEqual([
      { intent: 'prime', gesture: 3 },
      { intent: 'cancel-hold', gesture: 3 },
    ]);
  });
});

describe('the gesture driver', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const driven = () => {
    const seen: FnIntentEvent[] = [];
    const driver = new FnGestureDriver((intent, gesture) => seen.push({ intent, gesture }));
    return { driver, seen, intents: () => seen.map((s) => s.intent) };
  };

  it('arms no timer, so only the held line starts push to talk', () => {
    const { driver, intents } = driven();
    driver.feed({ type: 'fn-down', at: 0 });
    expect(intents()).toEqual(['prime']);
    expect(vi.getTimerCount()).toBe(0);
    // Main's clock runs on: nothing happens without the helper.
    vi.advanceTimersByTime(FN_HOLD_MS * 10);
    expect(intents()).toEqual(['prime']);
    driver.feed({ type: 'held', at: FN_HOLD_MS });
    expect(intents()).toEqual(['prime', 'start-hold']);
    driver.feed({ type: 'fn-up', at: 2000 });
    expect(intents()).toEqual(['prime', 'start-hold', 'end-hold']);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('a tap whose release is read after main’s clock passed the threshold is still a tap (row D)', () => {
    const { driver, intents } = driven();
    driver.feed({ type: 'fn-down', at: 1000 });
    // Main is busy from 1050 to 1350; the release written at 1090 is read only then.
    vi.advanceTimersByTime(350);
    driver.feed({ type: 'fn-up', at: 1090 });
    expect(intents()).toEqual(['prime', 'cancel-hold']);
    expect(driver.current).toEqual({ kind: 'tapped', gesture: 1, upAt: 1090 });
  });

  it('a double-tap whose four lines arrive after a 400 ms stall still toggles', () => {
    const { driver, seen } = driven();
    vi.advanceTimersByTime(400);
    for (const event of [...tap(1000), ...tap(1220)]) driver.feed(event);
    expect(seen).toEqual([
      { intent: 'prime', gesture: 1 },
      { intent: 'cancel-hold', gesture: 1 },
      { intent: 'prime', gesture: 1 },
      { intent: 'toggle', gesture: 1 },
    ]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('a single tap leaves nothing pending, and the next press after the gap is a new gesture', () => {
    const { driver, seen } = driven();
    for (const event of tap(0)) driver.feed(event);
    expect(vi.getTimerCount()).toBe(0);
    expect(driver.current.kind).toBe('tapped');
    for (const event of tap(90 + FN_DOUBLE_TAP_GAP_MS + 10)) driver.feed(event);
    expect(seen.map((s) => `${s.intent}:${s.gesture}`)).toEqual([
      'prime:1',
      'cancel-hold:1',
      'prime:2',
      'cancel-hold:2',
    ]);
  });

  it('dispose ends a hold', () => {
    const { driver, seen } = driven();
    driver.feed({ type: 'fn-down', at: 0 });
    driver.feed({ type: 'held', at: FN_HOLD_MS });
    driver.dispose();
    expect(seen).toEqual([
      { intent: 'prime', gesture: 1 },
      { intent: 'start-hold', gesture: 1 },
      { intent: 'end-hold', gesture: 1 },
    ]);
    expect(driver.current).toEqual({ kind: 'idle', gesture: 1 });
  });

  it('passes a one-argument handler only the intent', () => {
    const intents: FnIntent[] = [];
    const driver = new FnGestureDriver((intent) => intents.push(intent));
    for (const event of tap(0)) driver.feed(event);
    expect(intents).toEqual(['prime', 'cancel-hold']);
  });
});
