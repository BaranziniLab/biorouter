import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import ArtifactViewer from './ArtifactViewer';
import type { ArtifactSource } from './artifactTypes';
import { withLinkClickOrigin } from './browserLinks';
import type { EmbeddedBrowserOpened, EmbeddedBrowserState } from '../../browser/types';

/**
 * The Biorouter Browser in the panel (docs/desktop-ui/built-in-browser.md):
 *
 * - BB-02: a link the person clicked in a message loads at once; an agent supplied link (an MCP
 *   resource with an http URI) still shows the Open here card and loads nothing.
 * - BB-04: a page's `target=_blank` tab arrives from main and becomes a panel tab, but only for
 *   a page this panel shows.
 * - BB-01: closing a panel tab or the panel ends its browser tab; switching tabs keeps it.
 */

function pageState(url: string, title = 'Page'): EmbeddedBrowserState {
  return {
    url,
    title,
    managedApp: false,
    sourceRevision: '1:0',
    canGoBack: false,
    canGoForward: false,
    isLoading: false,
    error: null,
  };
}

function installBrowser() {
  let opened: ((payload: EmbeddedBrowserOpened) => void) | undefined;
  let push: ((payload: { viewId: string; state: EmbeddedBrowserState }) => void) | undefined;
  const browser = {
    isManagedAppUrl: vi.fn(async () => false),
    create: vi.fn(async (_viewId: string, url: string) =>
      pageState(url, `Page at ${new URL(url).hostname}`)
    ),
    onState: vi.fn((listener: typeof push) => {
      push = listener;
      return () => {};
    }),
    onOpened: vi.fn((listener: typeof opened) => {
      opened = listener;
      return () => {};
    }),
    onShortcut: vi.fn(() => () => {}),
    claim: vi.fn(async () => true),
    setBounds: vi.fn(async () => {}),
    setVisible: vi.fn(async () => {}),
    release: vi.fn(async () => {}),
    destroy: vi.fn(async () => {}),
    control: vi.fn(async () => true),
    navigate: vi.fn(async () => true),
    clearData: vi.fn(async () => true),
    suggest: vi.fn(async () => []),
  };
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: { embeddedBrowser: browser, openExternal: vi.fn(), platform: 'darwin' },
  });
  return {
    browser,
    open: (payload: EmbeddedBrowserOpened) => act(() => opened?.(payload)),
    report: (viewId: string, state: EmbeddedBrowserState) => act(() => push?.({ viewId, state })),
  };
}

function renderViewer(artifact: ArtifactSource, onOpenArtifact = vi.fn()) {
  const onClose = vi.fn();
  const view = (next: ArtifactSource) => (
    <ThemeProvider>
      <ArtifactViewer artifact={next} onClose={onClose} onOpenArtifact={onOpenArtifact} />
    </ThemeProvider>
  );
  const result = render(view(artifact));
  return {
    ...result,
    rerenderWith: (next: ArtifactSource) => result.rerender(view(next)),
    onClose,
  };
}

afterEach(() => vi.clearAllMocks());

describe('links into the Biorouter Browser (BB-02)', () => {
  it('loads a link the person clicked at once, with no Open here card', async () => {
    const { browser } = installBrowser();
    renderViewer(
      withLinkClickOrigin({ kind: 'externalUrl', title: 'Docs', url: 'https://example.com/docs' })
    );
    await waitFor(() =>
      expect(browser.create).toHaveBeenCalledWith(
        expect.stringMatching(/^browser-artifact-tab-/),
        'https://example.com/docs'
      )
    );
    expect(screen.queryByTestId('artifact-open-here')).not.toBeInTheDocument();
  });

  it('holds an agent supplied link behind the Open here card', async () => {
    const { browser } = installBrowser();
    renderViewer({ kind: 'externalUrl', title: 'From a tool', url: 'https://example.com/x' });
    expect(await screen.findByTestId('artifact-open-here')).toBeInTheDocument();
    expect(browser.create).not.toHaveBeenCalled();
  });

  it('cannot be talked into loading by an artifact that names a view nobody opened', async () => {
    const { browser } = installBrowser();
    renderViewer({
      kind: 'externalUrl',
      title: 'Forged',
      url: 'https://evil.example/',
      viewId: 'page-tab-99',
    });
    expect(await screen.findByTestId('artifact-open-here')).toBeInTheDocument();
    expect(browser.create).not.toHaveBeenCalled();
  });

  it('names a live page’s tab by its page title', async () => {
    const { browser, report } = installBrowser();
    renderViewer(
      withLinkClickOrigin({
        kind: 'externalUrl',
        title: 'https://example.com/',
        url: 'https://example.com/',
      })
    );
    await waitFor(() => expect(browser.create).toHaveBeenCalled());
    const viewId = browser.create.mock.calls[0][0];
    await report(viewId, pageState('https://example.com/', 'Example Domain'));
    expect(await screen.findByRole('tab', { name: 'Example Domain' })).toBeInTheDocument();
  });
});

describe('tabs a page opens (BB-04) and the tabs’ lives (BB-01)', () => {
  it('shows a page’s new tab as a panel tab of its own and keeps it', async () => {
    const { browser, open } = installBrowser();
    const onOpenArtifact = vi.fn();
    renderViewer(
      withLinkClickOrigin({ kind: 'externalUrl', title: 'Docs', url: 'https://example.com/docs' }),
      onOpenArtifact
    );
    await waitFor(() => expect(browser.create).toHaveBeenCalled());
    const opener = browser.create.mock.calls[0][0];

    await open({
      openerViewId: opener,
      viewId: 'page-tab-1',
      url: 'https://example.org/b',
      foreground: true,
    });
    expect(browser.claim).toHaveBeenCalledWith('page-tab-1');
    // Shown at once, and named by its page once the page reports a title.
    expect(await screen.findByRole('tab', { name: 'Page at example.org' })).toHaveAttribute(
      'aria-selected',
      'true'
    );
    await waitFor(() =>
      expect(browser.create).toHaveBeenCalledWith('page-tab-1', 'https://example.org/b')
    );
    // Switching away hides the first tab's page; it is not destroyed.
    expect(browser.release).toHaveBeenCalledWith(opener);
    expect(browser.destroy).not.toHaveBeenCalled();
  });

  it('adds a background tab without showing it', async () => {
    const { browser, open } = installBrowser();
    renderViewer(
      withLinkClickOrigin({ kind: 'externalUrl', title: 'Docs', url: 'https://example.com/docs' })
    );
    await waitFor(() => expect(browser.create).toHaveBeenCalled());
    await open({
      openerViewId: browser.create.mock.calls[0][0],
      viewId: 'page-tab-2',
      url: 'https://example.org/c',
      foreground: false,
    });
    expect(await screen.findByRole('tab', { name: 'example.org' })).toHaveAttribute(
      'aria-selected',
      'false'
    );
    expect(browser.claim).toHaveBeenCalledWith('page-tab-2');
  });

  it('ignores a tab opened by a page another panel shows', async () => {
    const { browser, open } = installBrowser();
    renderViewer(
      withLinkClickOrigin({ kind: 'externalUrl', title: 'Docs', url: 'https://example.com/docs' })
    );
    await waitFor(() => expect(browser.create).toHaveBeenCalled());
    await open({
      openerViewId: 'browser-artifact-tab-somebody-else',
      viewId: 'page-tab-3',
      url: 'https://example.org/d',
      foreground: true,
    });
    expect(browser.claim).not.toHaveBeenCalled();
    expect(screen.queryByRole('tab', { name: 'example.org' })).not.toBeInTheDocument();
  });

  it('ends a browser tab when its panel tab closes, and every one when the panel goes', async () => {
    const { browser, open } = installBrowser();
    const { unmount } = renderViewer(
      withLinkClickOrigin({ kind: 'externalUrl', title: 'Docs', url: 'https://example.com/docs' })
    );
    await waitFor(() => expect(browser.create).toHaveBeenCalled());
    const opener = browser.create.mock.calls[0][0];
    await open({
      openerViewId: opener,
      viewId: 'page-tab-4',
      url: 'https://example.org/e',
      foreground: true,
    });
    await screen.findByRole('tab', { name: 'Page at example.org' });

    await userEvent.click(screen.getByRole('button', { name: 'Close Page at example.org' }));
    expect(browser.destroy).toHaveBeenCalledWith('page-tab-4');

    unmount();
    expect(browser.destroy).toHaveBeenCalledWith(opener);
  });
});
