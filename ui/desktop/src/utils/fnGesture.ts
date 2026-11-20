/**
 * The fn key's dictation gestures on macOS, as a pure reducer that Electron main feeds with what
 * the `biorouter-fnkey` helper reports (`fnKeyHelper.ts`):
 *
 * - **Hold fn** for {@link FN_HOLD_MS} or longer: push to talk. Capture starts the moment fn goes
 *   down (`prime`), so the first words are kept, the hold is confirmed at the threshold
 *   (`start-hold`), and dictation stops when fn is released.
 * - **Double-tap fn**: two presses each shorter than {@link FN_HOLD_MS}, the second within
 *   {@link FN_DOUBLE_TAP_GAP_MS} of the first release, with no other key between them. It starts
 *   hands-free dictation, or stops whatever dictation is running, so a second double-tap ends it.
 * - **fn with any other key or modifier** (fn and an arrow, Delete, an F key or a letter) drops
 *   the gesture and cancels what it captured. Another key after a tap ends the wait for a second
 *   tap, so fn, a letter, fn is two taps and not a double-tap.
 * - A single short tap does nothing: what its press captured is discarded (`cancel-hold`).
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

export type FnGestureState =
  | { kind: 'idle' }
  /** fn is down and has not yet been held long enough to be a hold. */
  | { kind: 'pressed'; downAt: number; second: boolean }
  /** A hold started push to talk; fn is still down. */
  | { kind: 'holding' }
  /** One short tap ended with no other key after it; a second press may follow. */
  | { kind: 'tapped'; upAt: number }
  /** fn was used with another key; nothing happens until it is released. */
  | { kind: 'spoiled' };

export type FnGestureEvent =
  | { type: 'fn-down'; at: number }
  | { type: 'fn-up'; at: number }
  | { type: 'other-key'; at: number }
  | { type: 'tick'; at: number }
  /** The listener stopped, or the shortcut was turned off: forget the gesture. */
  | { type: 'reset' };

export type FnIntent = 'prime' | 'start-hold' | 'end-hold' | 'cancel-hold' | 'toggle';

export interface FnGestureStep {
  state: FnGestureState;
  intents: FnIntent[];
  /** When to send the next `tick`, or null when no timer is needed. */
  deadline: number | null;
}

export const FN_GESTURE_IDLE: FnGestureState = { kind: 'idle' };

const step = (
  state: FnGestureState,
  intents: FnIntent[] = [],
  deadline: number | null = null
): FnGestureStep => ({ state, intents, deadline });

/** The pending deadline of `state`, for a step that leaves it unchanged. */
function deadlineOf(state: FnGestureState): number | null {
  if (state.kind === 'pressed') return state.downAt + FN_HOLD_MS;
  if (state.kind === 'tapped') return state.upAt + FN_DOUBLE_TAP_GAP_MS;
  return null;
}

const unchanged = (state: FnGestureState): FnGestureStep => step(state, [], deadlineOf(state));

export function nextFnGesture(state: FnGestureState, event: FnGestureEvent): FnGestureStep {
  if (event.type === 'reset') {
    // No release will come, so a hold ends as its release would have ended it, and a press that
    // was not yet a hold drops what it captured.
    if (state.kind === 'holding') return step(FN_GESTURE_IDLE, ['end-hold']);
    return step(FN_GESTURE_IDLE, state.kind === 'pressed' ? ['cancel-hold'] : []);
  }
  switch (state.kind) {
    case 'idle':
      if (event.type === 'fn-down') {
        const next: FnGestureState = { kind: 'pressed', downAt: event.at, second: false };
        return step(next, ['prime'], deadlineOf(next));
      }
      return unchanged(state);
    case 'tapped':
      if (event.type === 'fn-down') {
        const second = event.at - state.upAt <= FN_DOUBLE_TAP_GAP_MS;
        const next: FnGestureState = { kind: 'pressed', downAt: event.at, second };
        return step(next, ['prime'], deadlineOf(next));
      }
      // Typing after a tap: fn was used for something else, so the next tap is a first tap.
      if (event.type === 'other-key') return step(FN_GESTURE_IDLE);
      if (event.type === 'tick' && event.at >= state.upAt + FN_DOUBLE_TAP_GAP_MS) {
        return step(FN_GESTURE_IDLE);
      }
      return unchanged(state);
    case 'pressed':
      if (event.type === 'other-key') return step({ kind: 'spoiled' }, ['cancel-hold']);
      if (event.type === 'tick' && event.at >= state.downAt + FN_HOLD_MS) {
        return step({ kind: 'holding' }, ['start-hold']);
      }
      if (event.type === 'fn-up') {
        if (event.at - state.downAt >= FN_HOLD_MS) {
          // The tick was late; the press was a hold all the same, and it has been capturing
          // since fn went down, so it ends as a hold.
          return step(FN_GESTURE_IDLE, ['start-hold', 'end-hold']);
        }
        if (state.second) return step(FN_GESTURE_IDLE, ['toggle']);
        // A tap: what the press captured is discarded, and a second tap may follow.
        const next: FnGestureState = { kind: 'tapped', upAt: event.at };
        return step(next, ['cancel-hold'], deadlineOf(next));
      }
      return unchanged(state);
    case 'holding':
      if (event.type === 'fn-up') return step(FN_GESTURE_IDLE, ['end-hold']);
      if (event.type === 'other-key') return step({ kind: 'spoiled' }, ['cancel-hold']);
      return unchanged(state);
    case 'spoiled':
      if (event.type === 'fn-up') return step(FN_GESTURE_IDLE);
      return unchanged(state);
  }
}

/**
 * Runs {@link nextFnGesture} against a clock: it schedules each deadline's `tick`, and applies an
 * overdue one before the next event, so a late timer never turns a hold into a tap.
 */
export class FnGestureDriver {
  private state: FnGestureState = FN_GESTURE_IDLE;
  private deadline: number | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly onIntent: (intent: FnIntent) => void,
    private readonly now: () => number = () => performance.now()
  ) {}

  get current(): FnGestureState {
    return this.state;
  }

  feed(event: FnGestureEvent): void {
    if (event.type !== 'reset' && this.deadline !== null && event.at >= this.deadline) {
      this.apply({ type: 'tick', at: event.at });
    }
    this.apply(event);
  }

  dispose(): void {
    this.feed({ type: 'reset' });
  }

  private apply(event: FnGestureEvent): void {
    const result = nextFnGesture(this.state, event);
    this.state = result.state;
    this.schedule(result.deadline);
    for (const intent of result.intents) this.onIntent(intent);
  }

  private schedule(deadline: number | null): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.deadline = deadline;
    if (deadline === null) return;
    this.timer = setTimeout(
      () => {
        this.timer = null;
        if (this.deadline === deadline) this.apply({ type: 'tick', at: this.now() });
      },
      Math.max(0, deadline - this.now())
    );
  }
}
