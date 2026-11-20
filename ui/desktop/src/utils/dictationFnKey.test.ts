import { describe, expect, it } from 'vitest';
import {
  defaultFnKeySettings,
  fnDictationRunning,
  fnShortcutWorks,
  fnStartRoute,
  fnToggleRoute,
  parseFnKeySettings,
  type FnWorld,
} from './dictationFnKey';

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
    ).toEqual({ kind: 'anywhere' });
    expect(fnToggleRoute(world({ anywherePhase: 'arming' }))).toEqual({ kind: 'anywhere' });
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
