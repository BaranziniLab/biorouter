import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import ArtifactViewer from './ArtifactViewer';
import type { ArtifactSource } from './artifactTypes';
import { ANNOTATE_NEEDS_DESKTOP_REASON } from './captureOnBrowser';
import { PREVIEW_COPY } from './copy';

/**
 * A figure's bar and header (spec 3.8, vision finding V2-16).
 *
 * The bar used to repeat the tab's title and nothing else, and the header's ⋯
 * held one item. A `ui://` artifact now says what it is in its transcript card's
 * word (Figure, Report, App), and where ⋯ would hold only "Send a region to the
 * chat" that action stands in the header as an icon. A file keeps the menu: it
 * also reveals itself.
 */

const html = '<!doctype html><html><body><h1>Plot</h1></body></html>';

const figure = (sourceUri?: string): ArtifactSource => ({
  kind: 'html',
  title: 'Donut Chart',
  html,
  sourceUri,
});

function installElectron() {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    writable: true,
    value: {
      readArtifactFile: vi.fn(async () => ({
        kind: 'text',
        title: 'notes.txt',
        path: '/work/notes.txt',
        mimeType: 'text/plain',
        text: 'hello',
        size: 5,
        found: true,
      })),
      prepareArtifactHtml: vi.fn(async ({ html: source }: { html: string }) => ({ html: source })),
      openDirectoryInExplorer: vi.fn(),
      openExternal: vi.fn(),
      openArtifactInBrowser: vi.fn(),
      captureRegion: vi.fn(async () => ({ path: '/tmp/region.png', width: 200, height: 120 })),
      platform: 'darwin',
    },
  });
}

const mount = (artifact: ArtifactSource, sessionId: string | null = 'session-1') =>
  render(
    <ThemeProvider>
      <ArtifactViewer
        artifact={artifact}
        onClose={vi.fn()}
        onOpenArtifact={vi.fn()}
        sessionId={sessionId ?? undefined}
      />
    </ThemeProvider>
  );

const bar = () => screen.getByTestId('artifact-status-strip');
const more = () => screen.queryByRole('button', { name: PREVIEW_COPY.moreActions });

beforeEach(() => {
  installElectron();
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: vi.fn().mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  });
});

afterEach(() => {
  cleanup();
  delete document.documentElement.dataset.biorouterSurface;
});

describe('a figure in the panel', () => {
  it.each([
    ['a figure', 'ui://autovisualiser/donut', 'Figure'],
    ['a report', 'ui://dashboard/report', 'Report'],
    ['an app', 'ui://agent-drafter/app-1', 'App'],
    ['a figure with no address', undefined, 'Figure'],
  ])('has %s say what it is in the bar, in its card’s word', async (_name, uri, word) => {
    mount(figure(uri));
    const meta = await screen.findByTestId('artifact-kind-meta');
    expect(bar()).toContainElement(meta);
    expect(meta).toHaveTextContent(new RegExp(`^${word}$`));
    // The bar still names the artifact once, as the tab does.
    expect(within(bar()).getByText('Donut Chart')).toBeInTheDocument();
  });

  it('puts Send a region in the header as an icon, with no menu of one', async () => {
    const user = userEvent.setup();
    mount(figure('ui://autovisualiser/donut'));
    const button = await screen.findByRole('button', { name: PREVIEW_COPY.annotate });
    expect(button).toHaveAttribute('data-testid', 'artifact-annotate');
    expect(more()).toBeNull();

    await user.click(button);
    expect(screen.getByTestId('annotation-overlay')).toBeInTheDocument();
    // The same control, in the same place, is now the pressed tool; pressing it
    // again cancels, and there is never a second camera in the header.
    const pressed = screen.getByRole('button', { name: PREVIEW_COPY.cancelAnnotate });
    expect(pressed).toHaveAttribute('data-testid', 'artifact-annotate-cancel');
    expect(pressed).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getAllByTestId(/^artifact-annotate/)).toHaveLength(1);

    await user.click(pressed);
    await waitFor(() => expect(screen.queryByTestId('annotation-overlay')).toBeNull());
    expect(screen.getByRole('button', { name: PREVIEW_COPY.annotate })).toBeInTheDocument();
  });

  it('offers neither the icon nor a menu where there is no chat to send a region to', async () => {
    mount(figure('ui://autovisualiser/donut'), null);
    await screen.findByTestId('artifact-viewer');
    expect(screen.queryByRole('button', { name: PREVIEW_COPY.annotate })).toBeNull();
    expect(more()).toBeNull();
  });

  it('keeps the menu, and the reason, where the action cannot work (a browser)', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const user = userEvent.setup();
    mount(figure('ui://autovisualiser/donut'));
    await user.click(await screen.findByRole('button', { name: PREVIEW_COPY.moreActions }));
    const item = await screen.findByTestId('artifact-annotate');
    expect(item).toHaveAttribute('aria-disabled', 'true');
    expect(item).toHaveTextContent(ANNOTATE_NEEDS_DESKTOP_REASON);
  });
});

describe('a file in the panel', () => {
  it('keeps the menu, because it holds a second item', async () => {
    const user = userEvent.setup();
    mount({ kind: 'file', title: 'notes.txt', path: '/work/notes.txt' });
    await user.click(await screen.findByRole('button', { name: PREVIEW_COPY.moreActions }));
    expect(await screen.findByTestId('artifact-annotate')).toBeInTheDocument();
    expect(screen.getByTestId('artifact-reveal')).toBeInTheDocument();
    // A file says nothing a figure would: its bar's meta is its own.
    expect(screen.queryByTestId('artifact-kind-meta')).toBeNull();
  });
});
