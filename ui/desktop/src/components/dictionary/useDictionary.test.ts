import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DICTATION_MESSAGES } from '../dictation/copy';
import { DICTIONARY_MESSAGES } from './copy';
import { answer, entry, listing, settings } from './dictionaryTestKit';

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  addDictionaryEntry: vi.fn(),
  editDictionaryEntry: vi.fn(),
  changeDictionaryEntryTier: vi.fn(),
  deleteDictionaryEntry: vi.fn(),
  deleteDictionaryEntries: vi.fn(),
  acceptDictionarySuggestions: vi.fn(),
  dismissDictionarySuggestions: vi.fn(),
  keepDictionaryEntries: vi.fn(),
  importDictionary: vi.fn(),
  exportDictionary: vi.fn(),
}));

vi.mock('../../api', () => api);

const PROOF = { 'X-User-Action': 'proof-of-a-person' };
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => PROOF),
}));

import {
  DICTIONARY_REFRESH_MS,
  acceptTerms,
  addTerm,
  installLiveRefresh,
  applyImport,
  deleteTerm,
  deleteTerms,
  derivedEtag,
  dictionaryRereadForTests,
  dismissTerms,
  editTerm,
  exportTerms,
  failureOf,
  getDictionarySnapshot,
  getWaitingCount,
  keepTerms,
  planImport,
  refreshDictionary,
  refreshWaitingCount,
  resetDictionaryStoreForTests,
  setTermTier,
  useDictionaryWaitingCount,
} from './useDictionary';

class Deferred<T> {
  promise: Promise<T>;
  resolve!: (value: T) => void;
  constructor() {
    this.promise = new Promise((resolve) => (this.resolve = resolve));
  }
}

beforeEach(() => {
  resetDictionaryStoreForTests();
  for (const fn of Object.values(api)) fn.mockReset();
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.listDictionaryEntries.mockResolvedValue(listing([]));
  for (const write of [
    api.addDictionaryEntry,
    api.editDictionaryEntry,
    api.changeDictionaryEntryTier,
    api.acceptDictionarySuggestions,
    api.dismissDictionarySuggestions,
    api.keepDictionaryEntries,
    api.importDictionary,
  ]) {
    write.mockResolvedValue(answer(200, { entry: entry(), created: true, accepted: 1 }));
  }
  api.deleteDictionaryEntry.mockResolvedValue(answer(204, null));
  api.deleteDictionaryEntries.mockResolvedValue(answer(200, { deleted: 2 }));
  api.exportDictionary.mockResolvedValue({ data: '{}', response: new Response('{}') });
});

afterEach(() => {
  vi.useRealTimers();
});

/** Every call's options carry the proof (the desktop) or the stated provider (`serve`). */
function expectProofOnEveryCall() {
  let calls = 0;
  for (const [name, fn] of Object.entries(api)) {
    for (const [options] of fn.mock.calls) {
      calls += 1;
      expect(options?.headers, name).toMatchObject(PROOF);
    }
  }
  return calls;
}

describe('every dictionary call carries userActionHeaders()', () => {
  it('on the listing, the settings and every write', async () => {
    const row = entry({ id: 'dct_a', rev: 3 });
    await refreshDictionary();
    await addTerm({ written: 'EPAS1', spoken_forms: ['epas one'] });
    await editTerm(row.id, row.rev, { note: 'gene' });
    await setTermTier(row.id, 'private');
    await deleteTerm(row.id);
    await deleteTerms([row.id, 'dct_b']);
    await acceptTerms([row.id]);
    await dismissTerms([row.id]);
    await keepTerms([row.id]);
    await planImport({ format: 'lines', content: 'UCSF' });
    await applyImport({ format: 'lines', content: 'UCSF' });
    await exportTerms(true);
    await refreshWaitingCount();

    for (const fn of Object.values(api)) expect(fn).toHaveBeenCalled();
    // Each write reads the listing again, so the count is far above the twelve routes.
    expect(expectProofOnEveryCall()).toBeGreaterThan(20);
  });

  it('sends the revision as If-Match on an edit, beside the proof', async () => {
    await editTerm('dct_a', 7, { written: 'EPAS1' });
    expect(api.editDictionaryEntry).toHaveBeenCalledWith(
      expect.objectContaining({
        path: { id: 'dct_a' },
        body: { written: 'EPAS1' },
        headers: { ...PROOF, 'If-Match': '7' },
      })
    );
  });

  it('marks a dry run as one and an import as not', async () => {
    await planImport({ format: 'csv', content: 'x', private: true });
    await applyImport({ format: 'csv', content: 'x', private: true });
    expect(api.importDictionary.mock.calls.map(([options]) => options.body.dry_run)).toEqual([
      true,
      false,
    ]);
  });

  it('splits a delete of more than 5,000 listed ids into requests of 5,000', async () => {
    const ids = Array.from({ length: 12_001 }, (_, i) => `dct_${i}`);
    const outcome = await deleteTerms(ids);
    expect(api.deleteDictionaryEntries.mock.calls.map(([o]) => o.body.ids.length)).toEqual([
      5_000, 5_000, 2_001,
    ]);
    expect(outcome).toEqual({ ok: true, status: 200, data: 6 });
  });
});

describe('the listing', () => {
  it('merges every page in order and reads the next one by its cursor', async () => {
    const first = [entry({ written: 'A' }), entry({ written: 'B' })];
    const second = [entry({ written: 'C' })];
    api.listDictionaryEntries
      .mockResolvedValueOnce(listing(first, 'cursor-2'))
      .mockResolvedValueOnce(listing(second));
    await refreshDictionary();
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['A', 'B', 'C']);
    expect(api.listDictionaryEntries.mock.calls[1][0].query).toEqual({
      status: 'all',
      cursor: 'cursor-2',
    });
    expect(getDictionarySnapshot().load).toBe('loaded');
  });

  it('drops a response that lands after a newer read started', async () => {
    const slow = new Deferred<ReturnType<typeof listing>>();
    const stale = [entry({ written: 'Old' })];
    const fresh = [entry({ written: 'New' })];
    api.listDictionaryEntries
      .mockImplementationOnce(() => slow.promise)
      .mockResolvedValueOnce(listing(fresh));
    const older = refreshDictionary();
    await refreshDictionary();
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['New']);
    slow.resolve(listing(stale));
    await older;
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['New']);
  });

  it('sends back the page’s tag as If-None-Match and keeps the page on a 304', async () => {
    const rows = [entry({ written: 'EPAS1' })];
    const page = listing(rows);
    api.listDictionaryEntries
      .mockResolvedValueOnce(page)
      .mockResolvedValueOnce({ error: {}, response: new Response(null, { status: 304 }) });
    await refreshDictionary();
    const before = getDictionarySnapshot().entries;
    await refreshDictionary();

    // The daemon's CORS layer hides `ETag` from the desktop, so the tag is the daemon's formula.
    const tag = await derivedEtag(page.data);
    expect(tag).toMatch(/^"[0-9a-f]{32}"$/);
    const second = api.listDictionaryEntries.mock.calls[1][0];
    expect(second.headers).toMatchObject({ ...PROOF, 'If-None-Match': tag });
    expect(second.cache).toBe('no-store');
    expect(getDictionarySnapshot().entries).toBe(before);
  });

  it('derives the tag the daemon’s etag_of gives a fixed page', async () => {
    // The same body and tag are pinned on the daemon's side by this test reading `etag_of`: a
    // change to either formula breaks one of them, never a conditional read in silence.
    expect(await derivedEtag('{"entries":[],"next_cursor":null}')).toBe(
      '"e361eacd7907fe46558d02abd26d21d0"'
    );
    const routes = readFileSync(
      join(__dirname, '../../../../../crates/biorouter-server/src/routes/dictionary.rs'),
      'utf8'
    );
    const etagOf = routes.slice(routes.indexOf('fn etag_of('));
    const body = etagOf.slice(0, etagOf.indexOf('\n}\n'));
    expect(body).toContain('let digest = Sha256::digest(bytes);');
    expect(body).toContain('format!("\\"{}\\"", hex::encode(&digest[..16]))');
  });

  it('prefers an ETag the response exposes', async () => {
    const rows = [entry()];
    const text = JSON.stringify({ entries: rows, next_cursor: null });
    api.listDictionaryEntries.mockResolvedValueOnce({
      data: text,
      response: new Response(text, { status: 200, headers: { ETag: '"from-the-daemon"' } }),
    });
    await refreshDictionary();
    await refreshDictionary();
    expect(api.listDictionaryEntries.mock.calls[1][0].headers['If-None-Match']).toBe(
      '"from-the-daemon"'
    );
  });

  it('shows the store’s sentence when the file cannot be read, and keeps rows already shown', async () => {
    api.listDictionaryEntries.mockResolvedValueOnce(listing([entry({ written: 'Kept' })]));
    await refreshDictionary();
    const unreadable =
      'The dictionary file at /x/dictionary.json could not be read, so nothing was changed. Fix or remove it, then try again.';
    api.listDictionaryEntries.mockResolvedValueOnce(
      answer(503, { code: 'unreadable', message: unreadable })
    );
    await refreshDictionary();
    expect(getDictionarySnapshot().error).toBe(unreadable);
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['Kept']);
  });

  it('marks a backend from before the dictionary as absent', async () => {
    api.listDictionaryEntries.mockResolvedValueOnce(answer(404, ''));
    await refreshDictionary();
    expect(getDictionarySnapshot().absent).toBe(true);
    expect(getDictionarySnapshot().error).toMatch(/is a version without local dictation/);
  });

  it('reads the listing again after a write, so the page shows confirmed state', async () => {
    api.listDictionaryEntries.mockResolvedValue(listing([]));
    await refreshDictionary();
    const calls = api.listDictionaryEntries.mock.calls.length;
    api.listDictionaryEntries.mockResolvedValue(listing([entry({ written: 'EPAS1' })]));
    await acceptTerms(['dct_a']);
    expect(api.listDictionaryEntries.mock.calls.length).toBe(calls + 1);
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['EPAS1']);
  });

  it('shows the row an add answered at once, then the listing as read again', async () => {
    const old = entry({ written: 'UCSF' });
    api.listDictionaryEntries.mockResolvedValue(listing([old]));
    await refreshDictionary();
    const added = entry({ written: 'EPAS1' });
    api.addDictionaryEntry.mockResolvedValueOnce(answer(201, { entry: added, created: true }));
    // The listing read after the add has not answered yet when the add does.
    const slow = new Deferred<ReturnType<typeof listing>>();
    api.listDictionaryEntries.mockImplementationOnce(() => slow.promise);
    const outcome = await addTerm({ written: 'EPAS1' });
    expect(outcome.ok).toBe(true);
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['EPAS1', 'UCSF']);
    const reread = { ...added, uses: 1 };
    slow.resolve(listing([reread, old]));
    await dictionaryRereadForTests();
    expect(getDictionarySnapshot().entries[0]).toEqual(reread);
  });

  it('takes a deleted row out at once and replaces an edited one in place', async () => {
    const one = entry({ written: 'One' });
    const two = entry({ written: 'Two' });
    api.listDictionaryEntries.mockResolvedValue(listing([one, two]));
    await refreshDictionary();
    const slow = new Deferred<ReturnType<typeof listing>>();
    api.listDictionaryEntries.mockImplementationOnce(() => slow.promise);
    await deleteTerm(one.id);
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['Two']);
    slow.resolve(listing([two]));
    await dictionaryRereadForTests();

    const renamed = { ...two, written: 'Two again', rev: 2 };
    api.editDictionaryEntry.mockResolvedValueOnce(answer(200, { entry: renamed }));
    await editTerm(two.id, two.rev, { written: 'Two again' });
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['Two again']);
  });

  it('keeps the listing as it was when a write is refused, and reads it again first', async () => {
    const one = entry({ written: 'One' });
    api.listDictionaryEntries.mockResolvedValue(listing([one]));
    await refreshDictionary();
    const calls = api.listDictionaryEntries.mock.calls.length;
    api.editDictionaryEntry.mockResolvedValueOnce(
      answer(412, { code: 'stale', message: 'Changed elsewhere.', rev: 4 })
    );
    const outcome = await editTerm(one.id, one.rev, { written: 'Uno' });
    expect(outcome.ok).toBe(false);
    expect(api.listDictionaryEntries.mock.calls.length).toBe(calls + 1);
    expect(getDictionarySnapshot().entries.map((row) => row.written)).toEqual(['One']);
  });
});

describe('a refusal’s sentence', () => {
  it('is the daemon’s own when it sent one', () => {
    expect(
      failureOf(
        answer(403, { code: 'host_private', message: 'Private terms are added on the server.' })
      )
    ).toMatchObject({
      status: 403,
      code: 'host_private',
      message: 'Private terms are added on the server.',
    });
  });

  it('keeps a stale edit’s revision', () => {
    expect(failureOf(answer(412, { code: 'stale', message: '', rev: 9 }))).toEqual({
      status: 412,
      code: 'stale',
      message: DICTIONARY_MESSAGES['MSG-DICTIONARY-STALE'],
      rev: 9,
    });
  });

  it('says the list is too large for the 413 that carries no sentence', () => {
    expect(failureOf(answer(413, { code: 'too_large', message: '' })).message).toBe(
      DICTIONARY_MESSAGES['MSG-DICTIONARY-TOO-LARGE']
    );
  });

  it('says the service did not answer when nothing came back', () => {
    expect(failureOf({})).toEqual({
      status: 0,
      code: 'unreachable',
      message: DICTATION_MESSAGES['MSG-SETTINGS-UNREACHABLE'],
    });
  });

  it('names the code and status of any other refusal with no sentence', () => {
    expect(failureOf(answer(422, { code: 'bad_request', message: '' })).message).toBe(
      'The Biorouter service refused this (bad_request, HTTP 422).'
    );
  });
});

describe('the sidebar’s count', () => {
  it('counts the suggestions the first page lists', async () => {
    api.listDictionaryEntries.mockResolvedValueOnce(
      listing([entry({ status: 'suggested', source: 'auto' }), entry({ status: 'suggested' })])
    );
    await refreshWaitingCount();
    expect(api.listDictionaryEntries.mock.calls[0][0].query).toEqual({ status: 'suggested' });
    expect(getWaitingCount()).toBe(2);
  });

  it('counts the agent’s rows not yet kept once the page has listed them', async () => {
    api.listDictionaryEntries.mockResolvedValueOnce(
      listing([
        entry({ status: 'suggested' }),
        entry({ source: 'agent', reviewed: false }),
        entry({ source: 'agent', reviewed: true }),
        entry(),
      ])
    );
    await refreshDictionary();
    expect(getWaitingCount()).toBe(2);
  });

  it('reads nothing while it is not enabled, and reads once it is', async () => {
    api.listDictionaryEntries.mockResolvedValue(listing([entry({ status: 'suggested' })]));
    const { result, rerender } = renderHook(({ on }) => useDictionaryWaitingCount(on), {
      initialProps: { on: false },
    });
    await act(async () => undefined);
    expect(api.listDictionaryEntries).not.toHaveBeenCalled();
    expect(result.current).toBe(0);
    rerender({ on: true });
    await waitFor(() => expect(result.current).toBe(1));
    expect(api.listDictionaryEntries).toHaveBeenCalledTimes(1);
  });
});

describe('live refresh', () => {
  it('reads on window focus, on becoming visible and every 60 s while visible', () => {
    vi.useFakeTimers();
    const read = vi.fn();
    const dispose = installLiveRefresh(read);
    window.dispatchEvent(new Event('focus'));
    expect(read).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(DICTIONARY_REFRESH_MS);
    expect(read).toHaveBeenCalledTimes(2);

    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    vi.advanceTimersByTime(DICTIONARY_REFRESH_MS);
    document.dispatchEvent(new Event('visibilitychange'));
    expect(read).toHaveBeenCalledTimes(2);
    visibility.mockReturnValue('visible');
    document.dispatchEvent(new Event('visibilitychange'));
    expect(read).toHaveBeenCalledTimes(3);

    dispose();
    window.dispatchEvent(new Event('focus'));
    vi.advanceTimersByTime(DICTIONARY_REFRESH_MS * 2);
    expect(read).toHaveBeenCalledTimes(3);
    visibility.mockRestore();
  });
});
