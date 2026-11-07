import { afterEach, describe, expect, it } from 'vitest';
import {
  isDictationBusy,
  resetDictationBusyForTests,
  restartAllowedWhileDictating,
  setDictationBusy,
  type AskRestart,
  type BusyContents,
} from './dictationBusy';

class FakeContents implements BusyContents {
  readonly listeners = new Map<string, Array<() => void>>();
  constructor(readonly id: number) {}
  once(event: 'destroyed' | 'render-process-gone', listener: () => void) {
    this.listeners.set(event, [...(this.listeners.get(event) ?? []), listener]);
    return this;
  }
  fire(event: 'destroyed' | 'render-process-gone') {
    const listeners = this.listeners.get(event) ?? [];
    this.listeners.delete(event);
    for (const listener of listeners) listener();
  }
  count(event: string) {
    return this.listeners.get(event)?.length ?? 0;
  }
}

afterEach(() => resetDictationBusyForTests());

describe('which windows are dictating', () => {
  it('follows what each window says', () => {
    const a = new FakeContents(1);
    const b = new FakeContents(2);
    expect(isDictationBusy()).toBe(false);
    setDictationBusy(a, true);
    setDictationBusy(b, true);
    setDictationBusy(a, false);
    expect(isDictationBusy()).toBe(true);
    setDictationBusy(b, false);
    expect(isDictationBusy()).toBe(false);
  });

  it('forgets a window that closes or whose renderer crashes, which will never say false', () => {
    const closing = new FakeContents(1);
    setDictationBusy(closing, true);
    closing.fire('destroyed');
    expect(isDictationBusy()).toBe(false);

    const crashing = new FakeContents(2);
    setDictationBusy(crashing, true);
    crashing.fire('render-process-gone');
    expect(isDictationBusy()).toBe(false);
    // The recovered renderer dictates again, and is watched again.
    setDictationBusy(crashing, true);
    expect(isDictationBusy()).toBe(true);
    crashing.fire('render-process-gone');
    expect(isDictationBusy()).toBe(false);
  });

  it('watches a window once, however often it dictates', () => {
    const contents = new FakeContents(1);
    for (let i = 0; i < 20; i++) {
      setDictationBusy(contents, true);
      setDictationBusy(contents, false);
    }
    expect(contents.count('destroyed')).toBe(1);
    expect(contents.count('render-process-gone')).toBe(1);
  });
});

describe('Restart & update while dictating (MSG-RESTART-BUSY)', () => {
  const asker = (response: number) => {
    const asked: Parameters<AskRestart>[0][] = [];
    const ask: AskRestart = async (options) => {
      asked.push(options);
      return { response };
    };
    return { ask, asked };
  };

  it('asks nothing when no window is dictating', async () => {
    const { ask, asked } = asker(1);
    await expect(restartAllowedWhileDictating(ask)).resolves.toBe(true);
    expect(asked).toEqual([]);
  });

  it('asks while a window dictates, with Cancel as the default', async () => {
    setDictationBusy(new FakeContents(7), true);
    const { ask, asked } = asker(1);
    await expect(restartAllowedWhileDictating(ask)).resolves.toBe(false);
    expect(asked).toEqual([
      {
        type: 'question',
        message: 'Dictation in progress. Restart anyway?',
        buttons: ['Restart anyway', 'Cancel'],
        defaultId: 1,
        cancelId: 1,
      },
    ]);
  });

  it('restarts when the person chooses Restart anyway', async () => {
    setDictationBusy(new FakeContents(8), true);
    await expect(restartAllowedWhileDictating(asker(0).ask)).resolves.toBe(true);
  });
});
