/**
 * The fn key shortcut's setting and where each gesture goes (`fnGesture.ts` names the gesture;
 * this decides which dictation it acts on).
 *
 * The setting lives in `<config>/dictation/fn-key.json`, written only by Electron main, for the
 * same reason as `anywhere.json` (`dictationAnywhere.ts`): a keyboard listener is not something
 * `/config/upsert` or a renderer's storage should be able to turn on. It is on by default on
 * macOS; no other system lets an app see fn, so it is off and has no switch elsewhere.
 *
 * It also holds the dictation key (DD-101): Automatic (fn, and a modifier with fn, giving bare fn
 * up only while an app that also acts on fn runs), fn alone, or the modifier with fn, and which
 * modifier that is (Control unless the person chose Option or Shift). A file written before the
 * key existed reads as Automatic with Control.
 */
import type { AnywherePhase } from './dictationAnywhere';
import {
  DEFAULT_FN_MODIFIER,
  fnAccept,
  parseFnCombination,
  parseFnModifier,
  type FnAccept,
  type FnCombination,
  type FnIntent,
  type FnIntentEvent,
  type FnModifier,
} from './fnGesture';
import type { FnKeyStatus, FnLineTrace } from './fnKeyHelper';
import { KNOWN_FN_APPS, type KnownFnApp } from './fnCollision';

export interface FnKeySettings {
  /** The fn gestures dictate into Biorouter. */
  enabled: boolean;
  /**
   * They also dictate into other apps, through Dictate to clipboard (off by default). Used only
   * while `enabled` and **Dictate into other apps** are on.
   */
  otherApps: boolean;
  /** The dictation key: Automatic, fn alone, or the modifier with fn. Parse always fills it. */
  combination?: FnCombination;
  /** The modifier held with fn for Automatic and the modifier choice. Parse always fills it. */
  modifier?: FnModifier;
}

/**
 * Which key dictates now, as main computes it ({@link fnActive}) for Settings, the microphone's
 * tooltip and the one-time key notice. It says nothing about whether the shortcut is on or the
 * listener runs: {@link fnShortcutWorks} says that.
 */
export interface FnActive {
  /** fn alone dictates now. */
  bare: boolean;
  /** The modifier whose chord with fn dictates now, or null for none. */
  modifier: FnModifier | null;
  /**
   * Why: the person chose fn or the modifier (`chosen`), Automatic with no listed app running
   * (`automatic`), or Automatic giving bare fn up to a listed app (`collision`).
   */
  reason: 'chosen' | 'automatic' | 'collision';
  /** The listed app that also acts on fn and runs now, whatever the choice; null for none. */
  collidingApp: KnownFnApp | null;
}

/** What the renderer is told: the setting, the listener's state, and the clipboard switch. */
export interface FnKeyView {
  settings: FnKeySettings;
  status: FnKeyStatus;
  /** **Dictate into other apps** is on, so **Use fn in other apps** can apply. */
  anywhereOn: boolean;
  /** Which key dictates now ({@link fnActive}); missing before main computes it. */
  active?: FnActive;
}

export function defaultFnKeySettings(platform: string): FnKeySettings {
  return {
    enabled: platform === 'darwin',
    otherApps: false,
    combination: 'automatic',
    modifier: DEFAULT_FN_MODIFIER,
  };
}

export function parseFnKeySettings(raw: unknown, platform: string): FnKeySettings {
  if (platform !== 'darwin') {
    return {
      enabled: false,
      otherApps: false,
      combination: 'automatic',
      modifier: DEFAULT_FN_MODIFIER,
    };
  }
  if (!raw || typeof raw !== 'object') return defaultFnKeySettings(platform);
  const row = raw as Record<string, unknown>;
  return {
    enabled: typeof row.enabled === 'boolean' ? row.enabled : true,
    otherApps: row.otherApps === true,
    // Missing in every file written before the dictation key; Command and unknown words too.
    combination: parseFnCombination(row.combination),
    modifier: parseFnModifier(row.modifier),
  };
}

/**
 * Which key dictates now, from the setting and the listed app the helper saw running
 * (`FnKeyStatus.collidingApp`). Automatic takes fn and the modifier with fn, and bare fn only
 * while no listed app runs; the fn choice takes fn alone, and the modifier choice the modifier
 * alone. The colliding app is named whatever the choice, so Settings can say when fn is shared.
 */
export function fnActive(
  settings: FnKeySettings,
  collidingApp: KnownFnApp | null | undefined
): FnActive {
  const combination = parseFnCombination(settings.combination);
  const app = collidingApp ?? null;
  const accept: FnAccept = fnAccept(combination, parseFnModifier(settings.modifier), app !== null);
  return {
    bare: accept.bare,
    modifier: accept.modifier,
    reason: combination !== 'automatic' ? 'chosen' : app ? 'collision' : 'automatic',
    collidingApp: app,
  };
}

const KEY_LABELS: Record<FnModifier, string> = {
  control: 'Control+fn',
  option: 'Option+fn',
  shift: 'Shift+fn',
};

/** A dictation key as the interface names it: `fn`, or `Control+fn`, `Option+fn`, `Shift+fn`. */
export function fnKeyLabel(mod: FnModifier | null): string {
  return mod === null ? 'fn' : KEY_LABELS[mod];
}

const ARIA_KEYS: Record<FnModifier, string> = {
  control: 'Control+Fn',
  option: 'Alt+Fn',
  shift: 'Shift+Fn',
};

/**
 * A dictation key as `aria-keyshortcuts` spells it: the key names of the UI Events standard, so
 * Option is `Alt` there.
 */
export function fnKeyAriaShortcut(mod: FnModifier | null): string {
  return mod === null ? 'Fn' : ARIA_KEYS[mod];
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
      /** Which presses dictated when it came (`FnKeyService.accept`); left out when missing. */
      accept?: FnAccept;
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

const TRACE_MODIFIERS: readonly FnModifier[] = ['shift', 'control', 'option'];

/** A modifier for the trace: one of the three words, else `none`. */
function traceMod(mod: FnModifier | null | undefined): string {
  return mod && TRACE_MODIFIERS.includes(mod) ? mod : 'none';
}

/** A bundle id for the trace: only one on Biorouter's own list, else `none`. */
function traceKnown(bundle: string | null): string {
  return KNOWN_FN_APPS.find((app) => app.bundle === bundle)?.bundle ?? 'none';
}

/**
 * Which presses dictate, for the trace: `fn` for bare fn and the modifier's name for the
 * modifier with fn, joined by `+` (Automatic with no collision is `fn+control`), or `none`.
 */
function traceAccept(accept: FnAccept): string {
  const parts: string[] = [];
  if (accept.bare) parts.push('fn');
  if (accept.modifier) parts.push(traceMod(accept.modifier));
  return parts.length ? parts.join('+') : 'none';
}

/**
 * One line of the fn trace (`BIOROUTER_FN_TRACE=1`, unpackaged app only). It holds line types,
 * intent words, gesture ids, window ids, phases, times, the modifier held with fn, how many apps
 * watch modifier keys and which listed app is among them, and nothing else: there is no key code
 * to log, a view's id is left out, no unlisted app is named, and no text of any kind reaches it.
 */
export function formatFnTrace(record: FnTraceRecord): string {
  if (record.kind === 'line') {
    const parts = [
      'fn-trace line',
      `type=${record.line}`,
      `helper=${traceNumber(record.helperT)}`,
      `mapped=${traceNumber(record.mappedAt, 1)}`,
      `arrival=${traceNumber(record.arrivalAt, 1)}`,
      `timed=${record.timedBy ?? 'none'}`,
    ];
    if (record.mod !== undefined) parts.push(`mod=${traceMod(record.mod)}`);
    if (record.listeners) {
      parts.push(
        `count=${traceNumber(record.listeners.count)}`,
        `known=${traceKnown(record.listeners.known)}`
      );
    }
    return parts.join(' ');
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
    ...(record.accept ? [`accept=${traceAccept(record.accept)}`] : []),
  ].join(' ');
}
