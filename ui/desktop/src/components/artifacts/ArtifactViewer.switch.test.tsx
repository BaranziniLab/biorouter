import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import { AppTooltipLayer } from '../ui/AppTooltipLayer';
import ArtifactViewer from './ArtifactViewer';
import type { ArtifactFilePreview, ArtifactSource } from './artifactTypes';
import { PREVIEW_COPY } from './copy';

/**
 * What a tab switch looks like from the bar and from the strip (spec 3.8):
 * a tab that has been seen never shows a loading state again, a tab that is
 * closed is forgotten, the bar belongs to the tab being shown while the old one
 * fades out, and the strip's names are the files', not their paths.
 */

type Read = (path: string) => Promise<ArtifactFilePreview>;

function textPreview(path: string): ArtifactFilePreview {
  const title = path.split('/').pop() ?? path;
  return {
    kind: 'text',
    title,
    path,
    mimeType: 'text/plain',
    text: `Preview of ${title}\nsecond line`,
    size: 32,
    found: true,
  };
}

function installElectron(read: Read) {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      readArtifactFile: vi.fn(read),
      prepareArtifactHtml: vi.fn(async ({ html }: { html: string }) => ({ html })),
      openDirectoryInExplorer: vi.fn(),
      openExternal: vi.fn(),
      openArtifactInBrowser: vi.fn(),
    },
  });
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: vi.fn().mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  });
}

const file = (title: string): ArtifactSource => ({
  kind: 'file',
  title,
  path: `/work/${title}`,
});

const viewer = (artifact: ArtifactSource) => (
  <ThemeProvider>
    <AppTooltipLayer />
    <ArtifactViewer artifact={artifact} onClose={vi.fn()} onOpenArtifact={vi.fn()} />
  </ThemeProvider>
);

/** A read the test settles by hand. */
function deferred() {
  let resolve!: (value: ArtifactFilePreview) => void;
  const promise = new Promise<ArtifactFilePreview>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const barCopyButtons = () =>
  document.querySelectorAll('.br-preview-bar [data-testid="artifact-copy"]');

describe('ArtifactViewer: switching tabs', () => {
  beforeEach(() => {
    installElectron(async (path) => textPreview(path));
  });

  afterEach(() => {
    // The crossfade test gives Element a Web Animations stub.
    Reflect.deleteProperty(Element.prototype, 'animate');
  });

  it('shows a tab that has been seen at once, without a loading state, while it re-reads', async () => {
    const user = userEvent.setup();
    const { rerender } = render(viewer(file('a.txt')));
    await screen.findByText('Preview of a.txt', { exact: false });
    rerender(viewer(file('b.txt')));
    await screen.findByText('Preview of b.txt', { exact: false });

    // Going back to a.txt re-reads it, and that read never answers in this test.
    const slow = deferred();
    (window.electron.readArtifactFile as ReturnType<typeof vi.fn>).mockImplementation(
      () => slow.promise
    );
    await user.click(screen.getByRole('tab', { name: 'a.txt' }));

    expect(screen.getByText('Preview of a.txt', { exact: false })).toBeInTheDocument();
    expect(document.querySelector('[data-preview-loading]')).toBeNull();
    // The bar still names what is shown.
    expect(document.querySelector('.br-preview-bar__name')).toHaveTextContent('a.txt');
    await act(async () => {
      slow.resolve(textPreview('/work/a.txt'));
    });
  });

  it('forgets a closed tab: opening it again starts from the quiet skeleton, not a stale preview', async () => {
    const user = userEvent.setup();
    const { rerender } = render(viewer(file('a.txt')));
    await screen.findByText('Preview of a.txt', { exact: false });
    rerender(viewer(file('b.txt')));
    await screen.findByText('Preview of b.txt', { exact: false });

    await user.click(screen.getByRole('button', { name: PREVIEW_COPY.closeTab('b.txt') }));
    await waitFor(() => expect(screen.queryByRole('tab', { name: 'b.txt' })).toBeNull());

    const slow = deferred();
    (window.electron.readArtifactFile as ReturnType<typeof vi.fn>).mockImplementation(
      () => slow.promise
    );
    rerender(viewer(file('b.txt')));
    await screen.findByRole('tab', { name: 'b.txt' });

    expect(document.querySelector('[data-preview-loading]')).not.toBeNull();
    expect(screen.queryByText('Preview of b.txt', { exact: false })).toBeNull();
    await act(async () => {
      slow.resolve(textPreview('/work/b.txt'));
    });
    await screen.findByText('Preview of b.txt', { exact: false });
  });

  it('puts only the shown tab’s Copy and counts in the bar while the old tab fades out', async () => {
    // A real engine animates the switch; jsdom has no Web Animations API, so give
    // it one. That is what turns the outgoing layer on.
    Object.defineProperty(Element.prototype, 'animate', {
      configurable: true,
      value: () => ({
        cancel: () => undefined,
        onfinish: null,
        playState: 'running',
        finished: Promise.resolve(),
      }),
    });
    const { rerender } = render(viewer(file('a.txt')));
    const oldText = await screen.findByText('Preview of a.txt', { exact: false });
    expect(barCopyButtons()).toHaveLength(1);

    const slow = deferred();
    (window.electron.readArtifactFile as ReturnType<typeof vi.fn>).mockImplementation(
      () => slow.promise
    );
    rerender(viewer(file('b.txt')));
    await screen.findByRole('tab', { name: 'b.txt' });

    // The old tab is still fading out beneath the new one…
    expect(document.querySelector('[data-preview-layer="outgoing"]')).not.toBeNull();
    // …as the very DOM it was showing, not a copy rebuilt for the exit: a frame
    // inside it would reload and a page would re-render while it faded.
    expect(oldText.isConnected).toBe(true);
    expect(oldText.closest('[data-preview-layer="outgoing"]')).not.toBeNull();
    // …but the bar is already the new tab's: it names it and holds nothing of a.txt's.
    expect(document.querySelector('.br-preview-bar__name')).toHaveTextContent('b.txt');
    expect(barCopyButtons()).toHaveLength(0);
    expect(
      document.querySelectorAll('.br-preview-bar [data-testid="artifact-strip-count"]')
    ).toHaveLength(0);

    await act(async () => {
      slow.resolve(textPreview('/work/b.txt'));
    });
    await waitFor(() => expect(barCopyButtons()).toHaveLength(1));
    expect(
      document.querySelectorAll('.br-preview-bar [data-testid="artifact-strip-count"]')
    ).toHaveLength(1);
  });
});

describe('ArtifactViewer: the tab strip and the header menu', () => {
  beforeEach(() => {
    installElectron(async (path) => textPreview(path));
  });

  it('names a tab by its file when the app tooltip layer turns the path title into a tooltip', async () => {
    const { rerender } = render(viewer(file('chart-a.html')));
    rerender(viewer(file('summary.md')));
    const tab = await screen.findByRole('tab', { name: 'summary.md' });
    // Let the layer's mutation observer run: it used to copy the `title` (the full
    // path) into the aria-label of any control that had none.
    await act(async () => {
      await Promise.resolve();
    });
    expect(tab).toHaveAccessibleName('summary.md');
    expect(screen.getByRole('tab', { name: 'chart-a.html' })).toHaveAccessibleName('chart-a.html');
  });

  it('offers Quote selection in the More actions menu only while text is selected', async () => {
    const user = userEvent.setup();
    render(
      <ThemeProvider>
        <ArtifactViewer
          artifact={file('notes.txt')}
          sessionId="session-1"
          onClose={vi.fn()}
          onOpenArtifact={vi.fn()}
        />
      </ThemeProvider>
    );
    const text = await screen.findByText('Preview of notes.txt', { exact: false });

    await user.click(screen.getByRole('button', { name: PREVIEW_COPY.moreActions }));
    expect(screen.queryByTestId('artifact-quote-menu-item')).toBeNull();
    await user.keyboard('{Escape}');

    // Select the text inside the preview, as a pointer drag or the keyboard would.
    const range = document.createRange();
    range.selectNodeContents(text);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    await act(async () => {
      document.dispatchEvent(new Event('selectionchange'));
    });

    // The header icon appears with the selection, and so does the menu's item.
    expect(await screen.findByTestId('artifact-quote')).toBeInTheDocument();
    fireEvent.pointerDown(screen.getByRole('button', { name: PREVIEW_COPY.moreActions }), {
      button: 0,
      ctrlKey: false,
    });
    expect(await screen.findByTestId('artifact-quote-menu-item')).toHaveTextContent(
      PREVIEW_COPY.quoteSelection
    );
    selection?.removeAllRanges();
  });
});
