import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const session = vi.hoisted(() => ({
  snapshot: { key: null as string | null, activity: 'idle' as string },
  token: null as object | null,
  quiet: true,
  stop: vi.fn(),
  cancel: vi.fn(),
}));

vi.mock('./dictationSession', () => ({
  getDictationSnapshot: () => session.snapshot,
  activeDictationToken: () => session.token,
  canStartDictationQuietly: () => session.quiet,
  stopDictation: (key?: string) => session.stop(key),
  cancelDictation: (key?: string) => {
    session.cancel(key);
    session.snapshot = { key: null, activity: 'idle' };
    session.token = null;
  },
}));

import {
  fnTargetKey,
  handleFnIntent,
  registerFnGestureTarget,
  resetFnGestureClientForTests,
} from './fnGestureClient';

function composer(key: string) {
  const field = document.createElement('textarea');
  document.body.append(field);
  const press = vi.fn(() => {
    // The composer's own press starts a recording, as the microphone does.
    session.snapshot = { key, activity: 'arming' };
    session.token = { key };
  });
  const off = registerFnGestureTarget(key, { press, field: () => field });
  return { field, press, off };
}

beforeEach(() => {
  resetFnGestureClientForTests();
  session.snapshot = { key: null, activity: 'idle' };
  session.token = null;
  session.quiet = true;
  session.stop.mockClear();
  session.cancel.mockClear();
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

  it('does nothing in a window with no composer', () => {
    handleFnIntent('prime');
    handleFnIntent('start-hold');
    handleFnIntent('toggle');
    handleFnIntent('end-hold');
    expect(session.stop).not.toHaveBeenCalled();
    expect(session.cancel).not.toHaveBeenCalled();
  });
});
