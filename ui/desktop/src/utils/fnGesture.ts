/**
 * The fn key's dictation gestures on macOS, as a pure reducer that Electron main feeds with what
 * the `biorouter-fnkey` helper reports (`fnKeyHelper.ts`):
 *
 * - **Hold fn** for {@link FN_HOLD_MS} or longer: push to talk. Capture starts the moment fn goes
 *   down (`prime`), the hold is confirmed when the helper says the press is still down at the
 *   threshold (its `held` line, `start-hold`), and dictation stops when fn is released.
 * - **Double-tap fn**: two presses each shorter than {@link FN_HOLD_MS}, the second within
 *   {@link FN_DOUBLE_TAP_GAP_MS} of the first release, with no other key between them. It starts
 *   hands-free dictation, or stops whatever dictation is running, so a second double-tap ends it.
 * - **fn with any other key or modifier** (fn and an arrow, Delete, an F key or a letter) drops
 *   the gesture and cancels what it captured. Another key after a tap ends the wait for a second
 *   tap, so fn, a letter, fn is two taps and not a double-tap.
 * - A single short tap does nothing: what its press captured is discarded (`cancel-hold`).
 *
 * **The same gestures with a modifier** (DD-101). When another app also acts on fn (Wispr Flow
 * on the owner's Mac), Biorouter takes the same gestures with one of Shift, Control or Option held
 * with fn ({@link DEFAULT_FN_MODIFIER}, Control, unless the person chose another). Which presses
 * dictate is an {@link FnAccept} (from {@link fnAccept}): bare fn, the modifier with fn, or both.
 * The modifier may be pressed before fn (the helper's fn down names it) or within
 * {@link FN_CHORD_GRACE_MS} after it (its `chord` line), and may be let go at any time after that:
 * the hold ends when fn is released. A bare press when only the modifier dictates is `unclaimed`:
 * it does nothing unless the modifier joins it in time, and a bare hold left to the other app is
 * noted (`ignored-bare-hold`) so main can say once which key to use instead. A press the accept
 * does not take starts nothing at all: no capture, no microphone, no indicator.
 *
 * The reducer keeps no clock and the driver arms no timer. A hold is confirmed by the helper,
 * which times it on its own clock and writes its `held` line in order with the release, so a busy
 * main thread can delay the decision but never reorder it. The wait for a second tap ends lazily:
 * the next press is the second one only when its event time is close enough to the release.
 *
 * Every intent carries a **gesture id**: a number that grows when a first press starts, and that
 * the second press of a double-tap and its `toggle` keep. Main resolves where a gesture goes once,
 * at its first press, and reuses that for every later intent with the same id
 * (`planFnIntent` in `dictationFnKey.ts`).
 *
 * The reducer only names intents. What an intent does depends on what is running, which it does
 * not know: `prime` and `start-hold` do nothing while another dictation runs, `start-hold` keeps
 * a capture `prime` started, and `toggle` keeps the capture the second press started (hands-free)
 * or, when that press started none, stops a running dictation (one the microphone button started
 * included) and starts one otherwise. Main and the renderer make those calls (`main.ts`,
 * `components/dictation/fnGestureClient.ts`), and a caller that cannot start a capture silently at
 * fn down (no speech model yet, a permission prompt, a consent to give) leaves `prime` alone, so a
 * tap never opens anything.
 */

/** How long fn is held before it counts as a hold; a shorter press is a tap. */
export const FN_HOLD_MS = 300;
/** The longest wait from a tap's release to the second press of a double-tap. */
export const FN_DOUBLE_TAP_GAP_MS = 400;
/**
 * How long after fn goes down a modifier may join it and still make a chord press
 * (`FN_CHORD_GRACE_MS` in `crates/biorouter-fnkey/src/protocol.rs`, which times it).
 */
export const FN_CHORD_GRACE_MS = 100;

/** A modifier that may be held with fn. Command never is: macOS and every app use it. */
export type FnModifier = 'shift' | 'control' | 'option';
/** The person's choice of dictation key: Automatic, fn alone, or the modifier with fn. */
export type FnCombination = 'automatic' | 'fn' | 'modifier';

/** The modifier with fn that Automatic and the modifier choice use unless another is chosen. */
export const DEFAULT_FN_MODIFIER: FnModifier = 'control';

/** Which presses dictate now: fn alone (`bare`), and the modifier whose chord with fn does. */
export interface FnAccept {
  bare: boolean;
  modifier: FnModifier | null;
}

/**
 * Which presses dictate for a choice. Automatic takes fn and the modifier with fn, and gives up
 * bare fn only while an app that also acts on fn runs (`collision`), so the alternative works at
 * all times and a detection that flips never breaks the key in the person's hand. The fn choice
 * takes fn alone (a modifier with fn is left to other apps, as before this choice existed), and
 * the modifier choice takes only the modifier with fn.
 */
export function fnAccept(
  combination: FnCombination,
  modifier: FnModifier,
  collision: boolean
): FnAccept {
  switch (combination) {
    case 'fn':
      return { bare: true, modifier: null };
    case 'modifier':
      return { bare: false, modifier };
    case 'automatic':
    default:
      return { bare: !collision, modifier };
  }
}

/** Automatic with Control and no collision: every bare press behaves as in protocol version 2. */
export const DEFAULT_FN_ACCEPT: FnAccept = Object.freeze(
  fnAccept('automatic', DEFAULT_FN_MODIFIER, false)
);

export function isFnModifier(value: unknown): value is FnModifier {
  return value === 'shift' || value === 'control' || value === 'option';
}

/** A stored dictation key: one of the three words, and anything else (or nothing) Automatic. */
export function parseFnCombination(value: unknown): FnCombination {
  return value === 'fn' || value === 'modifier' ? value : 'automatic';
}

/** A stored modifier: Shift, Control or Option, and anything else (Command included) Control. */
export function parseFnModifier(value: unknown): FnModifier {
  return isFnModifier(value) ? value : DEFAULT_FN_MODIFIER;
}

/**
 * Where a gesture stands. `gesture` is the id of the gesture in progress, or in idle the id of
 * the last one, so the next first press takes the number after it.
 */
export type FnGestureState =
  | { kind: 'idle'; gesture: number }
  /**
   * fn is down and the helper has not yet confirmed a hold. `mod` is the modifier held with it,
   * present only when there is one (missing reads as none).
   */
  | { kind: 'pressed'; gesture: number; downAt: number; second: boolean; mod?: FnModifier | null }
  /** A hold started push to talk; fn is still down. `mod` as for `pressed`. */
  | { kind: 'holding'; gesture: number; mod?: FnModifier | null }
  /** One short tap ended with no other key after it; a second press may follow. */
  | { kind: 'tapped'; gesture: number; upAt: number }
  /**
   * fn went down alone while only the modifier with fn dictates: nothing has started, and the
   * press becomes a `pressed` one if the modifier joins it within {@link FN_CHORD_GRACE_MS}.
   */
  | { kind: 'unclaimed'; gesture: number; downAt: number; second: boolean }
  /** fn was used with another key; nothing happens until it is released. */
  | { kind: 'spoiled'; gesture: number };

export type FnGestureEvent =
  /** `mod`: the one modifier held with fn when it went down; missing or null for none. */
  | { type: 'fn-down'; at: number; mod?: FnModifier | null }
  | { type: 'fn-up'; at: number }
  /** The helper: the press now down has been down for {@link FN_HOLD_MS} with no other key. */
  | { type: 'held'; at: number }
  /** The helper: `mod` joined a press that went down alone, within {@link FN_CHORD_GRACE_MS}. */
  | { type: 'chord'; at: number; mod: FnModifier }
  | { type: 'other-key'; at: number }
  /** The listener stopped, or the shortcut was turned off: forget the gesture. */
  | { type: 'reset' };

export type FnIntent = 'prime' | 'start-hold' | 'end-hold' | 'cancel-hold' | 'toggle';

/** An intent and the gesture it belongs to. */
export type FnIntentEvent = { intent: FnIntent; gesture: number };

/** Something the reducer saw that is not an intent: a bare hold left to another app. */
export type FnNote = 'ignored-bare-hold';

export interface FnGestureStep {
  state: FnGestureState;
  intents: FnIntentEvent[];
  /** Present only when there is a note. */
  notes?: FnNote[];
}

export const FN_GESTURE_IDLE: FnGestureState = { kind: 'idle', gesture: 0 };

const step = (state: FnGestureState, intents: FnIntent[] = []): FnGestureStep => ({
  state,
  intents: intents.map((intent) => ({ intent, gesture: state.gesture })),
});

/** `mod` as a state field: present only when a modifier is held. */
const withMod = (mod: FnModifier | null | undefined): { mod?: FnModifier } => (mod ? { mod } : {});

/** Whether a press with `mod` held (null for none) dictates. */
const takes = (accept: FnAccept, mod: FnModifier | null): boolean =>
  mod === null ? accept.bare : accept.modifier === mod;

/**
 * fn went down. `gesture` is the id the press belongs to (the next one for a first press, the
 * same one for the second of a double-tap). A press the accept takes primes; a bare one that only
 * the modifier could still claim waits unclaimed; any other is spoiled and starts nothing.
 */
function press(
  gesture: number,
  at: number,
  mod: FnModifier | null,
  second: boolean,
  accept: FnAccept
): FnGestureStep {
  if (takes(accept, mod)) {
    return step({ kind: 'pressed', gesture, downAt: at, second, ...withMod(mod) }, ['prime']);
  }
  if (mod === null && accept.modifier !== null) {
    return step({ kind: 'unclaimed', gesture, downAt: at, second });
  }
  return step({ kind: 'spoiled', gesture });
}

/**
 * The next state and the intents it asks for. `accept` says which presses dictate; it is read at
 * each event, so a change while a press is down applies from that press's next event and fully
 * from the next press. The default, Automatic with Control and no collision, takes every bare
 * press, so a caller that never names a modifier gets the version 2 gestures exactly.
 */
export function nextFnGesture(
  state: FnGestureState,
  event: FnGestureEvent,
  accept: FnAccept = DEFAULT_FN_ACCEPT
): FnGestureStep {
  const idle: FnGestureState = { kind: 'idle', gesture: state.gesture };
  if (event.type === 'reset') {
    // No release will come, so a hold ends as its release would have ended it, and a press that
    // was not yet a hold drops what it captured.
    if (state.kind === 'holding') return step(idle, ['end-hold']);
    return step(idle, state.kind === 'pressed' ? ['cancel-hold'] : []);
  }
  switch (state.kind) {
    case 'idle':
      if (event.type === 'fn-down') {
        return press(state.gesture + 1, event.at, event.mod ?? null, false, accept);
      }
      return step(state);
    case 'tapped':
      if (event.type === 'fn-down') {
        const mod = event.mod ?? null;
        // The wait ends lazily: a press after the gap is a first press of a new gesture.
        if (event.at - state.upAt > FN_DOUBLE_TAP_GAP_MS) {
          return press(state.gesture + 1, event.at, mod, false, accept);
        }
        return press(state.gesture, event.at, mod, true, accept);
      }
      // Typing after a tap: fn was used for something else, so the next tap is a first tap.
      if (event.type === 'other-key') return step(idle);
      // A `held`, a chord or a stray release after the tap changes nothing.
      return step(state);
    case 'unclaimed':
      if (event.type === 'chord') {
        if (!takes(accept, event.mod)) return step({ kind: 'spoiled', gesture: state.gesture });
        // The modifier joined in time: the press is claimed now, and its hold is still timed
        // from fn down, as the helper times it.
        return step(
          {
            kind: 'pressed',
            gesture: state.gesture,
            downAt: state.downAt,
            second: state.second,
            mod: event.mod,
          },
          ['prime']
        );
      }
      if (event.type === 'other-key') return step({ kind: 'spoiled', gesture: state.gesture });
      if (event.type === 'held') {
        // A bare hold, which is the other app's: it starts nothing here, and main may say once
        // which key dictates instead.
        return {
          ...step({ kind: 'spoiled', gesture: state.gesture }),
          notes: ['ignored-bare-hold'],
        };
      }
      if (event.type === 'fn-up') return step(idle);
      return step(state);
    case 'pressed':
      if (event.type === 'chord') {
        // A modifier joined a bare press that already dictates: it is the same press if that
        // chord dictates too, and otherwise fn was used with another key.
        if (takes(accept, event.mod)) return step({ ...state, mod: event.mod });
        return step({ kind: 'spoiled', gesture: state.gesture }, ['cancel-hold']);
      }
      if (event.type === 'other-key') {
        return step({ kind: 'spoiled', gesture: state.gesture }, ['cancel-hold']);
      }
      if (event.type === 'held') {
        // A tap and then a hold is a hold, so this holds for the second press too.
        return step({ kind: 'holding', gesture: state.gesture, ...withMod(state.mod) }, [
          'start-hold',
        ]);
      }
      if (event.type === 'fn-up') {
        if (event.at - state.downAt >= FN_HOLD_MS) {
          // The press lasted a hold but no `held` came before its release: it has been
          // capturing since fn went down, so it ends as a hold.
          return step(idle, ['start-hold', 'end-hold']);
        }
        if (state.second) return step(idle, ['toggle']);
        // A tap: what the press captured is discarded, and a second tap may follow.
        return step({ kind: 'tapped', gesture: state.gesture, upAt: event.at }, ['cancel-hold']);
      }
      return step(state);
    case 'holding':
      if (event.type === 'fn-up') return step(idle, ['end-hold']);
      if (event.type === 'other-key') {
        return step({ kind: 'spoiled', gesture: state.gesture }, ['cancel-hold']);
      }
      return step(state);
    case 'spoiled':
      if (event.type === 'fn-up') return step(idle);
      return step(state);
  }
}

/**
 * Runs {@link nextFnGesture} over what the helper reports and passes each intent on with its
 * gesture id, and each note to `onNote`. It keeps the state and which presses dictate, and
 * nothing else: no clock and no timer.
 */
export class FnGestureDriver {
  private state: FnGestureState = FN_GESTURE_IDLE;
  private accept: FnAccept = DEFAULT_FN_ACCEPT;

  constructor(
    private readonly onIntent: (intent: FnIntent, gesture: number) => void,
    private readonly onNote?: (note: FnNote, gesture: number) => void
  ) {}

  get current(): FnGestureState {
    return this.state;
  }

  get accepting(): FnAccept {
    return this.accept;
  }

  /** Which presses dictate from the next event on. */
  setAccept(accept: FnAccept): void {
    this.accept = { bare: accept.bare, modifier: accept.modifier };
  }

  feed(event: FnGestureEvent): void {
    const result = nextFnGesture(this.state, event, this.accept);
    this.state = result.state;
    for (const { intent, gesture } of result.intents) this.onIntent(intent, gesture);
    for (const note of result.notes ?? []) this.onNote?.(note, result.state.gesture);
  }

  dispose(): void {
    this.feed({ type: 'reset' });
  }
}
