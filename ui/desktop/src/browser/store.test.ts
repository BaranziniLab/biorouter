import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createBrowserStore, HISTORY_RETENTION_MS, isRecordableUrl } from './store';

let dir: string;
let now = Date.UTC(2026, 10, 19);
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-browser-store-'));
  now = Date.UTC(2026, 10, 19);
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

const open = () => createBrowserStore(dir, { now: () => now });

describe('the browser history and bookmarks', () => {
  it('records visits, counts them, and suggests by address or title', () => {
    const store = open();
    store.recordVisit('https://pubmed.ncbi.nlm.nih.gov/123/', 'Myelin repair in MS');
    store.recordVisit('https://pubmed.ncbi.nlm.nih.gov/123/', 'Myelin repair in MS');
    store.recordVisit('https://example.com/', 'Example Domain');
    expect(store.suggest('myelin')).toEqual([
      {
        url: 'https://pubmed.ncbi.nlm.nih.gov/123/',
        title: 'Myelin repair in MS',
        bookmarked: false,
      },
    ]);
    expect(store.suggest('example.com').map((item) => item.url)).toEqual(['https://example.com/']);
    expect(store.suggest('')).toEqual([]);
  });

  it('puts bookmarks first and survives a restart, with private file modes', () => {
    const store = open();
    store.recordVisit('https://www.ucsf.edu/news', 'UCSF News');
    store.addBookmark('https://www.ucsf.edu/', 'UCSF');
    store.flush();
    const reopened = open();
    expect(reopened.suggest('ucsf').map((item) => [item.url, item.bookmarked])).toEqual([
      ['https://www.ucsf.edu/', true],
      ['https://www.ucsf.edu/news', false],
    ]);
    expect(reopened.isBookmarked('https://www.ucsf.edu/')).toBe(true);
    for (const name of ['history.json', 'bookmarks.json']) {
      expect(fs.statSync(path.join(dir, name)).mode & 0o777).toBe(0o600);
    }
  });

  it('never records a page carrying a one time credential, or a non web page', () => {
    for (const url of [
      'https://app.example/callback?code=abc&state=xyz',
      'https://api.example/x?access_token=secret',
      'https://bucket.example/file?X-Amz-Signature=deadbeef',
      'https://user:pw@example.com/',
      'file:///etc/passwd',
      'about:blank',
    ]) {
      expect(isRecordableUrl(url)).toBe(false);
    }
    const store = open();
    store.recordVisit('https://app.example/callback?code=abc', 'Signing in');
    expect(store.suggest('app.example')).toEqual([]);
  });

  it('forgets history older than 90 days', () => {
    const store = open();
    store.recordVisit('https://old.example/', 'Old');
    now += HISTORY_RETENTION_MS + 1;
    store.recordVisit('https://new.example/', 'New');
    expect(store.suggest('example').map((item) => item.url)).toEqual(['https://new.example/']);
  });

  it('removes a bookmark and clears history on request', () => {
    const store = open();
    store.addBookmark('https://example.com/', 'Example');
    expect(store.addBookmark('https://example.com/', 'Again')).toBe(false);
    expect(store.removeBookmark('https://example.com/')).toBe(true);
    expect(store.bookmarks()).toEqual([]);
    store.recordVisit('https://example.com/', 'Example');
    store.clearHistory();
    expect(open().suggest('example')).toEqual([]);
  });

  it('writes nothing when it does not persist', () => {
    const store = createBrowserStore(dir, { persist: false, now: () => now });
    store.recordVisit('https://example.com/', 'Example');
    store.flush();
    expect(fs.readdirSync(dir)).toEqual([]);
  });
});
