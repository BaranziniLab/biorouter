import { act, render } from '@testing-library/react';
import { createElement, useRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FnKeyView } from '../../utils/dictationFnKey';

const session = vi.hoisted(() => ({
  snapshot: { key: null as string | null, activity: 'idle' as string },
  token: null as object | null,
  quiet: true,
  stop: vi.fn(),
  cancel: vi.fn(),
  /** What a cancel does: at once (the real session), or leave the capture closing first. */
  closingCancels: 0,
  attach: vi.fn(),
  start: vi.fn(),
  holding: new Set<string>(),
}));

vi.mock('./dictationSession', () => ({
  getDictationSnapshot: () => session.snapshot,
  activeDictationToken: () => session.token,
  canStartDictationQuietly: () => session.quiet,
  stopDictation: (key?: string) => session.stop(key),
  cancelDictation: (key?: string) => {
    session.cancel(key);
    if (session.closingCancels > 0) {
      // The capture is still closing: it reads as arming, under the same token, for a moment.
      session.closingCancels--;
      return;
    }
    session.snapshot = { key: null, activity: 'idle' };
    session.token = null;
  },
  attachDictationComposer: (composer: { key: string }) => {
    session.attach(composer);
    return () => session.attach(`detach:${composer.key}`);
  },
  isDictationHolding: (key: string) => session.holding.has(key),
  startDictation: (options: { key: string }) => {
    session.start(options);
    session.snapshot = { key: options.key, activity: 'arming' };
    session.token = { key: options.key };
    return Promise.resolve({ kind: 'recording' });
  },
  requestDictationSetup: vi.fn(),
  noteDictationUnavailable: vi.fn(),
  subscribeDictation: () => () => undefined,
  dictationActivityFor: (key: string) =>
    session.snapshot.key === key ? session.snapshot.activity : 'idle',
}));

vi.mock('./dictationStore', () => ({ noteDictationInterest: vi.fn() }));

const fnKey = vi.hoisted(() => ({ view: null as FnKeyView | null }));
vi.mock('./fnKeyStore', () => ({
  getFnKeySnapshot: () => fnKey.view,
  subscribeFnKey: () => () => undefined,
}));

import { FN_USAGE_ACTIONS, dictationMessage } from './copy';
import {
  FN_CONFLICT_SHOWN_KEY,
  dismissFnConflictNote,
  fnTargetKey,
  getFnConflictNote,
  handleFnIntent,
  registerFnGestureTarget,
  resetFnGestureClientForTests,
} from './fnGestureClient';
import { useDictationComposer } from './useDictationComposer';

function composer(key: string, { showsNotes = false, parent = document.body } = {}) {
  const field = document.createElement('textarea');
  parent.append(field);
  let live: HTMLTextAreaElement | null = field;
  const press = vi.fn(() => {
    // The composer's own press starts a recording, as the microphone does.
    session.snapshot = { key, activity: 'arming' };
    session.token = { key };
  });
  const off = registerFnGestureTarget(key, { press, field: () => live, showsNotes });
  return {
    field,
    press,
    off,
    /** The field unmounts, as a message editor's does when it closes. */
    close() {
      field.remove();
      live = null;
    },
  };
}

function fnView(fnUsage: 0 | 1 | 2 | 3 | null): FnKeyView {
  return {
    settings: { enabled: true, otherApps: false },
    status: {
      supported: true,
      active: true,
      permission: 'granted',
      listening: true,
      fnUsage,
      failed: false,
    },
    anywhereOn: false,
  };
}

beforeEach(() => {
  resetFnGestureClientForTests();
  session.snapshot = { key: null, activity: 'idle' };
  session.token = null;
  session.quiet = true;
  session.closingCancels = 0;
  session.holding.clear();
  session.stop.mockClear();
  session.cancel.mockClear();
  session.attach.mockClear();
  session.start.mockClear();
  fnKey.view = null;
  window.localStorage.clear();
  document.body.innerHTML = '';
});
afterEach(() => resetFnGestureClientForTests());

describe('which composer an fn dictation goes into', () => {
  it('the focused one, else the one focused last, else the only one', () => {
    const a = composer('a');
    expect(fnTargetKey()).toBe('a');
    const b = composer('b');
    expect(fnTargetKey()).toBeNull();
    b.field.focus();
    expect(fnTargetKey()).toBe('b');
    a.field.focus();
    a.field.blur();
    expect(fnTargetKey()).toBe('a');
    a.off();
    expect(fnTargetKey()).toBe('b');
  });

  it('never a target whose field is gone; after a closed editor, the composer', () => {
    const chat = composer('tab:a');
    const editor = composer('editor:m1');
    editor.field.focus();
    expect(fnTargetKey()).toBe('editor:m1');
    // Closing the editor removes its textarea and focuses its Edit button.
    editor.close();
    const edit = document.createElement('button');
    document.body.append(edit);
    edit.focus();
    expect(fnTargetKey()).toBe('tab:a');
    handleFnIntent('start-hold');
    expect(chat.press).toHaveBeenCalledTimes(1);
    expect(editor.press).not.toHaveBeenCalled();
  });

  it('forgets the composer focused last when focus moves to something that is no composer', () => {
    const a = composer('a');
    composer('b');
    a.field.focus();
    const button = document.createElement('button');
    document.body.append(button);
    button.focus();
    // a's field is still there, but the person moved on: no composer is chosen by memory.
    expect(fnTargetKey()).toBeNull();
  });

  it('a focused field no composer owns gets the dictation, with public terms only', () => {
    const chat = composer('tab:a');
    const rename = document.createElement('input');
    document.body.append(rename);
    rename.focus();
    const key = fnTargetKey();
    expect(key).toMatch(/^field:/);
    // The same field keeps its key.
    expect(fnTargetKey()).toBe(key);
    expect(session.attach).toHaveBeenCalledWith(
      expect.objectContaining({ key, storesDraft: false, loose: true })
    );
    handleFnIntent('start-hold');
    expect(session.start).toHaveBeenCalledWith({ key, target: { target_kind: 'external' } });
    expect(chat.press).not.toHaveBeenCalled();
  });

  it('never a password field: fn then goes to the composer', () => {
    const chat = composer('tab:a');
    const secret = document.createElement('input');
    secret.type = 'password';
    document.body.append(secret);
    secret.focus();
    expect(fnTargetKey()).toBe('tab:a');
    handleFnIntent('start-hold');
    expect(chat.press).toHaveBeenCalledTimes(1);
    expect(session.start).not.toHaveBeenCalled();
  });

  it('inside an open modal dialog, only its fields: a composer behind it is never chosen', () => {
    const chat = composer('tab:a');
    const dialog = document.createElement('div');
    dialog.setAttribute('role', 'dialog');
    dialog.setAttribute('aria-modal', 'true');
    const close = document.createElement('button');
    dialog.append(close);
    document.body.append(dialog);
    close.focus();
    expect(fnTargetKey()).toBeNull();
    handleFnIntent('start-hold');
    expect(chat.press).not.toHaveBeenCalled();
  });

  it('lets a field go once no dictation owes it text', () => {
    const rename = document.createElement('input');
    document.body.append(rename);
    rename.focus();
    const key = fnTargetKey()!;
    session.holding.add(key);
    rename.blur();
    composer('tab:a');
    fnTargetKey();
    expect(session.attach).not.toHaveBeenCalledWith(`detach:${key}`);
    session.holding.delete(key);
    fnTargetKey();
    expect(session.attach).toHaveBeenCalledWith(`detach:${key}`);
  });
});

describe('a composer whose field is not there', () => {
  function Editor({ enabled }: { enabled: boolean }) {
    const ref = useRef<HTMLTextAreaElement | null>(null);
    useDictationComposer({
      key: 'editor:m1',
      textAreaRef: ref,
      storesDraft: false,
      target: () => ({ target_kind: 'editor' }),
      setByState: () => undefined,
      enabled,
    });
    return enabled ? createElement('textarea', { ref }) : null;
  }

  it('a closed editor neither attaches to the session nor registers for fn', () => {
    const view = render(createElement(Editor, { enabled: false }));
    expect(session.attach).not.toHaveBeenCalled();
    expect(fnTargetKey()).toBeNull();
    view.rerender(createElement(Editor, { enabled: true }));
    expect(session.attach).toHaveBeenCalledWith(expect.objectContaining({ key: 'editor:m1' }));
    expect(fnTargetKey()).toBe('editor:m1');
    view.rerender(createElement(Editor, { enabled: false }));
    expect(session.attach).toHaveBeenCalledWith('detach:editor:m1');
    expect(fnTargetKey()).toBeNull();
    view.unmount();
  });
});

describe('the fn intents in a window', () => {
  it('a hold presses the focused composer and its release stops that recording', () => {
    const a = composer('a');
    a.field.focus();
    handleFnIntent('start-hold');
    expect(a.press).toHaveBeenCalledTimes(1);
    session.snapshot = { key: 'a', activity: 'recording' };
    handleFnIntent('end-hold');
    expect(session.stop).toHaveBeenCalledWith('a');
  });

  it('fn with another key during a hold cancels that recording', () => {
    composer('a');
    handleFnIntent('start-hold');
    handleFnIntent('cancel-hold');
    expect(session.cancel).toHaveBeenCalledWith('a');
    expect(session.stop).not.toHaveBeenCalled();
  });

  it('a hold while the microphone already records starts nothing, and its release stops nothing', () => {
    const a = composer('a');
    session.snapshot = { key: 'a', activity: 'recording' };
    session.token = { started: 'by the microphone' };
    handleFnIntent('start-hold');
    handleFnIntent('end-hold');
    expect(a.press).not.toHaveBeenCalled();
    expect(session.stop).not.toHaveBeenCalled();
  });

  it('a release does not stop a recording the microphone started after the hold’s was stopped', () => {
    composer('a');
    handleFnIntent('start-hold');
    // The microphone stopped the hold's recording and started another while fn was still held.
    session.token = { another: true };
    session.snapshot = { key: 'a', activity: 'recording' };
    handleFnIntent('end-hold');
    expect(session.stop).not.toHaveBeenCalled();
  });

  it('a double-tap stops whatever records, the microphone’s included, and starts one otherwise', () => {
    const a = composer('a');
    session.snapshot = { key: 'a', activity: 'recording' };
    session.token = { started: 'by the microphone' };
    handleFnIntent('toggle');
    expect(session.stop).toHaveBeenCalledWith('a');
    expect(a.press).not.toHaveBeenCalled();

    session.snapshot = { key: null, activity: 'idle' };
    session.token = null;
    handleFnIntent('toggle');
    expect(a.press).toHaveBeenCalledTimes(1);

    // While the text is being finished, a double-tap waits for it.
    session.snapshot = { key: 'a', activity: 'transcribing' };
    handleFnIntent('toggle');
    expect(a.press).toHaveBeenCalledTimes(1);
    expect(session.stop).toHaveBeenCalledTimes(1);
  });

  it('fn down starts capturing at once, and the hold keeps that recording', () => {
    const a = composer('a');
    handleFnIntent('prime');
    expect(a.press).toHaveBeenCalledTimes(1);
    session.snapshot = { key: 'a', activity: 'recording' };
    handleFnIntent('start-hold');
    expect(a.press).toHaveBeenCalledTimes(1);
    expect(session.cancel).not.toHaveBeenCalled();
    handleFnIntent('end-hold');
    expect(session.stop).toHaveBeenCalledWith('a');
  });

  it('a tap or fn with another key discards what fn down captured', () => {
    const a = composer('a');
    handleFnIntent('prime');
    handleFnIntent('cancel-hold');
    expect(session.cancel).toHaveBeenCalledWith('a');
    expect(session.stop).not.toHaveBeenCalled();
    expect(a.press).toHaveBeenCalledTimes(1);
  });

  it('fn down opens nothing when a press would ask first; the hold then presses as before', () => {
    const a = composer('a');
    session.quiet = false;
    handleFnIntent('prime');
    expect(a.press).not.toHaveBeenCalled();
    // A tap: nothing was started, so nothing is cancelled.
    handleFnIntent('cancel-hold');
    expect(session.cancel).not.toHaveBeenCalled();
    // A hold: the press happens at the threshold, setup popover and all.
    handleFnIntent('prime');
    handleFnIntent('start-hold');
    expect(a.press).toHaveBeenCalledTimes(1);
  });

  it('fn down while the microphone records starts nothing, and its tap cancels nothing', () => {
    const a = composer('a');
    session.snapshot = { key: 'a', activity: 'recording' };
    session.token = { started: 'by the microphone' };
    handleFnIntent('prime');
    handleFnIntent('cancel-hold');
    expect(a.press).not.toHaveBeenCalled();
    expect(session.cancel).not.toHaveBeenCalled();
  });

  it('a double-tap keeps the recording its second press started, hands-free', () => {
    const a = composer('a');
    handleFnIntent('prime');
    handleFnIntent('cancel-hold');
    handleFnIntent('prime');
    expect(a.press).toHaveBeenCalledTimes(2);
    session.snapshot = { key: 'a', activity: 'recording' };
    handleFnIntent('toggle');
    expect(session.stop).not.toHaveBeenCalled();
    expect(session.cancel).toHaveBeenCalledTimes(1);
    expect(a.press).toHaveBeenCalledTimes(2);
    // The next double-tap's presses start nothing, and its toggle stops that recording.
    handleFnIntent('prime');
    handleFnIntent('cancel-hold');
    handleFnIntent('prime');
    handleFnIntent('toggle');
    expect(session.stop).toHaveBeenCalledWith('a');
    expect(a.press).toHaveBeenCalledTimes(2);
  });

  it('a toggle after a second press that could not start quietly starts exactly one recording', () => {
    const a = composer('a');
    handleFnIntent('prime');
    handleFnIntent('cancel-hold');
    // The second press would have asked first (a prompt, a consent): it captures nothing.
    session.quiet = false;
    handleFnIntent('prime');
    expect(a.press).toHaveBeenCalledTimes(1);
    handleFnIntent('toggle');
    expect(a.press).toHaveBeenCalledTimes(2);
    expect(session.snapshot).toEqual({ key: 'a', activity: 'arming' });
    expect(session.stop).not.toHaveBeenCalled();
    expect(session.cancel).toHaveBeenCalledTimes(1);
  });

  it('a toggle while the first tap’s capture is still closing starts a fresh recording', () => {
    const a = composer('a');
    handleFnIntent('prime');
    const first = session.token;
    session.closingCancels = 1;
    handleFnIntent('cancel-hold');
    // The first tap's capture still reads as arming, so the second press starts nothing...
    expect(session.snapshot.activity).toBe('arming');
    handleFnIntent('prime');
    expect(a.press).toHaveBeenCalledTimes(1);
    // ...and the toggle does not stop it as a dictation: it goes, and a fresh one starts.
    handleFnIntent('toggle');
    expect(session.stop).not.toHaveBeenCalled();
    expect(a.press).toHaveBeenCalledTimes(2);
    expect(session.token).not.toBe(first);
    expect(session.snapshot).toEqual({ key: 'a', activity: 'arming' });
  });

  it('the stop double-tap calls stopDictation once and never cancelDictation', () => {
    const a = composer('a');
    session.snapshot = { key: 'a', activity: 'recording' };
    session.token = { started: 'hands-free' };
    handleFnIntent('prime');
    handleFnIntent('cancel-hold');
    handleFnIntent('prime');
    handleFnIntent('toggle');
    expect(session.stop).toHaveBeenCalledTimes(1);
    expect(session.stop).toHaveBeenCalledWith('a');
    expect(session.cancel).not.toHaveBeenCalled();
    expect(a.press).not.toHaveBeenCalled();
  });

  it('does nothing in a window with no composer', () => {
    handleFnIntent('prime');
    handleFnIntent('start-hold');
    handleFnIntent('toggle');
    handleFnIntent('end-hold');
    expect(session.stop).not.toHaveBeenCalled();
    expect(session.cancel).not.toHaveBeenCalled();
  });
});

describe('the one-time fn note', () => {
  it('shows MSG-FN-CONFLICT in the composer at the first press, once per profile', () => {
    const chat = composer('tab:a', { showsNotes: true });
    fnKey.view = fnView(2);
    chat.field.focus();
    handleFnIntent('prime');
    expect(getFnConflictNote()).toEqual({
      key: 'tab:a',
      text: dictationMessage('MSG-FN-CONFLICT', { action: FN_USAGE_ACTIONS[2] }),
    });
    expect(window.localStorage.getItem(FN_CONFLICT_SHOWN_KEY)).not.toBeNull();
    act(() => dismissFnConflictNote());
    expect(getFnConflictNote()).toBeNull();
    // A later press in this window, or a new window of the same profile, shows nothing.
    handleFnIntent('cancel-hold');
    handleFnIntent('prime');
    expect(getFnConflictNote()).toBeNull();
    resetFnGestureClientForTests();
    composer('tab:a', { showsNotes: true });
    handleFnIntent('prime');
    expect(getFnConflictNote()).toBeNull();
  });

  it('says what macOS may do when its fn setting is unset, and nothing when it is Do Nothing', () => {
    composer('tab:a', { showsNotes: true });
    fnKey.view = fnView(null);
    handleFnIntent('prime');
    expect(getFnConflictNote()?.text).toBe(dictationMessage('MSG-FN-CONFLICT-UNKNOWN'));

    resetFnGestureClientForTests();
    window.localStorage.clear();
    composer('tab:a', { showsNotes: true });
    fnKey.view = fnView(0);
    handleFnIntent('prime');
    expect(getFnConflictNote()).toBeNull();
    expect(window.localStorage.getItem(FN_CONFLICT_SHOWN_KEY)).toBeNull();
  });

  it('waits for the fn status, and for a composer that can show it', () => {
    // No composer draws notes here: the next press tries again.
    composer('editor:m1');
    fnKey.view = fnView(1);
    handleFnIntent('prime');
    expect(getFnConflictNote()).toBeNull();
    handleFnIntent('cancel-hold');
    composer('tab:a', { showsNotes: true });
    fnKey.view = null;
    handleFnIntent('prime');
    expect(getFnConflictNote()).toBeNull();
    handleFnIntent('cancel-hold');
    fnKey.view = fnView(1);
    handleFnIntent('prime');
    expect(getFnConflictNote()?.key).toBe('tab:a');
  });

  it('a profile whose storage cannot be read still shows it at most once in this window', () => {
    composer('tab:a', { showsNotes: true });
    fnKey.view = fnView(3);
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    try {
      handleFnIntent('prime');
      expect(getFnConflictNote()?.key).toBe('tab:a');
      dismissFnConflictNote();
      handleFnIntent('cancel-hold');
      handleFnIntent('prime');
      expect(getFnConflictNote()).toBeNull();
    } finally {
      getItem.mockRestore();
      setItem.mockRestore();
    }
  });
});
