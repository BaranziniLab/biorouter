import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../toasts', () => ({ toastError: vi.fn() }));

import { toastError } from '../toasts';
import { OPEN_IN_BROWSER_COPY, openInBrowser, previewOpenTarget } from './openInBrowser';

const openExternal = vi.fn();

beforeEach(() => {
  (window as unknown as { electron: unknown }).electron = { openExternal };
});
afterEach(() => {
  vi.clearAllMocks();
});

describe('openInBrowser', () => {
  it('says why an app link could not be opened, in the main process’s words', async () => {
    openExternal.mockResolvedValueOnce({ ok: false, message: 'This app no longer exists.' });
    await openInBrowser('http://127.0.0.1:51851/apps/cohort-explorer/');
    expect(openExternal).toHaveBeenCalledWith('http://127.0.0.1:51851/apps/cohort-explorer/');
    expect(toastError).toHaveBeenCalledWith({
      title: OPEN_IN_BROWSER_COPY.failedTitle,
      msg: 'This app no longer exists.',
    });
  });

  it.each([undefined, { ok: true }])('stays quiet when the main process answers %j', async (r) => {
    openExternal.mockResolvedValueOnce(r);
    await openInBrowser('https://example.test/');
    expect(toastError).not.toHaveBeenCalled();
  });

  it('reports a failure it was given no words for, and a bridge that failed', async () => {
    openExternal.mockResolvedValueOnce({ ok: false });
    await openInBrowser('http://127.0.0.1:1/apps/a/');
    openExternal.mockRejectedValueOnce(new Error('ipc gone'));
    await openInBrowser('http://127.0.0.1:1/apps/a/');
    expect(vi.mocked(toastError).mock.calls.map(([toast]) => toast.msg)).toEqual([
      OPEN_IN_BROWSER_COPY.failed,
      OPEN_IN_BROWSER_COPY.bridgeFailed,
    ]);
  });
});

/**
 * A preview's address bar is the page's to set: `history.replaceState` moves it without a
 * navigation, and a failed load commits the address it failed on. App A's code could put
 * `/apps/B/` there, and "Open in browser" then minted B's launch link and opened B.
 */
describe('previewOpenTarget', () => {
  const appA = 'http://127.0.0.1:51851/apps/app-a/';
  const appB = 'http://127.0.0.1:51851/apps/app-b/';

  it('opens the app a managed preview was opened for, wherever its page says it is', () => {
    expect(previewOpenTarget(appA, { url: appB, managedApp: true })).toBe(appA);
    expect(previewOpenTarget(appA, { url: `${appA}?view=2`, managedApp: true })).toBe(appA);
  });

  it('never lets an ordinary page name an app on this machine', () => {
    const site = 'https://example.org/start';
    expect(previewOpenTarget(site, { url: 'http://127.0.0.1:1/apps/app-b/' })).toBe(site);
    expect(previewOpenTarget(site, { url: 'http://[::1]:1/apps/app-b/' })).toBe(site);
  });

  it('opens where an ordinary page is now, including the app link it was opened at', () => {
    const site = 'https://example.org/start';
    expect(previewOpenTarget(site, { url: 'https://example.org/next' })).toBe(
      'https://example.org/next'
    );
    expect(previewOpenTarget(appA, { url: appA })).toBe(appA);
  });

  it('opens where it was opened before the page has reported anything', () => {
    expect(previewOpenTarget(appA, null)).toBe(appA);
    expect(previewOpenTarget(appA, { url: '' })).toBe(appA);
  });
});
