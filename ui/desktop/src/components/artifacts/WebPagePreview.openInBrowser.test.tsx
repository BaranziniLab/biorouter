import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import WebPagePreview, { type EmbeddedBrowserState } from './WebPagePreview';

/**
 * The toolbar's "Open in browser" used to hand over the view's current address. A previewed app
 * sets that itself (`history.replaceState(null, '', '/apps/other/')` fires no navigation guard),
 * and a link to an app on this machine is opened with a fresh launch link: the page chose which
 * app the person's browser was given.
 */
const appA = 'http://127.0.0.1:51851/apps/app-a/';
const appB = 'http://127.0.0.1:51851/apps/app-b/';

function stateAt(url: string, managedApp: boolean): EmbeddedBrowserState {
  return {
    url,
    title: 'App',
    managedApp,
    sourceRevision: '1:0',
    canGoBack: false,
    canGoForward: false,
    isLoading: false,
    error: null,
  };
}

function installBrowser(initial: EmbeddedBrowserState) {
  let push: ((payload: { viewId: string; state: EmbeddedBrowserState }) => void) | undefined;
  let viewId = '';
  const browser = {
    create: vi.fn(async (id: string) => {
      viewId = id;
      return initial;
    }),
    onState: vi.fn((listener: typeof push) => {
      push = listener;
      return () => {};
    }),
    setBounds: vi.fn(async () => {}),
    setVisible: vi.fn(async () => {}),
    destroy: vi.fn(async () => {}),
    control: vi.fn(async () => true),
    navigate: vi.fn(async () => {}),
    clearData: vi.fn(async () => true),
  };
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: { embeddedBrowser: browser },
  });
  return {
    browser,
    report: (state: EmbeddedBrowserState) => act(() => push?.({ viewId, state })),
  };
}

afterEach(() => vi.clearAllMocks());

describe('the live preview’s Open in browser', () => {
  it('opens the app a managed preview shows, not the address its page reports', async () => {
    const { browser, report } = installBrowser(stateAt(appA, true));
    const onOpenExternal = vi.fn();
    await act(async () => {
      render(<WebPagePreview url={appA} requireManagedApp onOpenExternal={onOpenExternal} />);
    });
    expect(browser.create).toHaveBeenCalled();

    await report(stateAt(appB, true));
    await userEvent.click(await screen.findByRole('button', { name: 'Open in browser' }));
    expect(onOpenExternal).toHaveBeenCalledWith(appA);
  });

  it('opens an ordinary page where it is now, but never on an app it moved itself to', async () => {
    const site = 'https://example.org/start';
    const { browser, report } = installBrowser(stateAt(site, false));
    const onOpenExternal = vi.fn();
    await act(async () => {
      render(<WebPagePreview url={site} onOpenExternal={onOpenExternal} />);
    });
    expect(browser.create).toHaveBeenCalled();
    const open = await screen.findByRole('button', { name: 'Open in browser' });

    await report(stateAt('https://example.org/next', false));
    await userEvent.click(open);
    expect(onOpenExternal).toHaveBeenLastCalledWith('https://example.org/next');

    await report(stateAt('http://127.0.0.1:1/apps/app-b/', false));
    await userEvent.click(open);
    expect(onOpenExternal).toHaveBeenLastCalledWith(site);
  });
});
