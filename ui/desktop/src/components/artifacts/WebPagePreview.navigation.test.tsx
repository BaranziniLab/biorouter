import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import WebPagePreview, { BROWSER_COPY } from './WebPagePreview';
import type {
  EmbeddedBrowserShortcut,
  EmbeddedBrowserState,
  EmbeddedBrowserSuggestion,
} from '../../browser/types';

/**
 * The Biorouter Browser's bar as a person uses it (docs/desktop-ui/built-in-browser.md):
 * the address field, the notes under the bar, downloads, keys pressed in the page, and the
 * tab's life (BB-01: hidden on unmount, never destroyed by the component).
 */

function stateAt(url: string, extra: Partial<EmbeddedBrowserState> = {}): EmbeddedBrowserState {
  return {
    url,
    title: 'Page',
    managedApp: false,
    sourceRevision: '1:0',
    canGoBack: true,
    canGoForward: false,
    isLoading: false,
    error: null,
    ...extra,
  };
}

function installBrowser(
  initial: EmbeddedBrowserState,
  suggestions: EmbeddedBrowserSuggestion[] = []
) {
  let push: ((payload: { viewId: string; state: EmbeddedBrowserState }) => void) | undefined;
  let shortcut: ((payload: EmbeddedBrowserShortcut) => void) | undefined;
  const browser = {
    create: vi.fn(async () => initial),
    onState: vi.fn((listener: typeof push) => {
      push = listener;
      return () => {};
    }),
    onShortcut: vi.fn((listener: typeof shortcut) => {
      shortcut = listener;
      return () => {};
    }),
    setBounds: vi.fn(async () => {}),
    setVisible: vi.fn(async () => {}),
    release: vi.fn(async () => {}),
    destroy: vi.fn(async () => {}),
    control: vi.fn(async () => true),
    navigate: vi.fn(async () => true),
    clearData: vi.fn(async () => true),
    dismissNotice: vi.fn(async () => {}),
    showDownload: vi.fn(async () => true),
    zoom: vi.fn(async () => 1.1),
    find: vi.fn(async () => true),
    stopFind: vi.fn(async () => {}),
    setBookmark: vi.fn(async () => true),
    bookmarks: vi.fn(async () => [{ url: 'https://www.ucsf.edu/', title: 'UCSF' }]),
    suggest: vi.fn(async () => suggestions),
  };
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: { embeddedBrowser: browser, platform: 'darwin' },
  });
  return {
    browser,
    report: (state: EmbeddedBrowserState) => act(() => push?.({ viewId: 'tab-1', state })),
    key: (action: EmbeddedBrowserShortcut['action']) =>
      act(() => shortcut?.({ viewId: 'tab-1', action })),
  };
}

async function mount(props: Partial<React.ComponentProps<typeof WebPagePreview>> = {}) {
  const onOpenExternal = vi.fn();
  const onOpenInNewTab = vi.fn();
  const onCloseTab = vi.fn();
  let result!: ReturnType<typeof render>;
  await act(async () => {
    result = render(
      <WebPagePreview
        url="https://example.com/"
        viewId="tab-1"
        onOpenExternal={onOpenExternal}
        onOpenInNewTab={onOpenInNewTab}
        onCloseTab={onCloseTab}
        {...props}
      />
    );
  });
  return { ...result, onOpenExternal, onOpenInNewTab, onCloseTab };
}

afterEach(() => vi.clearAllMocks());

describe('the browser tab’s life (BB-01)', () => {
  it('attaches to its tab by id and only hides it on unmount', async () => {
    const { browser } = installBrowser(stateAt('https://example.com/'));
    const { unmount } = await mount();
    expect(browser.create).toHaveBeenCalledWith('tab-1', 'https://example.com/');
    unmount();
    expect(browser.release).toHaveBeenCalledWith('tab-1');
    expect(browser.destroy).not.toHaveBeenCalled();
  });

  it('loads the link again when the person clicks it again, after the tab moved on', async () => {
    const { browser, report } = installBrowser(stateAt('https://example.com/'));
    const { rerender, onOpenExternal } = await mount({ navigateRevision: 1 });
    expect(browser.navigate).not.toHaveBeenCalled();
    await report(stateAt('https://example.com/next'));
    await act(async () => {
      rerender(
        <WebPagePreview
          url="https://example.com/"
          viewId="tab-1"
          onOpenExternal={onOpenExternal}
          navigateRevision={2}
        />
      );
    });
    expect(browser.navigate).toHaveBeenCalledWith('tab-1', 'https://example.com/');
  });
});

describe('the address field', () => {
  it('selects the whole address when it takes focus', async () => {
    installBrowser(stateAt('https://example.com/long/path'));
    await mount();
    const address = screen.getByRole('combobox', {
      name: BROWSER_COPY.address,
    }) as HTMLInputElement;
    await waitFor(() => expect(address.value).toBe('https://example.com/long/path'));
    fireEvent.focus(address);
    expect(address.selectionStart).toBe(0);
    expect(address.selectionEnd).toBe(address.value.length);
  });

  it('opens a typed host over https and searches for anything else', async () => {
    const { browser } = installBrowser(stateAt('https://example.com/'));
    await mount();
    const address = screen.getByRole('combobox', { name: BROWSER_COPY.address });
    await userEvent.clear(address);
    await userEvent.type(address, 'en.wikipedia.org/wiki/Myelin{Enter}');
    expect(browser.navigate).toHaveBeenLastCalledWith(
      'tab-1',
      'https://en.wikipedia.org/wiki/Myelin'
    );
    await userEvent.clear(address);
    await userEvent.type(address, 'myelin repair{Enter}');
    expect(browser.navigate).toHaveBeenLastCalledWith(
      'tab-1',
      'https://www.google.com/search?q=myelin%20repair'
    );
    await userEvent.clear(address);
    await userEvent.type(address, 'localhost:8888{Enter}');
    expect(browser.navigate).toHaveBeenLastCalledWith('tab-1', 'http://localhost:8888/');
  });

  it('offers the person’s own history and hides the page while the list is up', async () => {
    const { browser } = installBrowser(stateAt('https://example.com/'), [
      { url: 'https://pubmed.ncbi.nlm.nih.gov/1/', title: 'Myelin paper', bookmarked: false },
    ]);
    await mount();
    const address = screen.getByRole('combobox', { name: BROWSER_COPY.address });
    await userEvent.clear(address);
    await userEvent.type(address, 'myel');
    const option = await screen.findByRole('option', { name: /Myelin paper/ });
    expect(browser.setVisible).toHaveBeenLastCalledWith('tab-1', false);
    await userEvent.click(option);
    expect(browser.navigate).toHaveBeenLastCalledWith(
      'tab-1',
      'https://pubmed.ncbi.nlm.nih.gov/1/'
    );
  });

  it('takes focus when the person presses Cmd L in the page', async () => {
    const { key } = installBrowser(stateAt('https://example.com/'));
    await mount();
    await key('focus-address');
    expect(document.activeElement).toBe(
      screen.getByRole('combobox', { name: BROWSER_COPY.address })
    );
  });
});

describe('notes, errors and downloads under the bar', () => {
  it('opens a blocked popup in a new tab only when the person asks', async () => {
    const { report, browser } = installBrowser(stateAt('https://example.com/'));
    const { onOpenInNewTab } = await mount();
    await report(
      stateAt('https://example.com/', {
        notice: {
          kind: 'popup-blocked',
          text: 'This page tried to open a new window.',
          action: { label: 'Open in a new tab', kind: 'open-tab', url: 'https://example.com/p' },
        },
      })
    );
    expect(screen.getByTestId('embedded-browser-notice')).toHaveTextContent(
      'This page tried to open a new window.'
    );
    await userEvent.click(screen.getByRole('button', { name: 'Open in a new tab' }));
    expect(onOpenInNewTab).toHaveBeenCalledWith('https://example.com/p');
    expect(browser.dismissNotice).toHaveBeenCalledWith('tab-1');
  });

  it('opens a refused local address here, as the person’s own navigation', async () => {
    const { report, browser } = installBrowser(stateAt('https://example.com/'));
    await mount();
    await report(
      stateAt('https://example.com/', {
        notice: {
          kind: 'local-blocked',
          text: 'This page tried to open a local address, localhost:8888.',
          action: { label: 'Open it', kind: 'open-here', url: 'http://localhost:8888/' },
        },
      })
    );
    await userEvent.click(screen.getByRole('button', { name: 'Open it' }));
    expect(browser.navigate).toHaveBeenCalledWith('tab-1', 'http://localhost:8888/');
  });

  it('offers the default browser for a certificate error', async () => {
    const { report } = installBrowser(stateAt('https://self-signed.example/'));
    const { onOpenExternal } = await mount({ url: 'https://self-signed.example/' });
    await report(
      stateAt('https://self-signed.example/', {
        error: 'The connection to self-signed.example is not secure.',
        errorOffersExternal: true,
      })
    );
    await userEvent.click(screen.getByRole('button', { name: BROWSER_COPY.openInDefaultBrowser }));
    expect(onOpenExternal).toHaveBeenCalledWith('https://self-signed.example/');
  });

  it('shows a download’s progress and then where it went', async () => {
    const { report, browser } = installBrowser(stateAt('https://example.com/'));
    await mount();
    await report(
      stateAt('https://example.com/', {
        download: {
          id: 'download-1',
          filename: 'data.csv',
          receivedBytes: 50,
          totalBytes: 200,
          state: 'progressing',
        },
      })
    );
    expect(screen.getByTestId('embedded-browser-download')).toHaveTextContent(
      'Downloading data.csv, 25%'
    );
    await report(
      stateAt('https://example.com/', {
        download: {
          id: 'download-1',
          filename: 'data.csv',
          receivedBytes: 200,
          totalBytes: 200,
          state: 'completed',
        },
      })
    );
    await userEvent.click(screen.getByRole('button', { name: 'Reveal in Finder' }));
    expect(browser.showDownload).toHaveBeenCalledWith('tab-1');
  });

  it('marks a local page in the bar', async () => {
    installBrowser(stateAt('http://localhost:8888/lab', { isLocal: true }));
    await mount({ url: 'http://localhost:8888/lab' });
    expect(screen.getByTestId('browser-local-chip')).toHaveTextContent(BROWSER_COPY.local);
  });
});

describe('the bar’s ⋯ menu and keys', () => {
  it('hides the page while the menu is open, and zooms, bookmarks and finds from it', async () => {
    const { browser } = installBrowser(stateAt('https://example.com/'));
    await mount();
    await userEvent.click(screen.getByRole('button', { name: BROWSER_COPY.more }));
    expect(browser.setVisible).toHaveBeenLastCalledWith('tab-1', false);
    await userEvent.click(await screen.findByRole('menuitem', { name: BROWSER_COPY.zoomIn }));
    expect(browser.zoom).toHaveBeenCalledWith('tab-1', 'in');
    await waitFor(() => expect(browser.setVisible).toHaveBeenLastCalledWith('tab-1', true));

    await userEvent.click(screen.getByRole('button', { name: BROWSER_COPY.more }));
    await userEvent.click(await screen.findByRole('menuitem', { name: BROWSER_COPY.addBookmark }));
    expect(browser.setBookmark).toHaveBeenCalledWith('tab-1', true);

    await userEvent.click(screen.getByRole('button', { name: BROWSER_COPY.more }));
    await userEvent.click(await screen.findByRole('menuitem', { name: BROWSER_COPY.findInPage }));
    const find = await screen.findByRole('textbox', { name: BROWSER_COPY.findInPage });
    await userEvent.type(find, 'myelin');
    expect(browser.find).toHaveBeenLastCalledWith('tab-1', 'myelin', {
      forward: true,
      findNext: false,
    });
    await userEvent.type(find, '{Escape}');
    expect(browser.stopFind).toHaveBeenCalledWith('tab-1');
  });

  it('closes its panel tab when the person presses Cmd W in the page', async () => {
    const { key } = installBrowser(stateAt('https://example.com/'));
    const { onCloseTab } = await mount();
    await key('close-tab');
    expect(onCloseTab).toHaveBeenCalled();
  });

  it('opens the find field when the person presses Cmd F in the page', async () => {
    const { key } = installBrowser(stateAt('https://example.com/'));
    await mount();
    await key('find');
    expect(await screen.findByRole('textbox', { name: BROWSER_COPY.findInPage })).toBeVisible();
  });
});
