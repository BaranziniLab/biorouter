import { describe, expect, it } from 'vitest';
import {
  defaultFnKeySettings,
  fnDictationRunning,
  fnGestureRoute,
  fnShortcutWorks,
  fnStartRoute,
  fnToggleRoute,
  formatFnTrace,
  parseFnKeySettings,
  planFnIntent,
  type FnAction,
  type FnLatch,
  type FnWorld,
} from './dictationFnKey';
import type { FnIntent } from './fnGesture';

const world = (over: Partial<FnWorld> = {}): FnWorld => ({
  focusedAppWindowId: null,
  recordingWindowId: null,
  anywherePhase: 'off',
  anywhereFnOn: false,
  ...over,
});

describe('the fn key setting', () => {
  it('is on by default on macOS and off, with no way on, elsewhere', () => {
    expect(defaultFnKeySettings('darwin')).toEqual({ enabled: true, otherApps: false });
    expect(defaultFnKeySettings('win32')).toEqual({ enabled: false, otherApps: false });
    expect(parseFnKeySettings({ enabled: true, otherApps: true }, 'linux')).toEqual({
      enabled: false,
      otherApps: false,
    });
    expect(parseFnKeySettings(null, 'darwin')).toEqual({ enabled: true, otherApps: false });
    expect(parseFnKeySettings({ enabled: false }, 'darwin')).toEqual({
      enabled: false,
      otherApps: false,
    });
    expect(parseFnKeySettings({ enabled: 'no', otherApps: 'yes' }, 'darwin')).toEqual({
      enabled: true,
      otherApps: false,
    });
    expect(parseFnKeySettings({ enabled: true, otherApps: true }, 'darwin')).toEqual({
      enabled: true,
      otherApps: true,
    });
  });

  it('works only on macOS, switched on, with the listener running', () => {
    const status = {
      supported: true,
      active: true,
      permission: 'granted' as const,
      listening: true,
      fnUsage: 0 as const,
      failed: false,
    };
    const view = { settings: { enabled: true, otherApps: false }, status, anywhereOn: false };
    expect(fnShortcutWorks(view)).toBe(true);
    expect(fnShortcutWorks(null)).toBe(false);
    expect(fnShortcutWorks({ ...view, settings: { enabled: false, otherApps: false } })).toBe(
      false
    );
    expect(fnShortcutWorks({ ...view, status: { ...status, listening: false } })).toBe(false);
    expect(fnShortcutWorks({ ...view, status: { ...status, supported: false } })).toBe(false);
  });
});

describe('where a gesture goes', () => {
  it('a new dictation goes to the Biorouter window in front', () => {
    expect(fnStartRoute(world({ focusedAppWindowId: 3, anywhereFnOn: true }))).toEqual({
      kind: 'window',
      windowId: 3,
    });
  });

  it('with another app in front it goes to Dictate to clipboard only when fn is allowed there', () => {
    expect(fnStartRoute(world())).toBeNull();
    expect(fnStartRoute(world({ anywhereFnOn: true, anywherePhase: 'idle' }))).toEqual({
      kind: 'anywhere',
      source: 'shortcut',
    });
  });

  it('a double-tap stops a running dictation wherever it runs, whatever is in front', () => {
    expect(fnToggleRoute(world({ focusedAppWindowId: 1, recordingWindowId: 2 }))).toEqual({
      kind: 'window',
      windowId: 2,
    });
    expect(
      fnToggleRoute(
        world({ focusedAppWindowId: 1, anywherePhase: 'recording', anywhereFnOn: true })
      )
    ).toEqual({ kind: 'anywhere', source: 'shortcut' });
    expect(fnToggleRoute(world({ anywherePhase: 'arming' }))).toEqual({
      kind: 'anywhere',
      source: 'shortcut',
    });
  });

  it('a double-tap with nothing running starts where a hold would', () => {
    expect(fnToggleRoute(world({ focusedAppWindowId: 5 }))).toEqual({
      kind: 'window',
      windowId: 5,
    });
    expect(fnToggleRoute(world({ anywherePhase: 'transcribing' }))).toBeNull();
  });

  it('a hold starts nothing while something records', () => {
    expect(fnDictationRunning(world())).toBe(false);
    expect(fnDictationRunning(world({ recordingWindowId: 1 }))).toBe(true);
    expect(fnDictationRunning(world({ anywherePhase: 'recording' }))).toBe(true);
    expect(fnDictationRunning(world({ anywherePhase: 'transcribing' }))).toBe(false);
  });
});

describe('the route a gesture latches', () => {
  it('a preview focused inside the window in front is its own route, whatever the row says', () => {
    const embedded = { windowId: 4, viewId: 'app-view-1' };
    expect(fnGestureRoute(world({ focusedAppWindowId: 4, focusedEmbedded: embedded }))).toEqual({
      kind: 'preview',
      windowId: 4,
      viewId: 'app-view-1',
    });
    expect(
      fnGestureRoute(
        world({ focusedAppWindowId: 4, focusedEmbedded: embedded, anywhereFnOn: true })
      )
    ).toEqual({ kind: 'preview', windowId: 4, viewId: 'app-view-1' });
    // A view in a window that is not in front is not focused, so it is no route.
    expect(fnGestureRoute(world({ focusedAppWindowId: 2, focusedEmbedded: embedded }))).toEqual({
      kind: 'window',
      windowId: 2,
    });
    expect(fnGestureRoute(world({ focusedEmbedded: embedded, anywhereFnOn: true }))).toEqual({
      kind: 'anywhere',
      source: 'shortcut',
    });
    expect(fnGestureRoute(world({ focusedEmbedded: embedded }))).toBeNull();
    expect(fnGestureRoute(world({ focusedAppWindowId: 1, focusedEmbedded: null }))).toEqual({
      kind: 'window',
      windowId: 1,
    });
  });
});

/** Runs one gesture's intents through the planner, each against its own world. */
function drive(
  steps: [FnIntent, Partial<FnWorld>][],
  gesture = 1,
  start: FnLatch | null = null
): { actions: FnAction[][]; latch: FnLatch | null } {
  let latch = start;
  const actions: FnAction[][] = [];
  for (const [intent, over] of steps) {
    const planned = planFnIntent({ intent, gesture }, latch, world(over));
    actions.push(planned.actions);
    latch = planned.latch;
  }
  return { actions, latch };
}

const send = (windowId: number, intent: FnIntent): FnAction => ({ kind: 'send', windowId, intent });
const anywhere = (op: 'prime' | 'start' | 'stop' | 'cancel'): FnAction => ({
  kind: 'anywhere',
  op,
  source: 'shortcut',
});
const preview = (op: 'prime' | 'start' | 'stop' | 'cancel'): FnAction => ({
  kind: 'preview',
  op,
  windowId: 4,
  viewId: 'app-view-1',
});

describe('planning a gesture', () => {
  it('sends the whole double-tap to the window in front at its first press, even with focus gone', () => {
    const { actions, latch } = drive([
      ['prime', { focusedAppWindowId: 3 }],
      // The first tap's Globe action took focus, and its own capture still reports.
      ['cancel-hold', { recordingWindowId: 3 }],
      ['prime', {}],
      ['toggle', { recordingWindowId: 3 }],
    ]);
    expect(actions).toEqual([
      [send(3, 'prime')],
      [send(3, 'cancel-hold')],
      [send(3, 'prime')],
      [send(3, 'toggle')],
    ]);
    expect(latch).toEqual({
      gesture: 1,
      route: { kind: 'window', windowId: 3 },
      captured: false,
    });
    // The window could not start quietly at the second press: the toggle still goes to it,
    // which starts there as the microphone would.
    expect(
      drive([
        ['prime', { focusedAppWindowId: 3 }],
        ['cancel-hold', {}],
        ['prime', {}],
        ['toggle', {}],
      ]).actions[3]
    ).toEqual([send(3, 'toggle')]);
  });

  it('keeps the other-apps route for the whole gesture when a Biorouter window takes focus', () => {
    const { actions } = drive([
      ['prime', { anywhereFnOn: true, anywherePhase: 'idle' }],
      ['cancel-hold', { focusedAppWindowId: 2, anywhereFnOn: true, anywherePhase: 'arming' }],
      ['prime', { focusedAppWindowId: 2, anywhereFnOn: true, anywherePhase: 'idle' }],
      ['toggle', { focusedAppWindowId: 2, anywhereFnOn: true, anywherePhase: 'arming' }],
    ]);
    expect(actions).toEqual([
      [anywhere('prime')],
      [anywhere('cancel')],
      [anywhere('prime')],
      // The second press's capture goes on, hands-free: kept, never stopped.
      [anywhere('start')],
    ]);
    // A hold there: captured at fn down, confirmed, then stopped and delivered.
    expect(
      drive([
        ['prime', { anywhereFnOn: true, anywherePhase: 'idle' }],
        ['start-hold', { focusedAppWindowId: 2, anywhereFnOn: true, anywherePhase: 'arming' }],
        ['end-hold', { focusedAppWindowId: 2, anywhereFnOn: true, anywherePhase: 'recording' }],
      ]).actions
    ).toEqual([[anywhere('prime')], [anywhere('start')], [anywhere('stop')]]);
  });

  it('a stale arming report at the second press still ends in a hands-free start', () => {
    for (const atToggle of [{ recordingWindowId: 3 }, {}]) {
      const { actions } = drive([
        ['prime', { focusedAppWindowId: 3 }],
        ['cancel-hold', { focusedAppWindowId: 3, recordingWindowId: 3 }],
        // The first tap's capture has not yet reported idle: nothing is primed.
        ['prime', { focusedAppWindowId: 3, recordingWindowId: 3 }],
        ['toggle', { focusedAppWindowId: 3, ...atToggle }],
      ]);
      expect(actions).toEqual([
        [send(3, 'prime')],
        [send(3, 'cancel-hold')],
        [],
        [send(3, 'toggle')],
      ]);
    }
  });

  it('the stop double-tap reaches the recording window from another app with the row off', () => {
    const recording = { recordingWindowId: 1 };
    const { actions } = drive([
      ['prime', recording],
      ['cancel-hold', recording],
      ['prime', recording],
      ['toggle', recording],
    ]);
    expect(actions).toEqual([[], [], [], [send(1, 'toggle')]]);
    // Hands-free Dictate to clipboard is stopped the same way, from a Biorouter window.
    expect(
      drive([
        ['prime', { focusedAppWindowId: 2, anywherePhase: 'recording' }],
        ['cancel-hold', { focusedAppWindowId: 2, anywherePhase: 'recording' }],
        ['prime', { focusedAppWindowId: 2, anywherePhase: 'recording' }],
        ['toggle', { focusedAppWindowId: 2, anywherePhase: 'recording' }],
      ]).actions
    ).toEqual([[], [], [], [anywhere('stop')]]);
  });

  it('a focused preview is its own route, for a hold and a double-tap', () => {
    const focused = {
      focusedAppWindowId: 4,
      focusedEmbedded: { windowId: 4, viewId: 'app-view-1' },
    };
    expect(
      drive([
        ['prime', focused],
        ['start-hold', { ...focused, anywherePhase: 'arming' }],
        ['end-hold', { ...focused, anywherePhase: 'recording' }],
      ]).actions
    ).toEqual([[preview('prime')], [preview('start')], [preview('stop')]]);
    // Its own capture is kept by the toggle, never stopped as a running dictation.
    const { actions, latch } = drive([
      ['prime', focused],
      ['cancel-hold', { ...focused, anywherePhase: 'arming' }],
      ['prime', {}],
      ['toggle', { anywherePhase: 'arming' }],
    ]);
    expect(actions).toEqual([
      [preview('prime')],
      [preview('cancel')],
      [preview('prime')],
      [preview('start')],
    ]);
    expect(latch?.route).toEqual({ kind: 'preview', windowId: 4, viewId: 'app-view-1' });
  });

  it('a new gesture resolves its route afresh, and the same gesture never does', () => {
    const first = drive([
      ['prime', { focusedAppWindowId: 3 }],
      ['cancel-hold', {}],
    ]);
    // Gesture 1's second press, with another app in front and the row on: still window 3.
    expect(
      planFnIntent({ intent: 'prime', gesture: 1 }, first.latch, world({ anywhereFnOn: true }))
        .actions
    ).toEqual([send(3, 'prime')]);
    // Gesture 2 starts where things are now.
    const second = planFnIntent(
      { intent: 'prime', gesture: 2 },
      first.latch,
      world({ anywhereFnOn: true })
    );
    expect(second.actions).toEqual([anywhere('prime')]);
    expect(second.latch).toEqual({
      gesture: 2,
      route: { kind: 'anywhere', source: 'shortcut' },
      captured: true,
    });
  });

  it('a hold while a dictation runs does nothing, and a release reaches only what its press captured', () => {
    const recording = { focusedAppWindowId: 1, recordingWindowId: 1 };
    expect(
      drive([
        ['prime', recording],
        ['start-hold', recording],
        ['end-hold', recording],
      ]).actions
    ).toEqual([[], [], []]);
    expect(
      drive([
        ['prime', { anywherePhase: 'recording', anywhereFnOn: true }],
        ['cancel-hold', { anywherePhase: 'recording', anywhereFnOn: true }],
      ]).actions
    ).toEqual([[], []]);
  });

  it('a hold whose press found something running starts at the threshold once it has stopped', () => {
    expect(
      drive([
        ['prime', { focusedAppWindowId: 2, recordingWindowId: 2 }],
        ['start-hold', { focusedAppWindowId: 2 }],
        ['end-hold', { focusedAppWindowId: 2, recordingWindowId: 2 }],
      ]).actions
    ).toEqual([[], [send(2, 'start-hold')], [send(2, 'end-hold')]]);
  });

  it('with no route and nothing running, a gesture does nothing', () => {
    expect(
      drive([
        ['prime', {}],
        ['cancel-hold', {}],
        ['prime', {}],
        ['toggle', {}],
      ]).actions
    ).toEqual([[], [], [], []]);
    expect(
      drive([
        ['prime', {}],
        ['start-hold', {}],
        ['end-hold', {}],
      ]).actions
    ).toEqual([[], [], []]);
  });

  it('a latch with no captured field reads as not captured', () => {
    const latch: FnLatch = { gesture: 7, route: { kind: 'window', windowId: 5 } };
    expect(planFnIntent({ intent: 'end-hold', gesture: 7 }, latch, world()).actions).toEqual([]);
    expect(planFnIntent({ intent: 'start-hold', gesture: 7 }, latch, world()).actions).toEqual([
      send(5, 'start-hold'),
    ]);
  });
});

describe('the fn trace', () => {
  it('names a helper line by type and times only', () => {
    expect(
      formatFnTrace({
        kind: 'line',
        line: 'fn-down',
        helperT: 40150,
        mappedAt: 1150.25,
        arrivalAt: 1200.5,
        timedBy: 'helper',
      })
    ).toBe('fn-trace line type=fn-down helper=40150 mapped=1150.3 arrival=1200.5 timed=helper');
    expect(
      formatFnTrace({
        kind: 'line',
        line: 'unreadable',
        helperT: null,
        mappedAt: null,
        arrivalAt: 9,
        timedBy: null,
      })
    ).toBe('fn-trace line type=unreadable helper=none mapped=none arrival=9.0 timed=none');
  });

  it('names an intent with its gesture, route, focus and actions, and never a view’s id', () => {
    const line = formatFnTrace({
      kind: 'intent',
      intent: 'toggle',
      gesture: 12,
      route: { kind: 'preview', windowId: 4, viewId: 'secret-view-id' },
      world: world({
        focusedAppWindowId: 4,
        focusedEmbedded: { windowId: 4, viewId: 'secret-view-id' },
        anywherePhase: 'arming',
      }),
      actions: [preview('start')],
      at: 2000,
    });
    expect(line).toBe(
      'fn-trace intent intent=toggle gesture=12 at=2000.0 route=preview:4 focus=window:4 embedded=window:4 recording=none anywhere=arming actions=preview:start:4'
    );
    expect(line).not.toContain('secret');
    expect(
      formatFnTrace({
        kind: 'intent',
        intent: 'prime',
        gesture: 1,
        route: null,
        world: world({ recordingWindowId: 2 }),
        actions: [],
        at: 1,
      })
    ).toBe(
      'fn-trace intent intent=prime gesture=1 at=1.0 route=none focus=none embedded=none recording=window:2 anywhere=off actions=none'
    );
    expect(
      formatFnTrace({
        kind: 'intent',
        intent: 'cancel-hold',
        gesture: 3,
        route: { kind: 'anywhere', source: 'shortcut' },
        world: world({ anywherePhase: 'bogus' as never }),
        actions: [anywhere('cancel'), send(1, 'cancel-hold')],
        at: 3,
      })
    ).toBe(
      'fn-trace intent intent=cancel-hold gesture=3 at=3.0 route=anywhere focus=none embedded=none recording=none anywhere=unknown actions=anywhere:cancel,send:cancel-hold:1'
    );
  });
});
