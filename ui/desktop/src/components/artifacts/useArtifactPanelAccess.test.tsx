import { render, waitFor } from '@testing-library/react';
import { useRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ArtifactSource } from './artifactTypes';
import { panelAccessFor, resetPanelAccessRegistry } from './panelAccessRegistry';
import type { LiveBrowserShare } from './WebPagePreview';
import { useArtifactPanelAccess } from './useArtifactPanelAccess';
import { ARTIFACT_PANEL_ATTR } from '../../utils/tabCycle';

const pdfMocks = vi.hoisted(() => {
  const getTextContent = vi.fn(async () => ({ items: [{ str: 'PDF report' }] }));
  const cleanup = vi.fn();
  const destroy = vi.fn(async () => undefined);
  return {
    getTextContent,
    cleanup,
    destroy,
    getDocument: vi.fn(() => ({
      promise: Promise.resolve({
        numPages: 1,
        getPage: async () => ({ getTextContent, cleanup }),
      }),
      destroy,
    })),
  };
});

vi.mock('pdfjs-dist/legacy/build/pdf.mjs', () => ({
  getDocument: pdfMocks.getDocument,
  GlobalWorkerOptions: { workerPort: null },
}));
vi.mock('../../utils/pdfCompat', () => ({ createPdfWorker: () => null }));

const artifact: ArtifactSource = {
  kind: 'externalUrl',
  title: 'Example Domain',
  url: 'https://example.com/',
};

function Harness({
  share,
  shownArtifact = artifact,
  isOpen = true,
  fileSourceRevision = null,
}: {
  share: LiveBrowserShare | null;
  shownArtifact?: ArtifactSource;
  isOpen?: boolean;
  fileSourceRevision?: string | null;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  useArtifactPanelAccess({
    sessionId: 'session-web',
    artifact: shownArtifact,
    isOpen,
    liveBrowserShare: share,
    panelRootRef: rootRef,
    fileSourceRevision,
  });
  return (
    <div ref={rootRef}>
      <div {...{ [ARTIFACT_PANEL_ATTR]: '' }} />
    </div>
  );
}

function liveShare(url: string, title: string, sourceRevision: string): LiveBrowserShare {
  return {
    viewId: 'embedded-browser-test',
    state: {
      url,
      title,
      sourceRevision,
      canGoBack: true,
      canGoForward: false,
      isLoading: false,
      error: null,
    },
  };
}

beforeEach(() => {
  resetPanelAccessRegistry();
  vi.clearAllMocks();
  pdfMocks.getTextContent.mockResolvedValue({ items: [{ str: 'PDF report' }] });
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      embeddedBrowser: {
        readText: vi.fn(async () => ({
          url: 'https://www.iana.org/help/example-domains',
          title: 'Example Domains',
          sourceRevision: '42:2',
          text: 'Example Domains',
          truncated: false,
        })),
        capture: vi.fn(async () => ({
          path: '/tmp/live-capture.png',
          width: 800,
          height: 600,
          sourceRevision: '42:1',
        })),
      },
      readArtifactFile: vi.fn(),
      captureRegion: vi.fn(),
      deleteTempFile: vi.fn(),
    },
  });
});

describe('live web panel access', () => {
  it('tracks the page the user navigated to instead of the original artifact URL', async () => {
    const { rerender } = render(
      <Harness share={liveShare('https://example.com/', 'Example Domain', '42:1')} />
    );
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());

    rerender(
      <Harness
        share={liveShare('https://www.iana.org/help/example-domains', 'Example Domains', '42:2')}
      />
    );

    const panel = panelAccessFor('session-web');
    expect(panel?.describe()).toMatchObject({
      title: 'Example Domains',
      locator: 'https://www.iana.org/help/example-domains',
      sourceRevision: '42:2',
    });
    await expect(panel?.readText(500)).resolves.toMatchObject({
      title: 'Example Domains',
      locator: 'https://www.iana.org/help/example-domains',
      sourceRevision: '42:2',
      text: 'Example Domains',
    });
    expect(window.electron.embeddedBrowser.readText).toHaveBeenCalledWith(
      'embedded-browser-test',
      500
    );
  });

  it('revokes access when the user stops sharing', async () => {
    const { rerender } = render(
      <Harness share={liveShare('https://example.com/', 'Example Domain', '42:1')} />
    );
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    rerender(<Harness share={null} />);

    await expect(panelAccessFor('session-web')?.readText(500)).resolves.toBeNull();
  });

  it('refuses capture without a live share instead of capturing host chrome', async () => {
    render(<Harness share={null} />);
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());

    await expect(panelAccessFor('session-web')?.capture()).resolves.toBeNull();
    expect(window.electron.captureRegion).not.toHaveBeenCalled();
    expect(window.electron.embeddedBrowser.capture).not.toHaveBeenCalled();
  });

  it('deletes an in-flight capture when sharing is revoked', async () => {
    let resolveCapture!: (value: {
      path: string;
      width: number;
      height: number;
      sourceRevision: string;
    }) => void;
    vi.mocked(window.electron.embeddedBrowser.capture).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveCapture = resolve;
      })
    );
    const { rerender } = render(
      <Harness share={liveShare('https://example.com/', 'Example Domain', '42:1')} />
    );
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    const pending = panelAccessFor('session-web')?.capture();
    rerender(<Harness share={null} />);
    resolveCapture({
      path: '/tmp/revoked.png',
      width: 800,
      height: 600,
      sourceRevision: '42:1',
    });

    await expect(pending).resolves.toBeNull();
    expect(window.electron.deleteTempFile).toHaveBeenCalledWith('/tmp/revoked.png');
  });

  it('rejects a capture when the page revision or artifact changes', async () => {
    const { rerender } = render(
      <Harness share={liveShare('https://example.com/', 'Example Domain', '42:1')} />
    );
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    vi.mocked(window.electron.embeddedBrowser.capture).mockResolvedValueOnce({
      path: '/tmp/new-revision.png',
      width: 800,
      height: 600,
      sourceRevision: '42:2',
    });
    await expect(panelAccessFor('session-web')?.capture()).resolves.toBeNull();
    expect(window.electron.deleteTempFile).toHaveBeenCalledWith('/tmp/new-revision.png');

    let resolveCapture!: (value: {
      path: string;
      width: number;
      height: number;
      sourceRevision: string;
    }) => void;
    vi.mocked(window.electron.embeddedBrowser.capture).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveCapture = resolve;
      })
    );
    const pending = panelAccessFor('session-web')?.capture();
    rerender(
      <Harness
        shownArtifact={{ kind: 'externalUrl', title: 'Other', url: 'https://other.test/' }}
        share={liveShare('https://other.test/', 'Other', '77:1')}
      />
    );
    resolveCapture({
      path: '/tmp/old-artifact.png',
      width: 800,
      height: 600,
      sourceRevision: '42:1',
    });
    await expect(pending).resolves.toBeNull();
    expect(window.electron.deleteTempFile).toHaveBeenCalledWith('/tmp/old-artifact.png');
  });

  it('rejects live text that no longer matches the shared URL and revision', async () => {
    vi.mocked(window.electron.embeddedBrowser.readText).mockResolvedValueOnce({
      url: 'https://example.com/changed',
      title: 'Changed',
      sourceRevision: '42:2',
      text: 'new page',
      truncated: false,
    });
    render(<Harness share={liveShare('https://example.com/', 'Example Domain', '42:1')} />);
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());

    await expect(panelAccessFor('session-web')?.readText(500)).resolves.toBeNull();
  });
});

describe('local file panel access', () => {
  const textArtifact: ArtifactSource = {
    kind: 'file',
    title: 'notes.md',
    path: '/tmp/notes.md',
  };
  const pdfArtifact: ArtifactSource = {
    kind: 'file',
    title: 'report.pdf',
    path: '/tmp/report.pdf',
  };

  it('discards a local read when the panel closes while IPC is in flight', async () => {
    let resolveRead!: (value: {
      kind: 'text';
      title: string;
      path: string;
      mimeType: string;
      text: string;
      size: number;
      revision: string;
      found: true;
    }) => void;
    vi.mocked(window.electron.readArtifactFile).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveRead = resolve;
      })
    );
    const { rerender } = render(
      <Harness share={null} shownArtifact={textArtifact} isOpen fileSourceRevision="10:1" />
    );
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    const pending = panelAccessFor('session-web')?.readText(500);

    rerender(<Harness share={null} shownArtifact={textArtifact} isOpen={false} />);
    resolveRead({
      kind: 'text',
      title: 'notes.md',
      path: '/tmp/notes.md',
      mimeType: 'text/markdown',
      text: 'stale text',
      size: 10,
      revision: '10:1',
      found: true,
    });

    await expect(pending).resolves.toBeNull();
  });

  it('discards a local read when another artifact replaces it in flight', async () => {
    let resolveRead!: (value: {
      kind: 'text';
      title: string;
      path: string;
      mimeType: string;
      text: string;
      size: number;
      revision: string;
      found: true;
    }) => void;
    vi.mocked(window.electron.readArtifactFile).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveRead = resolve;
      })
    );
    const { rerender } = render(
      <Harness share={null} shownArtifact={textArtifact} fileSourceRevision="10:1" />
    );
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    const pending = panelAccessFor('session-web')?.readText(500);

    rerender(
      <Harness
        share={null}
        shownArtifact={{ kind: 'file', title: 'other.md', path: '/tmp/other.md' }}
      />
    );
    resolveRead({
      kind: 'text',
      title: 'notes.md',
      path: '/tmp/notes.md',
      mimeType: 'text/markdown',
      text: 'stale text',
      size: 10,
      revision: '10:1',
      found: true,
    });

    await expect(pending).resolves.toBeNull();
  });

  it('confirms a PDF revision after extraction and rejects changed bytes', async () => {
    const pdfPreview = (revision: string) => ({
      kind: 'document' as const,
      format: 'pdf' as const,
      title: 'report.pdf',
      path: '/tmp/report.pdf',
      mimeType: 'application/pdf',
      data: new Uint8Array([1, 2, 3]).buffer,
      size: 3,
      revision,
      found: true as const,
    });
    vi.mocked(window.electron.readArtifactFile)
      .mockResolvedValueOnce(pdfPreview('3:1'))
      .mockResolvedValueOnce(pdfPreview('3:2'));
    render(<Harness share={null} shownArtifact={pdfArtifact} fileSourceRevision="3:1" />);
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());

    await expect(panelAccessFor('session-web')?.readText(500)).resolves.toBeNull();
    expect(window.electron.readArtifactFile).toHaveBeenCalledTimes(2);
    expect(pdfMocks.getTextContent).toHaveBeenCalled();
  });

  it('discards extracted PDF text when the panel closes during extraction', async () => {
    let resolveText!: (value: { items: Array<{ str: string }> }) => void;
    pdfMocks.getTextContent.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveText = resolve;
      })
    );
    vi.mocked(window.electron.readArtifactFile).mockResolvedValueOnce({
      kind: 'document',
      format: 'pdf',
      title: 'report.pdf',
      path: '/tmp/report.pdf',
      mimeType: 'application/pdf',
      data: new Uint8Array([1, 2, 3]).buffer,
      size: 3,
      revision: '3:1',
      found: true,
    });
    const { rerender } = render(
      <Harness share={null} shownArtifact={pdfArtifact} fileSourceRevision="3:1" />
    );
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    const pending = panelAccessFor('session-web')?.readText(500);
    await waitFor(() => expect(pdfMocks.getTextContent).toHaveBeenCalled());

    rerender(<Harness share={null} shownArtifact={pdfArtifact} isOpen={false} />);
    resolveText({ items: [{ str: 'stale PDF text' }] });

    await expect(pending).resolves.toBeNull();
    expect(window.electron.readArtifactFile).toHaveBeenCalledTimes(1);
  });

  it('rejects file text when the bytes no longer match the rendered revision', async () => {
    vi.mocked(window.electron.readArtifactFile).mockResolvedValueOnce({
      kind: 'text',
      title: 'notes.md',
      path: '/tmp/notes.md',
      mimeType: 'text/markdown',
      text: 'new bytes not shown in the preview',
      size: 34,
      revision: '34:2',
      found: true,
    });
    render(<Harness share={null} shownArtifact={textArtifact} fileSourceRevision="10:1" />);
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());

    await expect(panelAccessFor('session-web')?.readText(500)).resolves.toBeNull();
    expect(panelAccessFor('session-web')?.describe().sourceRevision).toBe('10:1');
  });
});

/**
 * A `biorouter serve` browser has no Electron main process, so the bridge
 * `renderer.tsx` installs there carries no `captureRegion`. The capture used to
 * optional-chain only the bridge, so the agent's panel capture threw "is not a
 * function" there, and the workspace channel handed the model that TypeError as
 * the tool's result (measured on serve, 2026-09-14). A missing method is a
 * capture that did not happen: `null`, like every other one.
 */
describe('panel capture on a bridge without captureRegion', () => {
  const htmlArtifact: ArtifactSource = {
    kind: 'html',
    title: 'hello.html',
    html: '<h1>Hello panel</h1>',
  };

  /** The panel element's box. jsdom measures nothing, and a zero box never reaches the call. */
  function measurePanel() {
    const panel = document.querySelector<HTMLElement>(`[${ARTIFACT_PANEL_ATTR}]`);
    expect(panel).not.toBeNull();
    vi.spyOn(panel as HTMLElement, 'getBoundingClientRect').mockReturnValue({
      x: 900,
      y: 44,
      left: 900,
      top: 44,
      right: 1260,
      bottom: 620,
      width: 360,
      height: 576,
      toJSON: () => ({}),
    });
  }

  function installBridge(bridge: Record<string, unknown>) {
    Object.defineProperty(window, 'electron', { configurable: true, value: bridge });
  }

  // The positive control first, through the SAME harness and the same box: a
  // bridge that has the method is asked for the panel and its answer comes back.
  // Without it, a null below could be the zero-box early return, not the fix.
  it('reaches the capture call when the bridge has the method', async () => {
    const shot = { path: '/tmp/panel.png', width: 360, height: 576 };
    const captureRegion = vi.fn(async () => shot);
    installBridge({ captureRegion, deleteTempFile: vi.fn() });
    render(<Harness share={null} shownArtifact={htmlArtifact} />);
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    measurePanel();

    await expect(panelAccessFor('session-web')?.capture()).resolves.toEqual(shot);
    expect(captureRegion).toHaveBeenCalledWith(
      expect.objectContaining({ x: 900, y: 44, width: 360, height: 576, label: 'panel' })
    );
  });

  it.each([
    ['the browser bridge (no captureRegion)', () => ({ deleteTempFile: () => {} })],
    ['a bridge with no methods at all', () => ({})],
  ])('answers null instead of throwing on %s', async (_name, bridge) => {
    installBridge(bridge());
    render(<Harness share={null} shownArtifact={htmlArtifact} />);
    await waitFor(() => expect(panelAccessFor('session-web')).not.toBeNull());
    measurePanel();

    await expect(panelAccessFor('session-web')?.capture()).resolves.toBeNull();
  });
});
