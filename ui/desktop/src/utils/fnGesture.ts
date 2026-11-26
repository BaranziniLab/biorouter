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
 * Where a gesture stands. `gesture` is the id of the gesture in progress, or in idle the id of
 * the last one, so the next first press takes the number after it.
 */
export type FnGestureState =
  | { kind: 'idle'; gesture: number }
  /** fn is down and the helper has not yet confirmed a hold. */
  | { kind: 'pressed'; gesture: number; downAt: number; second: boolean }
  /** A hold started push to talk; fn is still down. */
  | { kind: 'holding'; gesture: number }
  /** One short tap ended with no other key after it; a second press may follow. */
  | { kind: 'tapped'; gesture: number; upAt: number }
  /** fn was used with another key; nothing happens until it is released. */
  | { kind: 'spoiled'; gesture: number };

export type FnGestureEvent =
  | { type: 'fn-down'; at: number }
  | { type: 'fn-up'; at: number }
  /** The helper: the press now down has been down for {@link FN_HOLD_MS} with no other key. */
  | { type: 'held'; at: number }
  | { type: 'other-key'; at: number }
  /** The listener stopped, or the shortcut was turned off: forget the gesture. */
  | { type: 'reset' };

export type FnIntent = 'prime' | 'start-hold' | 'end-hold' | 'cancel-hold' | 'toggle';

/** An intent and the gesture it belongs to. */
export type FnIntentEvent = { intent: FnIntent; gesture: number };

export interface FnGestureStep {
  state: FnGestureState;
  intents: FnIntentEvent[];
}

export const FN_GESTURE_IDLE: FnGestureState = { kind: 'idle', gesture: 0 };

const step = (state: FnGestureState, intents: FnIntent[] = []): FnGestureStep => ({
  state,
  intents: intents.map((intent) => ({ intent, gesture: state.gesture })),
});

/** fn went down with no gesture to continue: a first press, with the next gesture id. */
function firstPress(previous: number, at: number): FnGestureStep {
  return step({ kind: 'pressed', gesture: previous + 1, downAt: at, second: false }, ['prime']);
}

export function nextFnGesture(state: FnGestureState, event: FnGestureEvent): FnGestureStep {
  const idle: FnGestureState = { kind: 'idle', gesture: state.gesture };
  if (event.type === 'reset') {
    // No release will come, so a hold ends as its release would have ended it, and a press that
    // was not yet a hold drops what it captured.
    if (state.kind === 'holding') return step(idle, ['end-hold']);
    return step(idle, state.kind === 'pressed' ? ['cancel-hold'] : []);
  }
  switch (state.kind) {
    case 'idle':
      if (event.type === 'fn-down') return firstPress(state.gesture, event.at);
      return step(state);
    case 'tapped':
      if (event.type === 'fn-down') {
        // The wait ends lazily: a press after the gap is a first press of a new gesture.
        if (event.at - state.upAt > FN_DOUBLE_TAP_GAP_MS) {
          return firstPress(state.gesture, event.at);
        }
        return step({ kind: 'pressed', gesture: state.gesture, downAt: event.at, second: true }, [
          'prime',
        ]);
      }
      // Typing after a tap: fn was used for something else, so the next tap is a first tap.
      if (event.type === 'other-key') return step(idle);
      // A `held` or a stray release after the tap changes nothing.
      return step(state);
    case 'pressed':
      if (event.type === 'other-key') {
        return step({ kind: 'spoiled', gesture: state.gesture }, ['cancel-hold']);
      }
      if (event.type === 'held') {
        // A tap and then a hold is a hold, so this holds for the second press too.
        return step({ kind: 'holding', gesture: state.gesture }, ['start-hold']);
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
 * gesture id. It keeps the state and nothing else: no clock and no timer.
 */
export class FnGestureDriver {
  private state: FnGestureState = FN_GESTURE_IDLE;

  constructor(private readonly onIntent: (intent: FnIntent, gesture: number) => void) {}

  get current(): FnGestureState {
    return this.state;
  }

  feed(event: FnGestureEvent): void {
    const result = nextFnGesture(this.state, event);
    this.state = result.state;
    for (const { intent, gesture } of result.intents) this.onIntent(intent, gesture);
  }

  dispose(): void {
    this.feed({ type: 'reset' });
  }
}
