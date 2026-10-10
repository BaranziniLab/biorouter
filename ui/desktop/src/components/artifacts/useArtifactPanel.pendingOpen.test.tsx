import { act, render, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ARTIFACT_PANEL_EXIT_MS, useArtifactPanel } from './useArtifactPanel';
import type { ArtifactSource } from './artifactTypes';

const first: ArtifactSource = { kind: 'file', path: '/tmp/first.txt', title: 'First' };
const second: ArtifactSource = { kind: 'file', path: '/tmp/second.txt', title: 'Second' };

/**
 * THE WINDOW GROWS WHILE THE PANEL OPENS (spec 3.8, "Window grow").
 *
 * The open used to `await` the OS resize before presenting the panel. On macOS
 * that resize is synchronous in the main process, so the IPC resolved only once
 * the animation had finished: measured in Electron, the panel appeared 367ms
 * after the click, in a window that had grown with nothing in it. It now
 * presents in the same frame and asks for the grow alongside, so these tests pin
 * what must still hold while a grow is in flight.
 */
function setup() {
  const pending: Array<() => void> = [];
  vi.stubGlobal('innerWidth', 800);
  vi.stubGlobal('electron', {
    ensureWindowContentWidth: vi.fn(() => new Promise<void>((resolve) => pending.push(resolve))),
  });
  const hook = renderHook(() => useArtifactPanel({ isMobile: false, allowWindowResize: true }));
  return { ...hook, pending };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('opening while the window grows', () => {
  it('presents the panel before the grow settles, and asks for it once', () => {
    const { result, pending } = setup();
    act(() => {
      void result.current.openArtifact(first);
    });
    expect(pending).toHaveLength(1);
    expect(result.current.artifact).toBe(first);
  });

  it.each(['reset', 'closePanel'] as const)(
    'cannot resurrect a preview after %s when the grow settles late',
    async (cancel) => {
      vi.useFakeTimers();
      const { result, pending } = setup();
      act(() => {
        void result.current.openArtifact(first);
      });
      act(() => result.current[cancel]());
      act(() => {
        vi.advanceTimersByTime(ARTIFACT_PANEL_EXIT_MS * 2);
      });
      await act(async () => {
        pending[0]();
        await Promise.resolve();
      });
      expect(result.current.artifact).toBeNull();
      expect(result.current.viewerProps.isOpen).toBe(false);
    }
  );

  it('keeps the newest artifact when an earlier grow settles after it', async () => {
    const { result, pending } = setup();
    act(() => {
      void result.current.openArtifact(first);
    });
    act(() => {
      void result.current.openArtifact(second);
    });
    // The second open swaps the artifact in the panel that is already up; only
    // the first asked the window to grow.
    expect(pending).toHaveLength(1);
    expect(result.current.artifact).toBe(second);
    await act(async () => {
      pending[0]();
      await Promise.resolve();
    });
    expect(result.current.artifact).toBe(second);
  });

  it('keeps a new session preview when an earlier session grow settles', async () => {
    const { result, pending } = setup();
    act(() => {
      void result.current.openArtifact(first);
    });
    act(() => result.current.reset());
    act(() => {
      void result.current.openArtifact(second);
    });
    await act(async () => {
      for (const resolve of pending) resolve();
      await Promise.resolve();
    });
    expect(result.current.artifact).toBe(second);
  });

  // While the window grows, rung 2 decides on the width the box is growing TO,
  // so a panel opened from a narrow window is placed beside the chat at once
  // rather than stacked and then moved to the side a few frames later. Once the
  // grow settles the box is measured for real: if the window could not grow
  // (a full screen, a narrow display), the honest answer replaces the guess.
  it('places the panel for the width the window is growing to, then measures for real', async () => {
    const pending: Array<() => void> = [];
    vi.stubGlobal('innerWidth', 700);
    vi.stubGlobal('electron', {
      ensureWindowContentWidth: vi.fn(() => new Promise<void>((resolve) => pending.push(resolve))),
    });
    let panel: ReturnType<typeof useArtifactPanel> | null = null;
    function Host() {
      panel = useArtifactPanel({ isMobile: false, allowWindowResize: true });
      return <div ref={panel.splitPaneRef} data-testid="split" />;
    }
    const { getByTestId } = render(<Host />);
    const split = getByTestId('split');
    Object.defineProperty(split, 'clientWidth', { configurable: true, value: 700 });
    Object.defineProperty(split, 'clientHeight', { configurable: true, value: 700 });

    act(() => {
      void panel!.openArtifact(first);
    });
    expect(pending).toHaveLength(1);
    expect(panel!.previewMode).toBe('side');

    // The window did not grow after all: the box is still 700px wide.
    await act(async () => {
      pending[0]();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(panel!.previewMode).toBe('stack');
  });
});
