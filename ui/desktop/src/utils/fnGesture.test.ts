import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DEFAULT_FN_ACCEPT,
  DEFAULT_FN_MODIFIER,
  FN_CHORD_GRACE_MS,
  FN_DOUBLE_TAP_GAP_MS,
  FN_GESTURE_IDLE,
  FN_HOLD_MS,
  FnGestureDriver,
  fnAccept,
  nextFnGesture,
  parseFnCombination,
  parseFnModifier,
  type FnAccept,
  type FnGestureEvent,
  type FnGestureState,
  type FnIntent,
  type FnIntentEvent,
  type FnModifier,
  type FnNote,
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

// ── Protocol version 3: a modifier with fn (DD-101) ──────────────────────────────

/** Which presses dictate, as `fnAccept` answers for each choice. */
const AUTO: FnAccept = { bare: true, modifier: 'control' };
const AUTO_COLLISION: FnAccept = { bare: false, modifier: 'control' };
const FN_ONLY: FnAccept = { bare: true, modifier: null };
const CONTROL_ONLY: FnAccept = { bare: false, modifier: 'control' };

/** Runs `events` through the reducer under `accept`, and returns every intent and note. */
function runWith(
  accept: FnAccept,
  events: FnGestureEvent[],
  start: FnGestureState = FN_GESTURE_IDLE
) {
  let state = start;
  const steps: FnIntentEvent[] = [];
  const notes: FnNote[] = [];
  for (const event of events) {
    const result = nextFnGesture(state, event, accept);
    state = result.state;
    steps.push(...result.intents);
    notes.push(...(result.notes ?? []));
  }
  return { state, intents: steps.map((s) => s.intent), steps, notes };
}

const down = (at: number, mod: FnModifier | null = null): FnGestureEvent =>
  mod === null ? { type: 'fn-down', at } : { type: 'fn-down', at, mod };
const up = (at: number): FnGestureEvent => ({ type: 'fn-up', at });
const chord = (at: number, mod: FnModifier): FnGestureEvent => ({ type: 'chord', at, mod });
const held = (at: number): FnGestureEvent => ({ type: 'held', at });
/** A short press, with the modifier held from before fn went down. */
const modTap = (at: number, mod: FnModifier | null, length = 90): FnGestureEvent[] => [
  down(at, mod),
  up(at + length),
];
/** A hold: fn down, the helper's held line at the threshold, the release later. */
const modHold = (at: number, mod: FnModifier | null, length = 1000): FnGestureEvent[] => [
  down(at, mod),
  held(at + FN_HOLD_MS),
  up(at + length),
];

describe('which presses dictate', () => {
  it('Automatic takes fn and its modifier with fn, and gives up bare fn only during a collision', () => {
    expect(fnAccept('automatic', 'control', false)).toEqual({ bare: true, modifier: 'control' });
    expect(fnAccept('automatic', 'option', false)).toEqual({ bare: true, modifier: 'option' });
    expect(fnAccept('automatic', 'shift', true)).toEqual({ bare: false, modifier: 'shift' });
  });

  it('fn takes only bare fn, and the modifier choice only its modifier, collision or not', () => {
    for (const collision of [false, true]) {
      expect(fnAccept('fn', 'option', collision)).toEqual({ bare: true, modifier: null });
      expect(fnAccept('modifier', 'option', collision)).toEqual({
        bare: false,
        modifier: 'option',
      });
    }
  });

  it('defaults to Automatic with Control and no collision, which is version 2 for every bare press', () => {
    expect(DEFAULT_FN_MODIFIER).toBe('control');
    expect(FN_CHORD_GRACE_MS).toBe(100);
    expect(DEFAULT_FN_ACCEPT).toEqual(fnAccept('automatic', DEFAULT_FN_MODIFIER, false));
    expect(DEFAULT_FN_ACCEPT).toEqual(AUTO);
  });

  it('reads a stored choice and modifier, Command and anything unknown as the defaults', () => {
    expect(parseFnCombination('automatic')).toBe('automatic');
    expect(parseFnCombination('fn')).toBe('fn');
    expect(parseFnCombination('modifier')).toBe('modifier');
    for (const word of [undefined, null, '', 'Automatic', 'control', 'chord', 1, {}]) {
      expect(parseFnCombination(word), String(word)).toBe('automatic');
    }
    expect(parseFnModifier('control')).toBe('control');
    expect(parseFnModifier('option')).toBe('option');
    expect(parseFnModifier('shift')).toBe('shift');
    for (const word of [undefined, null, '', 'command', 'cmd', 'Control', 'alt', 'fn', 0, []]) {
      expect(parseFnModifier(word), String(word)).toBe('control');
    }
  });
});

describe('the reducer with a modifier (design 5.4)', () => {
  it('idle: an accepted fn down is a pressed first press, and primes', () => {
    const bare = nextFnGesture(FN_GESTURE_IDLE, down(10), AUTO);
    expect(bare.intents).toEqual([{ intent: 'prime', gesture: 1 }]);
    expect(bare.state).toEqual({ kind: 'pressed', gesture: 1, downAt: 10, second: false });
    for (const accept of [AUTO, AUTO_COLLISION, CONTROL_ONLY]) {
      const withMod = nextFnGesture({ kind: 'idle', gesture: 4 }, down(10, 'control'), accept);
      expect(withMod.intents).toEqual([{ intent: 'prime', gesture: 5 }]);
      expect(withMod.state).toEqual({
        kind: 'pressed',
        gesture: 5,
        downAt: 10,
        second: false,
        mod: 'control',
      });
    }
  });

  it('idle: a bare fn down when only the modifier dictates is unclaimed, and does nothing', () => {
    for (const accept of [AUTO_COLLISION, CONTROL_ONLY]) {
      const result = nextFnGesture({ kind: 'idle', gesture: 2 }, down(10), accept);
      expect(result.intents).toEqual([]);
      expect(result.notes).toBeUndefined();
      expect(result.state).toEqual({ kind: 'unclaimed', gesture: 3, downAt: 10, second: false });
    }
  });

  it('idle: any other fn down is spoiled, and does nothing', () => {
    for (const [accept, event] of [
      [FN_ONLY, down(10, 'control')],
      [AUTO, down(10, 'option')],
      [AUTO_COLLISION, down(10, 'shift')],
      [CONTROL_ONLY, down(10, 'option')],
      [{ bare: false, modifier: null }, down(10)],
    ] as const) {
      const result = nextFnGesture(FN_GESTURE_IDLE, event, accept);
      expect(result.intents, JSON.stringify({ accept, event })).toEqual([]);
      expect(result.state.kind).toBe('spoiled');
      // Nothing happens until fn is released, and then the next press is a press again.
      const after = runWith(accept, [held(310), up(400)], result.state);
      expect(after.intents).toEqual([]);
      expect(after.state.kind).toBe('idle');
    }
  });

  it('tapped after the gap: the same three answers, each a new gesture', () => {
    const tapped: FnGestureState = { kind: 'tapped', gesture: 6, upAt: 100 };
    const late = 100 + FN_DOUBLE_TAP_GAP_MS + 1;
    const accepted = nextFnGesture(tapped, down(late, 'control'), AUTO_COLLISION);
    expect(accepted.intents).toEqual([{ intent: 'prime', gesture: 7 }]);
    expect(accepted.state).toMatchObject({ kind: 'pressed', gesture: 7, second: false });
    const unclaimed = nextFnGesture(tapped, down(late), AUTO_COLLISION);
    expect(unclaimed.intents).toEqual([]);
    expect(unclaimed.state).toEqual({ kind: 'unclaimed', gesture: 7, downAt: late, second: false });
    const spoiled = nextFnGesture(tapped, down(late, 'option'), AUTO_COLLISION);
    expect(spoiled.intents).toEqual([]);
    expect(spoiled.state.kind).toBe('spoiled');
  });

  it('tapped within the gap: the same three answers, each the second press of the same gesture', () => {
    const tapped: FnGestureState = { kind: 'tapped', gesture: 6, upAt: 100 };
    const soon = 100 + FN_DOUBLE_TAP_GAP_MS;
    const accepted = nextFnGesture(tapped, down(soon, 'control'), AUTO_COLLISION);
    expect(accepted.intents).toEqual([{ intent: 'prime', gesture: 6 }]);
    expect(accepted.state).toEqual({
      kind: 'pressed',
      gesture: 6,
      downAt: soon,
      second: true,
      mod: 'control',
    });
    const unclaimed = nextFnGesture(tapped, down(soon), AUTO_COLLISION);
    expect(unclaimed.intents).toEqual([]);
    expect(unclaimed.state).toEqual({ kind: 'unclaimed', gesture: 6, downAt: soon, second: true });
    const spoiled = nextFnGesture(tapped, down(soon, 'shift'), AUTO_COLLISION);
    expect(spoiled.intents).toEqual([]);
    expect(spoiled.state).toEqual({ kind: 'spoiled', gesture: 6 });
  });

  it('unclaimed: an accepted chord makes it a pressed press, keeping second and the fn down time', () => {
    for (const second of [false, true]) {
      const unclaimed: FnGestureState = { kind: 'unclaimed', gesture: 3, downAt: 50, second };
      const result = nextFnGesture(unclaimed, chord(90, 'control'), AUTO_COLLISION);
      expect(result.intents).toEqual([{ intent: 'prime', gesture: 3 }]);
      expect(result.state).toEqual({
        kind: 'pressed',
        gesture: 3,
        downAt: 50,
        second,
        mod: 'control',
      });
    }
  });

  it('unclaimed: a chord not accepted, or another key, spoils it without a word', () => {
    const unclaimed: FnGestureState = { kind: 'unclaimed', gesture: 3, downAt: 50, second: false };
    for (const event of [chord(90, 'option'), chord(90, 'shift'), { type: 'other-key', at: 90 }]) {
      const result = nextFnGesture(unclaimed, event as FnGestureEvent, AUTO_COLLISION);
      expect(result.intents).toEqual([]);
      expect(result.notes).toBeUndefined();
      expect(result.state).toEqual({ kind: 'spoiled', gesture: 3 });
    }
  });

  it('unclaimed: a held line is a bare hold left to another app, noted once', () => {
    const unclaimed: FnGestureState = { kind: 'unclaimed', gesture: 3, downAt: 50, second: false };
    const result = nextFnGesture(unclaimed, held(50 + FN_HOLD_MS), AUTO_COLLISION);
    expect(result.intents).toEqual([]);
    expect(result.notes).toEqual(['ignored-bare-hold']);
    expect(result.state).toEqual({ kind: 'spoiled', gesture: 3 });
  });

  it('unclaimed: the release goes back to idle, and nothing waits for a second tap', () => {
    const unclaimed: FnGestureState = { kind: 'unclaimed', gesture: 3, downAt: 50, second: true };
    const result = nextFnGesture(unclaimed, up(140), AUTO_COLLISION);
    expect(result.intents).toEqual([]);
    expect(result.state).toEqual({ kind: 'idle', gesture: 3 });
  });

  it('unclaimed: a repeated down changes nothing, and a reset forgets it without a cancel', () => {
    const unclaimed: FnGestureState = { kind: 'unclaimed', gesture: 3, downAt: 50, second: false };
    expect(nextFnGesture(unclaimed, down(60), AUTO_COLLISION)).toEqual({
      state: unclaimed,
      intents: [],
    });
    expect(nextFnGesture(unclaimed, { type: 'reset' }, AUTO_COLLISION)).toEqual({
      state: { kind: 'idle', gesture: 3 },
      intents: [],
    });
  });

  it('pressed: an accepted chord takes the modifier and asks nothing new', () => {
    const pressed: FnGestureState = { kind: 'pressed', gesture: 2, downAt: 0, second: false };
    const result = nextFnGesture(pressed, chord(40, 'control'), AUTO);
    expect(result.intents).toEqual([]);
    expect(result.state).toEqual({ ...pressed, mod: 'control' });
    // The hold it becomes carries the modifier too.
    expect(nextFnGesture(result.state, held(FN_HOLD_MS), AUTO).state).toEqual({
      kind: 'holding',
      gesture: 2,
      mod: 'control',
    });
  });

  it('pressed: a chord not accepted spoils the press and cancels what it captured', () => {
    const pressed: FnGestureState = { kind: 'pressed', gesture: 2, downAt: 0, second: false };
    for (const [accept, mod] of [
      [FN_ONLY, 'control'],
      [AUTO, 'option'],
      [AUTO, 'shift'],
    ] as const) {
      const result = nextFnGesture(pressed, chord(40, mod), accept);
      expect(result.intents).toEqual([{ intent: 'cancel-hold', gesture: 2 }]);
      expect(result.state).toEqual({ kind: 'spoiled', gesture: 2 });
    }
  });

  it('a chord anywhere else changes nothing', () => {
    for (const state of [
      FN_GESTURE_IDLE,
      { kind: 'holding', gesture: 1 } as FnGestureState,
      { kind: 'tapped', gesture: 1, upAt: 0 } as FnGestureState,
      { kind: 'spoiled', gesture: 1 } as FnGestureState,
    ]) {
      for (const accept of [AUTO, FN_ONLY, AUTO_COLLISION]) {
        const result = nextFnGesture(state, chord(10, 'control'), accept);
        expect(result.intents, state.kind).toEqual([]);
        expect(result.state).toEqual(state);
      }
    }
  });

  it('the default accept leaves every version 2 sequence unchanged', () => {
    // Each version 2 sequence with the intents version 2 gave it (the tests above pin them).
    const sequences: [FnGestureEvent[], FnIntent[]][] = [
      [tap(0, 120), ['prime', 'cancel-hold']],
      [
        [...tap(0, 100), down(500), up(600)],
        ['prime', 'cancel-hold', 'prime', 'toggle'],
      ],
      [
        [...tap(0, 100), down(100 + FN_DOUBLE_TAP_GAP_MS + 1), up(600)],
        ['prime', 'cancel-hold', 'prime', 'cancel-hold'],
      ],
      [
        [...tap(0), ...tap(200), ...modHold(5000, null)],
        ['prime', 'cancel-hold', 'prime', 'toggle', 'prime', 'start-hold', 'end-hold'],
      ],
      [
        [down(0), { type: 'other-key', at: 80 }, held(FN_HOLD_MS), up(400)],
        ['prime', 'cancel-hold'],
      ],
      [
        [down(0), held(FN_HOLD_MS), { type: 'other-key', at: 900 }, up(1000)],
        ['prime', 'start-hold', 'cancel-hold'],
      ],
      [
        [...tap(0, 80), { type: 'other-key', at: 150 }, ...tap(220, 80)],
        ['prime', 'cancel-hold', 'prime', 'cancel-hold'],
      ],
      [
        [...tap(0, 50), down(200), up(200 + FN_HOLD_MS)],
        ['prime', 'cancel-hold', 'prime', 'start-hold', 'end-hold'],
      ],
      [
        [up(0), down(10), down(20), { type: 'reset' }],
        ['prime', 'cancel-hold'],
      ],
    ];
    for (const [events, expected] of sequences) {
      const implicit = run(events);
      const explicit = runWith(DEFAULT_FN_ACCEPT, events);
      expect(implicit.intents, JSON.stringify(events)).toEqual(expected);
      expect(explicit.steps).toEqual(implicit.steps);
      expect(explicit.state).toEqual(implicit.state);
      expect(explicit.notes).toEqual([]);
    }
  });
});

describe('the gestures with a modifier', () => {
  it('Automatic with no collision: fn and Control+fn holds and double-taps all dictate', () => {
    for (const mod of [null, 'control'] as const) {
      expect(runWith(AUTO, modHold(0, mod)).intents, String(mod)).toEqual([
        'prime',
        'start-hold',
        'end-hold',
      ]);
      expect(runWith(AUTO, [...modTap(0, mod), ...modTap(220, mod)]).intents).toEqual([
        'prime',
        'cancel-hold',
        'prime',
        'toggle',
      ]);
    }
    // Control joining a bare press within the grace is the same hold.
    expect(
      runWith(AUTO, [down(0), chord(40, 'control'), held(FN_HOLD_MS), up(900)]).intents
    ).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('Automatic with a collision: a bare hold starts nothing and is noted, while Control+fn works', () => {
    const bare = runWith(AUTO_COLLISION, modHold(0, null));
    expect(bare.intents).toEqual([]);
    expect(bare.notes).toEqual(['ignored-bare-hold']);
    expect(bare.state.kind).toBe('idle');
    // A bare double-tap is left to the other app too, and says nothing.
    const bareTaps = runWith(AUTO_COLLISION, [...modTap(0, null), ...modTap(220, null)]);
    expect(bareTaps.intents).toEqual([]);
    expect(bareTaps.notes).toEqual([]);
    expect(runWith(AUTO_COLLISION, modHold(5000, 'control')).intents).toEqual([
      'prime',
      'start-hold',
      'end-hold',
    ]);
    expect(
      runWith(AUTO_COLLISION, [...modTap(0, 'control'), ...modTap(220, 'control')]).intents
    ).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
    // Control may also join the bare press within the grace.
    const joined = runWith(AUTO_COLLISION, [down(0), chord(60, 'control'), held(300), up(800)]);
    expect(joined.intents).toEqual(['prime', 'start-hold', 'end-hold']);
    expect(joined.notes).toEqual([]);
  });

  it('the fn choice: a Control+fn press is spoiled and never primed, and a chord cancels a bare press', () => {
    const withControl = runWith(FN_ONLY, modHold(0, 'control'));
    expect(withControl.intents).toEqual([]);
    expect(withControl.state.kind).toBe('idle');
    // Control joining a bare press: the bare press primed, and the chord cancels it.
    const joined = runWith(FN_ONLY, [down(1000), chord(1040, 'control'), held(1300), up(2000)]);
    expect(joined.intents).toEqual(['prime', 'cancel-hold']);
    expect(joined.state.kind).toBe('idle');
    // Bare fn still dictates.
    expect(runWith(FN_ONLY, modHold(5000, null)).intents).toEqual([
      'prime',
      'start-hold',
      'end-hold',
    ]);
  });

  it('the modifier choice: bare fn is left alone and only its own modifier is taken', () => {
    const bare = runWith(CONTROL_ONLY, modHold(0, null));
    expect(bare.intents).toEqual([]);
    expect(bare.notes).toEqual(['ignored-bare-hold']);
    for (const other of ['option', 'shift'] as const) {
      const result = runWith(CONTROL_ONLY, modHold(0, other));
      expect(result.intents, other).toEqual([]);
      expect(result.notes).toEqual([]);
    }
    expect(runWith(CONTROL_ONLY, modHold(0, 'control')).intents).toEqual([
      'prime',
      'start-hold',
      'end-hold',
    ]);
    const option: FnAccept = fnAccept('modifier', 'option', false);
    expect(runWith(option, modHold(0, 'option')).intents).toEqual([
      'prime',
      'start-hold',
      'end-hold',
    ]);
    expect(runWith(option, modHold(0, 'control')).intents).toEqual([]);
  });

  it('the modifier choice: a chord after a bare fn down primes at the chord', () => {
    const { steps, state } = runWith(CONTROL_ONLY, [down(0), chord(70, 'control')]);
    expect(steps).toEqual([{ intent: 'prime', gesture: 1 }]);
    expect(state).toEqual({
      kind: 'pressed',
      gesture: 1,
      downAt: 0,
      second: false,
      mod: 'control',
    });
    // The hold is timed from fn down, as the helper times it.
    expect(runWith(CONTROL_ONLY, [held(FN_HOLD_MS), up(1200)], state).intents).toEqual([
      'start-hold',
      'end-hold',
    ]);
    // A press as long as a hold with no held line still ends as a hold, from fn down.
    expect(runWith(CONTROL_ONLY, [up(FN_HOLD_MS)], state).intents).toEqual([
      'start-hold',
      'end-hold',
    ]);
  });

  it('one gesture id runs from the first press to the toggle, across a chord', () => {
    const { steps } = runWith(AUTO_COLLISION, [
      ...modTap(0, 'control'),
      // Control was let go between the taps and joins the second press after fn.
      down(220),
      chord(250, 'control'),
      up(310),
      // The next gesture takes the next id.
      ...modHold(2000, 'control'),
    ]);
    expect(steps).toEqual([
      { intent: 'prime', gesture: 1 },
      { intent: 'cancel-hold', gesture: 1 },
      { intent: 'prime', gesture: 1 },
      { intent: 'toggle', gesture: 1 },
      { intent: 'prime', gesture: 2 },
      { intent: 'start-hold', gesture: 2 },
      { intent: 'end-hold', gesture: 2 },
    ]);
  });

  it('a double-tap mixing a bare tap and a Control+fn tap toggles in Automatic, and not in the modifier choice', () => {
    const bareFirst = [...modTap(0, null), ...modTap(220, 'control')];
    const controlFirst = [...modTap(0, 'control'), ...modTap(220, null)];
    for (const events of [bareFirst, controlFirst]) {
      expect(runWith(AUTO, events).intents).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
      const chosen = runWith(CONTROL_ONLY, events);
      expect(chosen.intents).not.toContain('toggle');
      expect(chosen.state.kind).not.toBe('pressed');
    }
    expect(runWith(CONTROL_ONLY, bareFirst).intents).toEqual(['prime', 'cancel-hold']);
    expect(runWith(CONTROL_ONLY, controlFirst).intents).toEqual(['prime', 'cancel-hold']);
  });

  it('a double-tap with Control held throughout, or let go and pressed again, toggles', () => {
    // The helper writes nothing for Control between the taps (R7), so both reach the reducer as
    // two Control+fn presses; the second may also go down bare and take Control as a chord.
    for (const events of [
      [...modTap(0, 'control'), ...modTap(220, 'control')],
      [...modTap(0, 'control'), down(220), chord(260, 'control'), up(310)],
    ]) {
      expect(runWith(AUTO_COLLISION, events).intents).toEqual([
        'prime',
        'cancel-hold',
        'prime',
        'toggle',
      ]);
    }
  });

  it('the hold goes on when the modifier is let go, and ends when fn is released', () => {
    // A released modifier writes nothing (R5), so a hold is a hold until fn comes up.
    const { intents } = runWith(CONTROL_ONLY, [down(0, 'control'), held(300), up(5000)]);
    expect(intents).toEqual(['prime', 'start-hold', 'end-hold']);
  });

  it('another key during a Control+fn hold cancels it, as with bare fn', () => {
    expect(
      runWith(CONTROL_ONLY, [
        down(0, 'control'),
        held(300),
        { type: 'other-key', at: 600 },
        up(900),
      ]).intents
    ).toEqual(['prime', 'start-hold', 'cancel-hold']);
  });
});

describe('the gesture driver with keys', () => {
  it('reads the accept at each event, so a change applies from the next event', () => {
    const seen: FnIntentEvent[] = [];
    const notes: Array<{ note: FnNote; gesture: number }> = [];
    const driver = new FnGestureDriver(
      (intent, gesture) => seen.push({ intent, gesture }),
      (note, gesture) => notes.push({ note, gesture })
    );
    // The default is Automatic with no collision: bare fn dictates.
    driver.feed(down(0));
    expect(seen).toEqual([{ intent: 'prime', gesture: 1 }]);
    // A collision appears while fn is down: the press it already claimed goes on.
    driver.setAccept(AUTO_COLLISION);
    driver.feed(up(90));
    expect(seen.map((s) => s.intent)).toEqual(['prime', 'cancel-hold']);
    // The next bare press is left to the other app; its hold is noted with its gesture.
    for (const event of modHold(5000, null)) driver.feed(event);
    expect(seen.map((s) => s.intent)).toEqual(['prime', 'cancel-hold']);
    expect(notes).toEqual([{ note: 'ignored-bare-hold', gesture: 2 }]);
    // Back to no collision: bare fn dictates again.
    driver.setAccept(AUTO);
    for (const event of modHold(9000, null)) driver.feed(event);
    expect(seen.slice(2)).toEqual([
      { intent: 'prime', gesture: 3 },
      { intent: 'start-hold', gesture: 3 },
      { intent: 'end-hold', gesture: 3 },
    ]);
  });

  it('a driver with no note handler still drops a bare hold quietly', () => {
    const intents: FnIntent[] = [];
    const driver = new FnGestureDriver((intent) => intents.push(intent));
    driver.setAccept(CONTROL_ONLY);
    for (const event of modHold(0, null)) driver.feed(event);
    expect(intents).toEqual([]);
    expect(driver.current.kind).toBe('idle');
  });
});
