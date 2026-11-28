import { describe, expect, it } from 'vitest';
import {
  defaultFnKeySettings,
  fnActive,
  fnDictationRunning,
  fnGestureRoute,
  fnKeyAriaShortcut,
  fnKeyLabel,
  fnShortcutWorks,
  fnStartRoute,
  fnToggleRoute,
  formatFnTrace,
  parseFnKeySettings,
  planFnIntent,
  type FnAction,
  type FnKeySettings,
  type FnLatch,
  type FnWorld,
} from './dictationFnKey';
import type { FnIntent } from './fnGesture';
import { KNOWN_FN_APPS } from './fnCollision';

const world = (over: Partial<FnWorld> = {}): FnWorld => ({
  focusedAppWindowId: null,
  recordingWindowId: null,
  anywherePhase: 'off',
  anywhereFnOn: false,
  ...over,
});

/**
 * The dictation key fields every parsed setting carries since DD-101: Automatic with Option.
 * Option since the owner's ruling O-49 (2026-11-27); it was Control under O-46.
 */
const KEYS = { combination: 'automatic', modifier: 'option' } as const;

describe('the fn key setting', () => {
  // Rewritten for DD-101: the setting gained the dictation key and the modifier held with fn,
  // which parse always fills (Automatic and Option unless the file says otherwise; Control until
  // the owner's ruling O-49 of 2026-11-27 made Option the default).
  it('is on by default on macOS and off, with no way on, elsewhere', () => {
    expect(defaultFnKeySettings('darwin')).toEqual({ enabled: true, otherApps: false, ...KEYS });
    expect(defaultFnKeySettings('win32')).toEqual({ enabled: false, otherApps: false, ...KEYS });
    expect(
      parseFnKeySettings(
        { enabled: true, otherApps: true, combination: 'modifier', modifier: 'shift' },
        'linux'
      )
    ).toEqual({ enabled: false, otherApps: false, ...KEYS });
    expect(parseFnKeySettings(null, 'darwin')).toEqual({
      enabled: true,
      otherApps: false,
      ...KEYS,
    });
    expect(parseFnKeySettings({ enabled: false }, 'darwin')).toEqual({
      enabled: false,
      otherApps: false,
      ...KEYS,
    });
    expect(parseFnKeySettings({ enabled: 'no', otherApps: 'yes' }, 'darwin')).toEqual({
      enabled: true,
      otherApps: false,
      ...KEYS,
    });
    expect(parseFnKeySettings({ enabled: true, otherApps: true }, 'darwin')).toEqual({
      enabled: true,
      otherApps: true,
      ...KEYS,
    });
  });

  // Rewritten for O-49 (2026-11-27): the default modifier is Option, no longer Control (O-46).
  it('a file written before the dictation key reads as Automatic with Option', () => {
    expect(parseFnKeySettings({ enabled: true, otherApps: false }, 'darwin')).toEqual({
      enabled: true,
      otherApps: false,
      combination: 'automatic',
      modifier: 'option',
    });
  });

  it('a file that chose Control keeps Control', () => {
    expect(
      parseFnKeySettings({ enabled: true, otherApps: false, modifier: 'control' }, 'darwin')
    ).toEqual({ enabled: true, otherApps: false, combination: 'automatic', modifier: 'control' });
  });

  it('reads each dictation key and modifier it knows, and anything else as the defaults', () => {
    for (const combination of ['automatic', 'fn', 'modifier'] as const) {
      for (const modifier of ['control', 'option', 'shift'] as const) {
        expect(
          parseFnKeySettings({ enabled: true, otherApps: false, combination, modifier }, 'darwin')
        ).toEqual({ enabled: true, otherApps: false, combination, modifier });
      }
    }
    // Command is never a choice: macOS and every app use it for shortcuts. Anything else reads
    // as the default, Option since O-49 (Control under O-46).
    for (const modifier of ['command', 'cmd', 'Command', 'fn', 'alt', '', 3, null, ['control']]) {
      expect(parseFnKeySettings({ modifier }, 'darwin').modifier, String(modifier)).toBe('option');
    }
    for (const combination of ['Automatic', 'control+fn', 'bare', '', 1, null, {}]) {
      expect(parseFnKeySettings({ combination }, 'darwin').combination, String(combination)).toBe(
        'automatic'
      );
    }
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

describe('which key works now', () => {
  // Control chosen, so a row that names the modifier shows a choice; the default is Option since
  // the owner's ruling O-49 (2026-11-27), pinned by the setting with no dictation key fields below.
  const settings = (over: Partial<FnKeySettings> = {}): FnKeySettings => ({
    enabled: true,
    otherApps: false,
    combination: 'automatic',
    modifier: 'control',
    ...over,
  });
  const wispr = KNOWN_FN_APPS[0];

  it('Automatic with nothing listed running: fn, and Control+fn too', () => {
    expect(fnActive(settings(), null)).toEqual({
      bare: true,
      modifier: 'control',
      reason: 'automatic',
      collidingApp: null,
    });
    expect(fnActive(settings(), undefined)).toEqual(fnActive(settings(), null));
  });

  it('Automatic with a listed app running: only the modifier, because of that app', () => {
    expect(fnActive(settings({ modifier: 'option' }), wispr)).toEqual({
      bare: false,
      modifier: 'option',
      reason: 'collision',
      collidingApp: wispr,
    });
  });

  it('the fn choice: fn alone, with or without a listed app, which is still named', () => {
    expect(fnActive(settings({ combination: 'fn' }), null)).toEqual({
      bare: true,
      modifier: null,
      reason: 'chosen',
      collidingApp: null,
    });
    expect(fnActive(settings({ combination: 'fn' }), wispr)).toEqual({
      bare: true,
      modifier: null,
      reason: 'chosen',
      collidingApp: wispr,
    });
  });

  it('the modifier choice: the modifier alone, with or without a listed app', () => {
    for (const app of [null, wispr]) {
      expect(fnActive(settings({ combination: 'modifier', modifier: 'shift' }), app)).toEqual({
        bare: false,
        modifier: 'shift',
        reason: 'chosen',
        collidingApp: app,
      });
    }
  });

  // Rewritten for O-49 (2026-11-27): the default modifier is Option, no longer Control (O-46).
  it('a setting with no dictation key fields reads as Automatic with Option', () => {
    const option = settings({ modifier: 'option' });
    expect(fnActive({ enabled: true, otherApps: false }, null)).toEqual(fnActive(option, null));
    expect(fnActive({ enabled: true, otherApps: false }, null).modifier).toBe('option');
    expect(
      fnActive({ enabled: true, otherApps: false, modifier: 'command' as never }, wispr)
    ).toEqual(fnActive(option, wispr));
  });

  it('names each key as the interface shows it', () => {
    expect(fnKeyLabel(null)).toBe('fn');
    expect(fnKeyLabel('control')).toBe('Control+fn');
    expect(fnKeyLabel('option')).toBe('Option+fn');
    expect(fnKeyLabel('shift')).toBe('Shift+fn');
  });

  it('names each key as aria-keyshortcuts spells it (Option is Alt there)', () => {
    expect(fnKeyAriaShortcut(null)).toBe('Fn');
    expect(fnKeyAriaShortcut('control')).toBe('Control+Fn');
    expect(fnKeyAriaShortcut('option')).toBe('Alt+Fn');
    expect(fnKeyAriaShortcut('shift')).toBe('Shift+Fn');
  });
});

describe('the fn trace in version 3', () => {
  it('names the modifier of an fn down or chord line, none for a bare press', () => {
    const base = { helperT: 40, mappedAt: 1, arrivalAt: 2, timedBy: 'helper' as const };
    expect(formatFnTrace({ kind: 'line', line: 'fn-down', ...base, mod: null })).toBe(
      'fn-trace line type=fn-down helper=40 mapped=1.0 arrival=2.0 timed=helper mod=none'
    );
    expect(formatFnTrace({ kind: 'line', line: 'fn-down', ...base, mod: 'control' })).toBe(
      'fn-trace line type=fn-down helper=40 mapped=1.0 arrival=2.0 timed=helper mod=control'
    );
    expect(formatFnTrace({ kind: 'line', line: 'chord', ...base, mod: 'option' })).toBe(
      'fn-trace line type=chord helper=40 mapped=1.0 arrival=2.0 timed=helper mod=option'
    );
  });

  it('names a listeners line by its count and a listed app only, never another id', () => {
    const line = (listeners: { count: number; known: string | null }) =>
      formatFnTrace({
        kind: 'line',
        line: 'listeners',
        helperT: 9,
        mappedAt: null,
        arrivalAt: 3,
        timedBy: null,
        listeners,
      });
    expect(line({ count: 2, known: 'com.electron.wispr-flow' })).toBe(
      'fn-trace line type=listeners helper=9 mapped=none arrival=3.0 timed=none count=2 known=com.electron.wispr-flow'
    );
    expect(line({ count: 0, known: null })).toBe(
      'fn-trace line type=listeners helper=9 mapped=none arrival=3.0 timed=none count=0 known=none'
    );
    // A known name that is not a listed bundle never reaches the trace.
    expect(line({ count: 1, known: 'com.example.secret' })).toBe(
      'fn-trace line type=listeners helper=9 mapped=none arrival=3.0 timed=none count=1 known=none'
    );
  });

  it('names a restart by type only', () => {
    expect(
      formatFnTrace({
        kind: 'line',
        line: 'restart',
        helperT: null,
        mappedAt: null,
        arrivalAt: 7,
        timedBy: null,
      })
    ).toBe('fn-trace line type=restart helper=none mapped=none arrival=7.0 timed=none');
  });

  it('names which presses dictate on an intent record', () => {
    const intent = (accept: { bare: boolean; modifier: 'control' | 'option' | 'shift' | null }) =>
      formatFnTrace({
        kind: 'intent',
        intent: 'prime',
        gesture: 1,
        route: { kind: 'window', windowId: 2 },
        world: world({ focusedAppWindowId: 2 }),
        actions: [send(2, 'prime')],
        at: 5,
        accept,
      });
    const head =
      'fn-trace intent intent=prime gesture=1 at=5.0 route=window:2 focus=window:2 embedded=none recording=none anywhere=off actions=send:prime:2';
    expect(intent({ bare: true, modifier: 'control' })).toBe(`${head} accept=fn+control`);
    expect(intent({ bare: false, modifier: 'option' })).toBe(`${head} accept=option`);
    expect(intent({ bare: true, modifier: null })).toBe(`${head} accept=fn`);
    expect(intent({ bare: false, modifier: null })).toBe(`${head} accept=none`);
  });
});
