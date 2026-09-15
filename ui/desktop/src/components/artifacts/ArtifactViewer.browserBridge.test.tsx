import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import {
  onArtifactAnnotation,
  resetAnnotationChannelForTests,
} from '../../utils/annotationChannel';
import ArtifactViewer from './ArtifactViewer';
import { ANNOTATE_NEEDS_DESKTOP_REASON } from './captureOnBrowser';

/**
 * "Send a region to the chat" against a bridge with no `captureRegion`.
 *
 * A `biorouter serve` browser has no Electron. `renderer.tsx` installs a bridge
 * of browser-safe stand-ins in its place, and a compositor grab has no stand-in,
 * so that bridge carries no `captureRegion`. The viewer called
 * `window.electron?.captureRegion(…)`, which guards the bridge and not the
 * method: in a browser the drag ended in "captureRegion is not a function", the
 * rejection skipped `finishAnnotation()`, and the selection overlay stayed up
 * over the preview with the camera still pressed (measured on serve,
 * 2026-09-14). Only Escape took it down.
 *
 * Two separate facts are pinned here. The call itself must survive a missing
 * method on ANY surface — that is what ends the stuck overlay. And on a browser
 * the control must not be offered at all (SD-8), because no drag there can ever
 * produce a picture.
 */

const originalElectron = window.electron;

const image = {
  kind: 'image',
  title: 'figure.png',
  path: '/tmp/figure.png',
  mimeType: 'image/png',
  size: 68,
  revision: '68:1',
  found: true,
  dataUrl: 'data:image/png;base64,iVBORw0KGgo=',
};

/** The bridge as a browser has it: enough to render a preview, no capture. */
function browserShapedBridge() {
  return {
    readArtifactFile: vi.fn(async () => image),
    prepareArtifactHtml: vi.fn(async ({ html }: { html: string }) => ({ html })),
    getTempImage: vi.fn(async (path: string) => path),
    deleteTempFile: vi.fn(),
    on: () => () => {},
    platform: 'linux',
  };
}

function installBridge(bridge: Record<string, unknown>) {
  Object.defineProperty(window, 'electron', { configurable: true, writable: true, value: bridge });
}

const mount = () =>
  render(
    <ThemeProvider>
      <ArtifactViewer
        artifact={{ kind: 'file', title: 'figure.png', path: '/tmp/figure.png' }}
        onClose={vi.fn()}
        onOpenArtifact={vi.fn()}
        sessionId="session-1"
      />
    </ThemeProvider>
  );

async function dragARegion() {
  const overlay = screen.getByTestId('annotation-overlay');
  Object.defineProperty(overlay, 'setPointerCapture', { value: vi.fn() });
  fireEvent.pointerDown(overlay, { button: 0, clientX: 20, clientY: 30, pointerId: 1 });
  fireEvent.pointerMove(overlay, { clientX: 220, clientY: 150, pointerId: 1 });
  fireEvent.pointerUp(overlay, { pointerId: 1 });
}

beforeEach(() => {
  resetAnnotationChannelForTests();
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
  resetAnnotationChannelForTests();
  installBridge(originalElectron as unknown as Record<string, unknown>);
});

describe('a selection against a bridge without captureRegion', () => {
  it.each([
    ['the browser-shaped bridge', () => browserShapedBridge()],
    [
      'a bridge whose captureRegion is not a function',
      () => ({ ...browserShapedBridge(), captureRegion: undefined }),
    ],
  ])('ends the selection on %s instead of leaving the overlay up', async (_name, bridge) => {
    // The call has to survive a missing method on its own, whatever else guards
    // the control — so this runs on the DESKTOP surface, where the camera is
    // offered and the drag really reaches the capture.
    installBridge(bridge());
    const rejections: unknown[] = [];
    const onRejection = (reason: unknown) => rejections.push(reason);
    process.on('unhandledRejection', onRejection);
    const received = vi.fn();
    const stopListening = onArtifactAnnotation('session-1', received);

    try {
      mount();
      expect(await screen.findByRole('img', { name: 'figure.png' })).toBeInTheDocument();
      await userEvent.click(screen.getByTestId('artifact-annotate'));
      expect(screen.getByTestId('annotation-overlay')).toBeInTheDocument();

      await dragARegion();

      await waitFor(() =>
        expect(screen.queryByTestId('annotation-overlay')).not.toBeInTheDocument()
      );
      expect(screen.getByTestId('artifact-annotate')).toHaveAttribute('aria-pressed', 'false');
      // Nothing was captured, so nothing may be sent as though it had been.
      expect(received).not.toHaveBeenCalled();
      // Let a rejection that escaped the handler reach the process before asking.
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(rejections.map(String)).toEqual([]);
    } finally {
      stopListening();
      process.off('unhandledRejection', onRejection);
    }
  });
});

describe('the camera control on a browser surface (SD-8)', () => {
  it('is not offered in a browser, and says why before it is touched', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    installBridge(browserShapedBridge());
    mount();
    expect(await screen.findByRole('img', { name: 'figure.png' })).toBeInTheDocument();

    const camera = screen.getByTestId('artifact-annotate');
    expect(camera).toBeDisabled();
    expect(camera).toHaveAttribute('title', ANNOTATE_NEEDS_DESKTOP_REASON);
    // The reason is the tooltip, not the control's name: a screen reader still
    // announces what the control is.
    expect(camera).toHaveAccessibleName('Send a region to the chat');

    await userEvent.click(camera);
    expect(screen.queryByTestId('annotation-overlay')).not.toBeInTheDocument();
  });

  it('is offered on the desktop, where the bridge can capture', async () => {
    installBridge({
      ...browserShapedBridge(),
      captureRegion: vi.fn(async () => ({ path: '/tmp/region.png', width: 200, height: 120 })),
    });
    mount();
    expect(await screen.findByRole('img', { name: 'figure.png' })).toBeInTheDocument();

    const camera = screen.getByTestId('artifact-annotate');
    expect(camera).toBeEnabled();
    expect(camera).toHaveAttribute('title', 'Send a region to the chat');
    await userEvent.click(camera);
    expect(screen.getByTestId('annotation-overlay')).toBeInTheDocument();
  });
});
