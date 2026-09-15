import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import ArtifactViewer from './ArtifactViewer';
import { useArtifactPanel } from './useArtifactPanel';

/**
 * Rung 2's fold and drag wiring, end to end through the real hook and the real
 * panel.
 *
 * jsdom has no layout engine, so the split box's geometry is stubbed to the
 * numbers the running app measured for a 576px pane at 1440×900 (the operator's
 * repro): a 900px split box, a 44px header, a 174px composer bar. What is under
 * test is not the geometry — `yieldLadder.test.ts` owns that — but the STATE a
 * user drives: folding by drag, folding by the toggle, unfolding by a tab or the
 * bare strip, and which height a sheet comes back to. A prototype found the
 * last of these wrong only by driving it live (a drag that folded the sheet
 * forgot the height it started from), which is why it has a test of its own.
 */

const SPLIT_W = 576;
const SPLIT_H = 900;
const HEADER_H = 44;
const COMPOSER_H = 174;

function rect(top: number, height: number, width = SPLIT_W): DOMRect {
  return {
    x: 0,
    y: top,
    top,
    left: 0,
    width,
    height,
    right: width,
    bottom: top + height,
    toJSON: () => ({}),
  } as DOMRect;
}

function Host() {
  const panel = useArtifactPanel({ isMobile: false });
  return (
    <div ref={panel.splitPaneRef} {...panel.splitPaneProps} data-testid="split">
      <div data-preview-area="column">
        <div data-preview-area="body">
          <div data-preview-area="header" data-testid="header" />
          <div data-preview-area="transcript" data-preview-transcript="" data-testid="transcript" />
        </div>
        <div data-preview-area="composer" />
      </div>
      <button
        type="button"
        onClick={() =>
          void panel.openArtifact({ kind: 'file', title: 'notes.txt', path: '/tmp/notes.txt' })
        }
      >
        open
      </button>
      {panel.artifact && <ArtifactViewer {...panel.viewerProps} />}
    </div>
  );
}

function installGeometry() {
  const clientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth');
  const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight');
  const bounding = HTMLElement.prototype.getBoundingClientRect;
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
    configurable: true,
    get(this: HTMLElement) {
      return this.dataset.testid === 'split' ? SPLIT_W : 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get(this: HTMLElement) {
      return this.dataset.testid === 'split' ? SPLIT_H : 0;
    },
  });
  HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
    if (this.dataset.testid === 'split') return rect(0, SPLIT_H);
    if (this.dataset.testid === 'header') return rect(0, HEADER_H);
    // The transcript as the flex layout lays it out before any sheet: below the
    // header, above the composer — so the measured chrome is the composer bar.
    if (this.dataset.testid === 'transcript') {
      return rect(HEADER_H, SPLIT_H - HEADER_H - COMPOSER_H);
    }
    return rect(0, 0, 0);
  };
  return () => {
    if (clientWidth) Object.defineProperty(HTMLElement.prototype, 'clientWidth', clientWidth);
    if (clientHeight) Object.defineProperty(HTMLElement.prototype, 'clientHeight', clientHeight);
    HTMLElement.prototype.getBoundingClientRect = bounding;
  };
}

/**
 * `binary` by default: a preview that is ready and cannot say how tall it is, so
 * the sheet takes the ladder's half — jsdom would otherwise "measure" a text
 * file at a few pixels of padding. The crossing test below reads real text.
 */
function installElectronMock(kind: 'binary' | 'text' = 'binary') {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      prepareArtifactHtml: vi.fn(async ({ html }: { html: string }) => ({ html })),
      readArtifactFile: vi.fn(async (path: string) => ({
        kind,
        title: path.split('/').pop(),
        path,
        mimeType: kind === 'text' ? 'text/plain' : 'application/octet-stream',
        text: 'one\ntwo\nthree',
        size: 13,
        found: true,
      })),
      ensureWindowContentWidth: vi.fn(async () => undefined),
      openDirectoryInExplorer: vi.fn(async () => undefined),
      broadcastThemeChange: vi.fn(),
      on: vi.fn().mockReturnValue(() => undefined),
    },
  });
}

const split = () => screen.getByTestId('split');
const sheetHeight = () => split().style.getPropertyValue('--preview-stack-height');

async function openPanel() {
  render(
    <ThemeProvider>
      <Host />
    </ThemeProvider>
  );
  fireEvent.click(screen.getByRole('button', { name: 'open' }));
  await screen.findByTestId('artifact-viewer');
  // The content reports (jsdom lays nothing out, so it reports "cannot say"),
  // which ends the measuring phase: the sheet takes its default half.
  await waitFor(() => expect(split().hasAttribute('data-preview-measuring')).toBe(false));
}

function drag(deltaY: number) {
  const handle = screen.getByRole('separator', { name: 'Resize artifact panel' });
  Object.defineProperty(handle, 'setPointerCapture', { configurable: true, value: vi.fn() });
  Object.defineProperty(handle, 'hasPointerCapture', { configurable: true, value: () => false });
  fireEvent.pointerDown(handle, { pointerId: 7, clientX: 10, clientY: 500 });
  // jsdom has no PointerEvent constructor; the hook reads only these three fields.
  const pointer = (type: string) =>
    Object.assign(new Event(type), { pointerId: 7, clientX: 10, clientY: 500 + deltaY });
  act(() => {
    window.dispatchEvent(pointer('pointermove'));
  });
  act(() => {
    window.dispatchEvent(pointer('pointerup'));
  });
}

describe('rung 2 — folding and dragging a stacked sheet', () => {
  let restoreGeometry: () => void;

  beforeEach(() => {
    installElectronMock();
    restoreGeometry = installGeometry();
  });

  afterEach(() => {
    restoreGeometry();
  });

  it('stacks in a 576px pane at half the body, leaving the conversation its floor', async () => {
    await openPanel();
    expect(split().getAttribute('data-preview-layout')).toBe('stack');
    // H = 900 − 44 = 856; half = 428; F = 174 + 8 + 146 = 328.
    expect(sheetHeight()).toBe('428px');
    expect(split().style.getPropertyValue('--preview-chat-floor')).toBe('328px');
    expect(screen.getByRole('separator', { name: 'Resize artifact panel' })).toHaveAttribute(
      'aria-orientation',
      'horizontal'
    );
  });

  it('holds a fresh sheet back, taking no room, until its content has answered', async () => {
    render(
      <ThemeProvider>
        <Host />
      </ThemeProvider>
    );
    fireEvent.click(screen.getByRole('button', { name: 'open' }));
    await screen.findByTestId('artifact-viewer');
    // The first committed frame of a fresh sheet: measuring, zero rows of room.
    expect(split().hasAttribute('data-preview-measuring')).toBe(true);
    expect(sheetHeight()).toBe('0px');
    expect(split().style.getPropertyValue('--preview-provisional-height')).toBe('428px');
    await waitFor(() => expect(sheetHeight()).toBe('428px'));
  });

  it('folds by the toggle, and unfolds back to the same height', async () => {
    await openPanel();
    const toggle = screen.getByTestId('artifact-fold-toggle');
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(toggle);
    expect(split().hasAttribute('data-preview-folded')).toBe(true);
    expect(sheetHeight()).toBe('36px');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(toggle);
    expect(split().hasAttribute('data-preview-folded')).toBe(false);
    expect(sheetHeight()).toBe('428px');
  });

  it('folds when the edge is dragged above the midpoint of [36, 200], and not before', async () => {
    await openPanel();
    // 428 − 310 = 118: exactly the threshold — clamps to the 200 floor, stays open.
    drag(-310);
    expect(split().hasAttribute('data-preview-folded')).toBe(false);
    expect(sheetHeight()).toBe('200px');
    // From 200, −83 wants 117: past the threshold, so it folds.
    drag(-83);
    expect(split().hasAttribute('data-preview-folded')).toBe(true);
    expect(sheetHeight()).toBe('36px');
  });

  it('THE REGRESSION: a drag that folds keeps the height the sheet had when the drag began', async () => {
    await openPanel();
    drag(-128); // 428 → 300, a real choice the user made
    expect(sheetHeight()).toBe('300px');
    drag(-250); // 300 → 50: folds
    expect(sheetHeight()).toBe('36px');
    fireEvent.click(screen.getByTestId('artifact-fold-toggle'));
    // Back to 300 — not to the sliver the pointer passed through on its way up.
    expect(sheetHeight()).toBe('300px');
  });

  it('clamps a drag to the conversation’s floor', async () => {
    await openPanel();
    drag(+2000);
    expect(sheetHeight()).toBe(`${856 - 328}px`);
  });

  it('unfolds on a tab click and on the bare strip, but not on another control', async () => {
    await openPanel();
    fireEvent.click(screen.getByTestId('artifact-fold-toggle'));
    expect(split().hasAttribute('data-preview-folded')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Open active artifact outside preview' }));
    expect(split().hasAttribute('data-preview-folded')).toBe(true);

    fireEvent.click(screen.getByRole('tablist', { name: 'Open artifact previews' }));
    expect(split().hasAttribute('data-preview-folded')).toBe(false);

    fireEvent.click(screen.getByTestId('artifact-fold-toggle'));
    fireEvent.click(screen.getByRole('tab', { name: /notes\.txt/ }));
    expect(split().hasAttribute('data-preview-folded')).toBe(false);
  });

  it('closing the panel restores the ordinary layout and forgets the fold', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      await openPanel();
      fireEvent.click(screen.getByTestId('artifact-fold-toggle'));
      fireEvent.click(screen.getByRole('button', { name: 'Close preview panel' }));
      await act(async () => {
        vi.advanceTimersByTime(200);
      });
      expect(screen.queryByTestId('artifact-viewer')).toBeNull();
      expect(split().hasAttribute('data-preview-layout')).toBe(false);
      expect(split().hasAttribute('data-preview-folded')).toBe(false);
      expect(sheetHeight()).toBe('');
      expect(split().style.getPropertyValue('--preview-panel-width')).toBe('');
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('rung 2 — a crossing re-renders attributes, never the panel', () => {
  it('keeps the aside, the separator and the preview body as the same nodes', async () => {
    installElectronMock('text');
    const artifact = { kind: 'file' as const, title: 'notes.txt', path: '/tmp/notes.txt' };
    const props = {
      artifact,
      onClose: vi.fn(),
      onOpenArtifact: vi.fn(),
      onResizeStart: vi.fn(),
      onToggleFold: vi.fn(),
      onUnfold: vi.fn(),
      onContentHeightChange: vi.fn(),
    };
    const { rerender, container } = render(
      <ThemeProvider>
        <ArtifactViewer {...props} layout="side" />
      </ThemeProvider>
    );
    const aside = await screen.findByTestId('artifact-viewer');
    const separator = screen.getByRole('separator', { name: 'Resize artifact panel' });
    const body = screen.getByTestId('artifact-preview-content');
    const toggle = screen.getByTestId('artifact-fold-toggle');
    await screen.findByTestId('artifact-status-strip');
    const removed: Node[] = [];
    const observer = new MutationObserver((records) => {
      for (const record of records) removed.push(...record.removedNodes);
    });
    observer.observe(container, { childList: true, subtree: true });

    rerender(
      <ThemeProvider>
        <ArtifactViewer {...props} layout="stack" />
      </ThemeProvider>
    );
    rerender(
      <ThemeProvider>
        <ArtifactViewer {...props} layout="side" />
      </ThemeProvider>
    );
    // Folding and unfolding turn one glyph; they mount nothing either.
    rerender(
      <ThemeProvider>
        <ArtifactViewer {...props} layout="stack" folded />
      </ThemeProvider>
    );
    rerender(
      <ThemeProvider>
        <ArtifactViewer {...props} layout="stack" folded={false} />
      </ThemeProvider>
    );
    await Promise.resolve();
    observer.disconnect();

    expect(screen.getByTestId('artifact-viewer')).toBe(aside);
    expect(screen.getByRole('separator', { name: 'Resize artifact panel' })).toBe(separator);
    expect(screen.getByTestId('artifact-preview-content')).toBe(body);
    expect(screen.getByTestId('artifact-fold-toggle')).toBe(toggle);
    expect(removed).toHaveLength(0);
  });
});
