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
import type { FnKeyStatus } from './fnKeyHelper';

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
  /** Dictate to clipboard's phase. */
  anywherePhase: AnywherePhase;
  /** fn may dictate into other apps: the shortcut, Dictate into other apps and its fn row are on. */
  anywhereFnOn: boolean;
}

export type FnRoute = { kind: 'window'; windowId: number } | { kind: 'anywhere' } | null;

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
  if (world.anywhereFnOn) return { kind: 'anywhere' };
  return null;
}

/**
 * Where a double-tap goes. A running dictation is stopped wherever it runs, whichever app is in
 * front and whatever started it (the microphone button included); otherwise one starts where
 * {@link fnStartRoute} says.
 */
export function fnToggleRoute(world: FnWorld): FnRoute {
  if (world.recordingWindowId !== null)
    return { kind: 'window', windowId: world.recordingWindowId };
  if (anywhereRecording(world.anywherePhase)) return { kind: 'anywhere' };
  return fnStartRoute(world);
}
