import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { answer, listing, scan, settings } from './dictionaryTestKit';

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  getDictionarySuggestionScan: vi.fn(),
  startDictionarySuggestionScan: vi.fn(),
  stopDictionarySuggestionScan: vi.fn(),
}));
vi.mock('../../api', () => api);

const PROOF = { 'X-User-Action': 'proof-of-a-person' };
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => PROOF),
}));

import { resetDictionaryStoreForTests } from './useDictionary';
import {
  SCAN_POLL_MS,
  getScanSnapshot,
  readScan,
  resetSuggestionScanForTests,
  scanLine,
  startScan,
  stopScan,
  useSuggestionScan,
  type ScanSnapshot,
} from './useSuggestionScan';

const running = (read: number, total: number, found: number) =>
  scan({ state: 'running', chats_read: read, chats_total: total, words_found: found });

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetSuggestionScanForTests();
  for (const fn of Object.values(api)) fn.mockReset();
  api.listDictionaryEntries.mockResolvedValue(listing([]));
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.getDictionarySuggestionScan.mockResolvedValue(answer(200, scan()));
});

afterEach(() => {
  vi.useRealTimers();
});

describe('the scan store', () => {
  it('sends userActionHeaders() on every call', async () => {
    api.startDictionarySuggestionScan.mockResolvedValue(answer(200, running(0, 3, 0)));
    api.stopDictionarySuggestionScan.mockResolvedValue(answer(200, running(1, 3, 0)));
    await readScan();
    await startScan();
    await stopScan();
    for (const fn of [
      api.getDictionarySuggestionScan,
      api.startDictionarySuggestionScan,
      api.stopDictionarySuggestionScan,
    ]) {
      expect(fn).toHaveBeenCalledWith(expect.objectContaining({ headers: PROOF }));
    }
  });

  it('drops a poll that lands after Stop, so it cannot undo it', async () => {
    api.startDictionarySuggestionScan.mockResolvedValue(answer(200, running(1, 40, 1)));
    await startScan();
    let land!: (value: unknown) => void;
    api.getDictionarySuggestionScan.mockReturnValue(new Promise((resolve) => (land = resolve)));
    const poll = readScan();
    api.stopDictionarySuggestionScan.mockResolvedValue(
      answer(200, scan({ state: 'stopped', chats_read: 2, chats_total: 40, words_found: 1 }))
    );
    await stopScan();
    land(answer(200, running(2, 40, 1)));
    await poll;
    expect(getScanSnapshot().scan?.state).toBe('stopped');
  });

  it('reads the listing again once when a scan it watched ends, and not for an old one', async () => {
    api.getDictionarySuggestionScan.mockResolvedValue(
      answer(200, scan({ state: 'finished', chats_read: 3, chats_total: 3, words_found: 2 }))
    );
    // A scan that ended before this window looked: nothing to re-read, nothing to say.
    await readScan();
    expect(api.listDictionaryEntries).not.toHaveBeenCalled();
    expect(scanLine(getScanSnapshot())).toBeNull();

    api.startDictionarySuggestionScan.mockResolvedValue(answer(200, running(0, 3, 0)));
    await startScan();
    await readScan();
    expect(api.listDictionaryEntries).toHaveBeenCalledTimes(1);
    await readScan();
    expect(api.listDictionaryEntries).toHaveBeenCalledTimes(1);
    expect(scanLine(getScanSnapshot())).toBe('Read 3 chats. 2 new words to review.');
  });

  it('keeps the daemon’s refusal, and a new start clears it', async () => {
    api.startDictionarySuggestionScan.mockResolvedValueOnce(
      answer(503, {
        code: 'unreadable',
        message: 'Biorouter could not read your dictionary, so it did not look for words.',
      })
    );
    await startScan();
    expect(getScanSnapshot().refusal).toBe(
      'Biorouter could not read your dictionary, so it did not look for words.'
    );
    api.startDictionarySuggestionScan.mockResolvedValue(answer(200, running(0, 2, 0)));
    await startScan();
    expect(getScanSnapshot().refusal).toBeNull();
  });
});

describe('the scan line', () => {
  const at = (overrides: Partial<ScanSnapshot>): ScanSnapshot => ({
    scan: null,
    watched: true,
    refusal: null,
    starting: false,
    stopping: false,
    ...overrides,
  });

  it('counts words and chats in proper plurals', () => {
    // The chat being read is the one after those done, never past the last.
    expect(scanLine(at({ scan: running(0, 40, 1) }))).toBe(
      'Reading chat 1 of 40, 1 word found so far'
    );
    expect(scanLine(at({ scan: running(2, 40, 3) }))).toBe(
      'Reading chat 3 of 40, 3 words found so far'
    );
    expect(scanLine(at({ scan: running(40, 40, 0) }))).toBe(
      'Reading chat 40 of 40, 0 words found so far'
    );
    expect(scanLine(at({ scan: running(0, 0, 0) }))).toBeNull();
    expect(
      scanLine(
        at({ scan: scan({ state: 'finished', chats_read: 1, chats_total: 1, words_found: 1 }) })
      )
    ).toBe('Read 1 chat. 1 new word to review.');
    expect(
      scanLine(
        at({ scan: scan({ state: 'finished', chats_read: 9, chats_total: 9, words_found: 0 }) })
      )
    ).toBe('Read 9 chats. No new words found.');
    expect(
      scanLine(
        at({ scan: scan({ state: 'stopped', chats_read: 3, chats_total: 40, words_found: 2 }) })
      )
    ).toBe('Stopped after 3 of 40 chats. 2 new words to review.');
    expect(
      scanLine(
        at({ scan: scan({ state: 'stopped', chats_read: 3, chats_total: 40, words_found: 0 }) })
      )
    ).toBe('Stopped after 3 of 40 chats. No new words found.');
    expect(scanLine(at({ scan: scan({ state: 'failed' }) }))).toBe(
      'Biorouter could not finish reading your chats.'
    );
    expect(scanLine(at({ scan: scan() }))).toBeNull();
  });
});

describe('useSuggestionScan', () => {
  it('reads once on mount and does not poll an idle scan', async () => {
    vi.useFakeTimers();
    renderHook(() => useSuggestionScan(true));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SCAN_POLL_MS * 4);
    });
    expect(api.getDictionarySuggestionScan).toHaveBeenCalledTimes(1);
  });

  it('polls about every 700 ms while a scan runs, and stops when it ends', async () => {
    vi.useFakeTimers();
    api.getDictionarySuggestionScan
      .mockResolvedValueOnce(answer(200, running(1, 3, 0)))
      .mockResolvedValueOnce(answer(200, running(2, 3, 1)))
      .mockResolvedValue(
        answer(200, scan({ state: 'finished', chats_read: 3, chats_total: 3, words_found: 1 }))
      );
    const { result } = renderHook(() => useSuggestionScan(true));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.scan?.chats_read).toBe(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SCAN_POLL_MS);
    });
    expect(result.current.scan?.chats_read).toBe(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SCAN_POLL_MS);
    });
    expect(result.current.scan?.state).toBe('finished');
    const reads = api.getDictionarySuggestionScan.mock.calls.length;
    expect(reads).toBe(3);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SCAN_POLL_MS * 5);
    });
    expect(api.getDictionarySuggestionScan).toHaveBeenCalledTimes(reads);
    // The scan it watched ended: the listing is read again for the suggestions it added.
    expect(api.listDictionaryEntries).toHaveBeenCalled();
  });

  it('stops polling when the control unmounts', async () => {
    vi.useFakeTimers();
    api.getDictionarySuggestionScan.mockResolvedValue(answer(200, running(1, 50, 0)));
    const { unmount } = renderHook(() => useSuggestionScan(true));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SCAN_POLL_MS * 2);
    });
    const reads = api.getDictionarySuggestionScan.mock.calls.length;
    expect(reads).toBeGreaterThan(1);
    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SCAN_POLL_MS * 5);
    });
    expect(api.getDictionarySuggestionScan).toHaveBeenCalledTimes(reads);
  });

  it('reads nothing while not enabled', async () => {
    vi.useFakeTimers();
    renderHook(() => useSuggestionScan(false));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SCAN_POLL_MS * 2);
    });
    expect(api.getDictionarySuggestionScan).not.toHaveBeenCalled();
  });
});
