import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { dictation, historyAnswer, historyPage, historySettings } from './history/historyTestKit';

const api = vi.hoisted(() => ({
  listDictationHistory: vi.fn(),
  getDictationHistorySettings: vi.fn(),
  changeDictationHistorySettings: vi.fn(),
  changeDictationHistoryTier: vi.fn(),
  deleteDictationHistoryEntry: vi.fn(),
  clearDictationHistory: vi.fn(),
}));
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  ...api,
}));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));

import { notifyDictationTranscribed } from '../dictation/dictationSession';
import { derivedEtag } from './useDictionary';
import {
  clearHistory,
  deleteHistoryEntry,
  getHistorySnapshot,
  HISTORY_PAGE_SIZE,
  historyQueryOf,
  historyRereadForTests,
  loadMoreHistory,
  refreshHistory,
  resetHistoryStoreForTests,
  searchHistory,
  setHistorySettings,
  setHistoryTier,
  useDictationHistory,
} from './useDictationHistory';

type ListArgs = {
  query?: { q?: string; cursor?: string; limit?: number };
  headers: Record<string, string>;
};

beforeEach(() => {
  resetHistoryStoreForTests();
  for (const fn of Object.values(api)) fn.mockReset();
  api.listDictationHistory.mockResolvedValue(historyPage([]));
  api.getDictationHistorySettings.mockResolvedValue(historyAnswer(200, historySettings()));
});

afterEach(() => {
  vi.useRealTimers();
});

/** Every call the generated client saw, of every route. */
const everyCall = () =>
  Object.values(api).flatMap((fn) => fn.mock.calls.map((call) => call[0] as ListArgs));

describe('the listing', () => {
  it('reads the first page newest first, with the proof, 100 at a time', async () => {
    const rows = [dictation({ text: 'EPAS1 binds HIF', minutesAgo: 1 })];
    api.listDictationHistory.mockResolvedValue(historyPage(rows, { total: 1 }));
    await refreshHistory();
    const snapshot = getHistorySnapshot();
    expect(snapshot.load).toBe('loaded');
    expect(snapshot.entries.map((entry) => entry.text)).toEqual(['EPAS1 binds HIF']);
    expect(snapshot.total).toBe(1);
    expect(snapshot.everything).toEqual({ total: 1, newestAt: rows[0].created_at });
    const call = api.listDictationHistory.mock.calls[0][0] as ListArgs;
    expect(call.query).toEqual({ limit: HISTORY_PAGE_SIZE });
    expect(call.headers).toMatchObject({ 'X-User-Action': 'proof' });
  });

  it('sends the derived tag back and keeps the very same rows on a 304', async () => {
    const page = historyPage([dictation()]);
    api.listDictationHistory.mockResolvedValueOnce(page);
    await refreshHistory();
    const first = getHistorySnapshot().entries;
    api.listDictationHistory.mockResolvedValueOnce({
      response: new Response(null, { status: 304 }),
    });
    await refreshHistory();
    const second = api.listDictationHistory.mock.calls[1][0] as ListArgs;
    expect(second.headers['If-None-Match']).toBe(await derivedEtag(page.data));
    expect(getHistorySnapshot().entries).toBe(first);
  });

  it('reads the next page as asked and appends it, and a refresh keeps as many', async () => {
    const one = Array.from({ length: 100 }, (_, index) => dictation({ minutesAgo: index + 1 }));
    const two = [dictation({ minutesAgo: 500 })];
    api.listDictationHistory.mockImplementation(async ({ query }: ListArgs) =>
      query?.cursor === 'c2'
        ? historyPage(two, { total: 101 })
        : historyPage(one, { total: 101, next_cursor: 'c2' })
    );
    await refreshHistory();
    expect(getHistorySnapshot().nextCursor).toBe('c2');
    await loadMoreHistory();
    expect(getHistorySnapshot().entries).toHaveLength(101);
    expect(getHistorySnapshot().nextCursor).toBeNull();

    api.listDictationHistory.mockClear();
    await refreshHistory();
    // The refresh walked both pages, so the person keeps their place.
    expect(api.listDictationHistory).toHaveBeenCalledTimes(2);
    expect(getHistorySnapshot().entries).toHaveLength(101);
  });

  it('says the file could not be read, and a backend from before the history', async () => {
    const message = 'The dictation history file at /x could not be read, so nothing was changed.';
    api.listDictationHistory.mockResolvedValueOnce(
      historyAnswer(503, { code: 'unreadable', message })
    );
    await refreshHistory();
    expect(getHistorySnapshot().error).toMatchObject({ code: 'unreadable', message });
    expect(getHistorySnapshot().absent).toBe(false);

    api.listDictationHistory.mockResolvedValueOnce(historyAnswer(404, {}));
    await refreshHistory();
    expect(getHistorySnapshot().absent).toBe(true);
  });
});

describe('the search', () => {
  it('cuts a search to 200 characters and trims it', () => {
    expect(historyQueryOf('  EPAS1  ')).toBe('EPAS1');
    expect(Array.from(historyQueryOf('é'.repeat(300)))).toHaveLength(200);
  });

  it('drops an answer that lands after a newer search started', async () => {
    let releaseOld: (() => void) | null = null;
    api.listDictationHistory.mockImplementation(async ({ query }: ListArgs) => {
      if (query?.q === 'EP') {
        await new Promise<void>((done) => (releaseOld = done));
        return historyPage([dictation({ text: 'EPO old' })]);
      }
      return historyPage([dictation({ text: 'EPAS1 new' })]);
    });
    const older = searchHistory('EP');
    await waitFor(() => expect(releaseOld).not.toBeNull());
    await searchHistory('EPAS1');
    releaseOld!();
    await older;
    expect(getHistorySnapshot().query).toBe('EPAS1');
    expect(getHistorySnapshot().entries.map((entry) => entry.text)).toEqual(['EPAS1 new']);
  });

  it('sends the field 200 ms after the last keystroke, once', async () => {
    vi.useFakeTimers();
    const { rerender } = renderHook(
      ({ query }: { query: string }) => useDictationHistory({ enabled: true, query }),
      { initialProps: { query: '' } }
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    api.listDictationHistory.mockClear();
    for (const typed of ['E', 'EP', 'EPA', 'EPAS1']) {
      rerender({ query: typed });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(50);
      });
    }
    expect(api.listDictationHistory).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    const asked = api.listDictationHistory.mock.calls.map((call) => (call[0] as ListArgs).query?.q);
    expect(asked).toEqual(['EPAS1']);
  });
});

describe('live updates', () => {
  it('reads again on focus, on becoming visible, every 60 s and after a transcript', async () => {
    vi.useFakeTimers();
    const { unmount } = renderHook(() => useDictationHistory({ enabled: true, query: '' }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const reads = () => api.listDictationHistory.mock.calls.length;
    const before = reads();
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(reads()).toBe(before + 1);
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(reads()).toBe(before + 2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(reads()).toBe(before + 3);
    await act(async () => {
      notifyDictationTranscribed();
      await vi.advanceTimersByTimeAsync(499);
    });
    expect(reads()).toBe(before + 3);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(reads()).toBe(before + 4);
    unmount();
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(reads()).toBe(before + 4);
  });

  // F18 (DD-102): a dictation made in another window, another app or the hidden capture page.
  it('reads again half a second after main says the history changed elsewhere, and after its own transcript, once each', async () => {
    vi.useFakeTimers();
    let changed: (() => void) | null = null;
    const off = vi.fn();
    Object.assign(window, {
      electron: {
        ...(window as { electron?: object }).electron,
        onDictationHistoryChanged: (callback: () => void) => {
          changed = callback;
          return off;
        },
      },
    });
    try {
      const { unmount } = renderHook(() => useDictationHistory({ enabled: true, query: '' }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(changed).not.toBeNull();
      const reads = () => api.listDictationHistory.mock.calls.length;
      const before = reads();
      await act(async () => {
        changed!();
        await vi.advanceTimersByTimeAsync(499);
      });
      expect(reads()).toBe(before);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1);
      });
      expect(reads()).toBe(before + 1);
      await act(async () => {
        notifyDictationTranscribed();
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(reads()).toBe(before + 2);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_000);
      });
      expect(reads()).toBe(before + 2);
      unmount();
      expect(off).toHaveBeenCalledTimes(1);
    } finally {
      Object.assign(window, {
        electron: {
          ...(window as { electron?: object }).electron,
          onDictationHistoryChanged: undefined,
        },
      });
    }
  });

  it('reads nothing while the tab is closed', async () => {
    renderHook(() => useDictationHistory({ enabled: false, query: 'EPAS1' }));
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    expect(api.listDictationHistory).not.toHaveBeenCalled();
    expect(api.getDictationHistorySettings).not.toHaveBeenCalled();
  });
});

describe('writes', () => {
  it('puts a tier change and a delete in place at once, then reads again', async () => {
    const [first, second] = [dictation({ minutesAgo: 1 }), dictation({ minutesAgo: 2 })];
    api.listDictationHistory.mockResolvedValue(historyPage([first, second]));
    await refreshHistory();

    api.changeDictationHistoryTier.mockResolvedValue(
      historyAnswer(200, { entry: { ...first, tier: 'public' } })
    );
    const tier = await setHistoryTier(first.id, 'public');
    expect(tier.ok).toBe(true);
    expect(getHistorySnapshot().entries[0].tier).toBe('public');
    expect(api.changeDictationHistoryTier.mock.calls[0][0]).toMatchObject({
      path: { id: first.id },
      body: { tier: 'public' },
    });
    await historyRereadForTests();

    api.deleteDictationHistoryEntry.mockResolvedValue(historyAnswer(204, null));
    await deleteHistoryEntry(second.id);
    expect(getHistorySnapshot().entries.map((entry) => entry.id)).not.toContain(second.id);
    await historyRereadForTests();
  });

  it('clears through the newest dictation the last unsearched read showed', async () => {
    const newest = dictation({ minutesAgo: 1 });
    api.listDictationHistory.mockResolvedValue(historyPage([newest, dictation()], { total: 2 }));
    await refreshHistory();
    api.listDictationHistory.mockResolvedValue(historyPage([], { total: 0 }));
    api.clearDictationHistory.mockResolvedValue(historyAnswer(200, { deleted: 2 }));
    const outcome = await clearHistory();
    expect(outcome).toMatchObject({ ok: true, data: { deleted: 2 } });
    expect(api.clearDictationHistory.mock.calls[0][0].body).toEqual({
      through: newest.created_at,
    });
  });

  it('says why a write was refused, with the daemon’s sentence', async () => {
    api.changeDictationHistorySettings.mockResolvedValue(
      historyAnswer(403, { code: 'host_settings', message: 'Do this on the server.' })
    );
    const outcome = await setHistorySettings({ keep_history: true });
    expect(outcome).toMatchObject({ ok: false, error: { code: 'host_settings' } });
  });

  it('sends the proof on every call', async () => {
    const one = dictation();
    api.listDictationHistory.mockResolvedValue(historyPage([one], { next_cursor: 'c2' }));
    api.changeDictationHistoryTier.mockResolvedValue(historyAnswer(200, { entry: one }));
    api.deleteDictationHistoryEntry.mockResolvedValue(historyAnswer(204, null));
    api.clearDictationHistory.mockResolvedValue(historyAnswer(200, { deleted: 1 }));
    api.changeDictationHistorySettings.mockResolvedValue(historyAnswer(200, historySettings()));
    await refreshHistory();
    await loadMoreHistory();
    await searchHistory('EPAS1');
    await setHistoryTier(one.id, 'private');
    await historyRereadForTests();
    await deleteHistoryEntry(one.id);
    await historyRereadForTests();
    await clearHistory();
    await setHistorySettings({ keep_history: false });
    const calls = everyCall();
    expect(calls.length).toBeGreaterThanOrEqual(12);
    for (const call of calls) expect(call.headers).toMatchObject({ 'X-User-Action': 'proof' });
  });
});
