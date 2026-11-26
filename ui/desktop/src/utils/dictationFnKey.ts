/**
 * The fn key shortcut's setting and where each gesture goes (`fnGesture.ts` names the gesture;
 * this decides which dictation it acts on).
 *
 * The setting lives in `<config>/dictation/fn-key.json`, written only by Electron main, for the
 * same reason as `anywhere.json` (`dictationAnywhere.ts`): a keyboard listener is not something
 * `/config/upsert` or a renderer's storage should be able to turn on. It is on by default on
 * macOS; no other system lets an app see fn, so it is off and has no switch elsewhere.
 */
import type { AnywherePhase } from './dictationAnywhere';
import type { FnIntent, FnIntentEvent } from './fnGesture';
import type { FnKeyStatus, FnLineTrace } from './fnKeyHelper';

export interface FnKeySettings {
  /** The fn gestures dictate into Biorouter. */
  enabled: boolean;
  /**
   * They also dictate into other apps, through Dictate to clipboard (off by default). Used only
   * while `enabled` and **Dictate into other apps** are on.
   */
  otherApps: boolean;
}

/** What the renderer is told: the setting, the listener's state, and the clipboard switch. */
export interface FnKeyView {
  settings: FnKeySettings;
  status: FnKeyStatus;
  /** **Dictate into other apps** is on, so **Use fn in other apps** can apply. */
  anywhereOn: boolean;
}

export function defaultFnKeySettings(platform: string): FnKeySettings {
  return { enabled: platform === 'darwin', otherApps: false };
}

export function parseFnKeySettings(raw: unknown, platform: string): FnKeySettings {
  if (platform !== 'darwin') return { enabled: false, otherApps: false };
  if (!raw || typeof raw !== 'object') return defaultFnKeySettings(platform);
  const row = raw as Record<string, unknown>;
  return {
    enabled: typeof row.enabled === 'boolean' ? row.enabled : true,
    otherApps: row.otherApps === true,
  };
}

/** Whether the shortcut works now: macOS, on, and the listener running. */
export function fnShortcutWorks(view: FnKeyView | null): boolean {
  return Boolean(view?.status.supported && view.settings.enabled && view.status.listening);
}

/** What main knows when a gesture arrives. */
export interface FnWorld {
  /** A Biorouter window has keyboard focus (the app is in front). */
  focusedAppWindowId: number | null;
  /** A Biorouter window whose dictation arms or records, from its `dictation:indicator` reports. */
  recordingWindowId: number | null;
  /** Dictate to clipboard's phase (it also runs a managed app preview's dictation). */
  anywherePhase: AnywherePhase;
  /** fn may dictate into other apps: the shortcut, Dictate into other apps and its fn row are on. */
  anywhereFnOn: boolean;
  /**
   * A managed app preview or in-app browser view that has focus inside the focused window, which
   * dictates through its own path (public terms) whatever **Use fn in other apps** says. Absent or
   * null when none does.
   */
  focusedEmbedded?: { windowId: number; viewId: string } | null;
}

/** Where a gesture goes: a Biorouter window, Dictate to clipboard, or a focused preview. */
export type FnRoute =
  | { kind: 'window'; windowId: number }
  | { kind: 'anywhere'; source: 'shortcut' }
  | { kind: 'preview'; windowId: number; viewId: string }
  | null;

const anywhereRecording = (phase: AnywherePhase): boolean =>
  phase === 'arming' || phase === 'recording';

/** Something already records, so a hold starts nothing. */
export function fnDictationRunning(world: FnWorld): boolean {
  return world.recordingWindowId !== null || anywhereRecording(world.anywherePhase);
}

/**
 * Where a new dictation from fn goes: the Biorouter window in front, or, with Biorouter behind
 * another app, Dictate to clipboard when fn is allowed there. Null when neither applies.
 */
export function fnStartRoute(world: FnWorld): FnRoute {
  if (world.focusedAppWindowId !== null)
    return { kind: 'window', windowId: world.focusedAppWindowId };
  if (world.anywhereFnOn) return { kind: 'anywhere', source: 'shortcut' };
  return null;
}

/**
 * Where a gesture goes, decided once at its first press: a preview focused inside the window in
 * front is its own route, and otherwise {@link fnStartRoute} decides.
 */
export function fnGestureRoute(world: FnWorld): FnRoute {
  const embedded = world.focusedEmbedded;
  if (
    embedded &&
    world.focusedAppWindowId !== null &&
    embedded.windowId === world.focusedAppWindowId
  ) {
    return { kind: 'preview', windowId: embedded.windowId, viewId: embedded.viewId };
  }
  return fnStartRoute(world);
}

/**
 * Where a double-tap goes. A running dictation is stopped wherever it runs, whichever app is in
 * front and whatever started it (the microphone button included); otherwise one starts where
 * {@link fnStartRoute} says.
 */
export function fnToggleRoute(world: FnWorld): FnRoute {
  if (world.recordingWindowId !== null)
    return { kind: 'window', windowId: world.recordingWindowId };
  if (anywhereRecording(world.anywherePhase)) return { kind: 'anywhere', source: 'shortcut' };
  return fnStartRoute(world);
}

/**
 * One gesture's route, latched at its first press and reused by every later intent with the same
 * gesture id, so a focus change during a double-tap (macOS's own fn action can take it) does not
 * lose the second press. `captured` says the press now down, or just released, asked the route to
 * capture (it replaces main's old `fnHold`); its release, a tap or another key reaches that route
 * and no other. Missing reads as false.
 */
export interface FnLatch {
  gesture: number;
  route: FnRoute;
  captured?: boolean;
}

/**
 * What main does for an intent:
 *
 * - `send`: pass the intent word to that window (`fnGestureClient.ts` decides there, by the
 *   capture its own press started).
 * - `anywhere` and `preview`, through Dictate to clipboard (a preview dictation runs on the same
 *   machine, for that window's view):
 *   - `prime`: start capturing at fn down, only when that asks nothing first, and say nothing
 *     about a refusal while the press may still be a tap;
 *   - `start`: this gesture wants a dictation running there: keep the one its press primed (and
 *     stop being quiet about it), or start one;
 *   - `stop`: stop the running dictation and deliver what it heard; a stop starts nothing, so no
 *     switch gates it;
 *   - `cancel`: discard what this gesture's press captured there.
 */
export type FnAction =
  | { kind: 'send'; windowId: number; intent: FnIntent }
  | { kind: 'anywhere'; op: 'prime' | 'start' | 'stop' | 'cancel'; source: 'shortcut' }
  | {
      kind: 'preview';
      op: 'prime' | 'start' | 'stop' | 'cancel';
      windowId: number;
      viewId: string;
    };

const OP_FOR: Record<FnIntent, 'prime' | 'start' | 'stop' | 'cancel'> = {
  prime: 'prime',
  'start-hold': 'start',
  'end-hold': 'stop',
  'cancel-hold': 'cancel',
  toggle: 'start',
};

/** The action that carries `intent` to `route`. */
function actionAt(route: NonNullable<FnRoute>, intent: FnIntent): FnAction {
  switch (route.kind) {
    case 'window':
      return { kind: 'send', windowId: route.windowId, intent };
    case 'anywhere':
      return { kind: 'anywhere', op: OP_FOR[intent], source: 'shortcut' };
    case 'preview':
      return {
        kind: 'preview',
        op: OP_FOR[intent],
        windowId: route.windowId,
        viewId: route.viewId,
      };
  }
}

/**
 * Where one fn intent goes, and the latch for the intents after it. Pure: main passes the latch
 * it got back last time and what it knows now, and carries out the actions.
 *
 * - A new gesture id resolves its route afresh ({@link fnGestureRoute}); the same id reuses the
 *   latched one, whatever has focus now.
 * - `prime` starts a capture at the route unless something already records.
 * - `start-hold` confirms what the press captured, or starts one if nothing records.
 * - `end-hold` and `cancel-hold` reach the route only when the press captured there.
 * - `toggle` keeps the second press's capture hands-free; otherwise it stops whatever records,
 *   wherever it runs (a window keeps a capture of its own press there, `fnGestureClient.ts`), and
 *   otherwise starts at the latched route.
 */
export function planFnIntent(
  event: FnIntentEvent,
  latch: FnLatch | null,
  world: FnWorld
): { actions: FnAction[]; latch: FnLatch | null } {
  const current: FnLatch =
    latch && latch.gesture === event.gesture
      ? latch
      : { gesture: event.gesture, route: fnGestureRoute(world), captured: false };
  const route = current.route;
  const captured = current.captured === true && route !== null;
  const next = (nowCaptured: boolean): FnLatch => ({
    gesture: current.gesture,
    route,
    captured: nowCaptured,
  });
  const plan = (actions: FnAction[], nowCaptured: boolean) => ({
    actions,
    latch: next(nowCaptured),
  });
  switch (event.intent) {
    case 'prime':
      // A first press, or the second of a double-tap (the first's capture ended at its release).
      if (route === null || fnDictationRunning(world)) return plan([], false);
      return plan([actionAt(route, 'prime')], true);
    case 'start-hold':
      if (captured) return plan([actionAt(route, 'start-hold')], true);
      if (route === null || fnDictationRunning(world)) return plan([], false);
      return plan([actionAt(route, 'start-hold')], true);
    case 'end-hold':
    case 'cancel-hold':
      if (!captured) return plan([], false);
      return plan([actionAt(route, event.intent)], false);
    case 'toggle': {
      // The second press's capture at Dictate to clipboard or a preview goes on, hands-free.
      if (captured && route.kind !== 'window') return plan([actionAt(route, 'toggle')], false);
      if (world.recordingWindowId !== null) {
        return plan([{ kind: 'send', windowId: world.recordingWindowId, intent: 'toggle' }], false);
      }
      if (anywhereRecording(world.anywherePhase)) {
        return plan([{ kind: 'anywhere', op: 'stop', source: 'shortcut' }], false);
      }
      if (route === null) return plan([], false);
      return plan([actionAt(route, 'toggle')], false);
    }
  }
}

/** One fn trace record: a helper line as the service read it, or an intent as main planned it. */
export type FnTraceRecord =
  | ({ kind: 'line' } & FnLineTrace)
  | {
      kind: 'intent';
      intent: FnIntent;
      gesture: number;
      /** The route the gesture's latch holds after this intent. */
      route: FnRoute;
      world: FnWorld;
      actions: FnAction[];
      /** When main handled the intent, on its clock. */
      at: number;
    };

/** A time or id for the trace: a finite number, else `none`. */
function traceNumber(value: number | null | undefined, digits = 0): string {
  return typeof value === 'number' && Number.isFinite(value) ? value.toFixed(digits) : 'none';
}

function traceRoute(route: FnRoute): string {
  if (route === null) return 'none';
  if (route.kind === 'anywhere') return 'anywhere';
  // A view's id is the renderer's string, so the trace names only its window.
  return `${route.kind}:${traceNumber(route.windowId)}`;
}

function traceAction(action: FnAction): string {
  if (action.kind === 'send') return `send:${action.intent}:${traceNumber(action.windowId)}`;
  if (action.kind === 'anywhere') return `anywhere:${action.op}`;
  return `preview:${action.op}:${traceNumber(action.windowId)}`;
}

const PHASES: readonly AnywherePhase[] = [
  'off',
  'idle',
  'arming',
  'recording',
  'transcribing',
  'review',
];

/**
 * One line of the fn trace (`BIOROUTER_FN_TRACE=1`, unpackaged app only). It holds line types,
 * intent words, gesture ids, window ids, phases and times, and nothing else: there is no key code
 * to log, a view's id is left out, and no text of any kind reaches it.
 */
export function formatFnTrace(record: FnTraceRecord): string {
  if (record.kind === 'line') {
    return [
      'fn-trace line',
      `type=${record.line}`,
      `helper=${traceNumber(record.helperT)}`,
      `mapped=${traceNumber(record.mappedAt, 1)}`,
      `arrival=${traceNumber(record.arrivalAt, 1)}`,
      `timed=${record.timedBy ?? 'none'}`,
    ].join(' ');
  }
  const { world } = record;
  const phase = PHASES.includes(world.anywherePhase) ? world.anywherePhase : 'unknown';
  return [
    'fn-trace intent',
    `intent=${record.intent}`,
    `gesture=${traceNumber(record.gesture)}`,
    `at=${traceNumber(record.at, 1)}`,
    `route=${traceRoute(record.route)}`,
    `focus=${world.focusedAppWindowId === null ? 'none' : `window:${traceNumber(world.focusedAppWindowId)}`}`,
    `embedded=${world.focusedEmbedded ? `window:${traceNumber(world.focusedEmbedded.windowId)}` : 'none'}`,
    `recording=${world.recordingWindowId === null ? 'none' : `window:${traceNumber(world.recordingWindowId)}`}`,
    `anywhere=${phase}`,
    `actions=${record.actions.length ? record.actions.map(traceAction).join(',') : 'none'}`,
  ].join(' ');
}
