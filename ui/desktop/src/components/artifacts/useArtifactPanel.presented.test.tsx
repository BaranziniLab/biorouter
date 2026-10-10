import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PREVIEW_MIN_WIDTH } from '../Layout/yieldLadder';
import {
  ARTIFACT_PANEL_EXIT_MS,
  useArtifactPanel,
  type ArtifactPanelPresentedChange,
} from './useArtifactPanel';
import type { ArtifactSource } from './artifactTypes';

const first: ArtifactSource = { kind: 'file', path: '/tmp/first.txt', title: 'First' };
const second: ArtifactSource = { kind: 'file', path: '/tmp/second.txt', title: 'Second' };

// The contract the summary rail and the conversation glide read (WS-SUMMARY):
// `sideWidth` is the room a side panel takes, and `onPresentedChange` fires
// before an ACTION changes the grid, never for a reset or a switch.
describe('useArtifactPanel presentation contract', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal('electron', {});
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function setup(options: { enabled?: boolean } = {}) {
    const calls: Array<{ change: ArtifactPanelPresentedChange; artifactAtCall: unknown }> = [];
    let current: ReturnType<typeof useArtifactPanel> | null = null;
    const hook = renderHook(() => {
      const panel = useArtifactPanel({
        isMobile: false,
        enabled: options.enabled,
        onPresentedChange: (change) => calls.push({ change, artifactAtCall: current?.artifact }),
      });
      current = panel;
      return panel;
    });
    return { ...hook, calls };
  }

  it('reports no side width while nothing is presented', () => {
    const { result } = setup();
    expect(result.current.sideWidth).toBe(0);
  });

  it('reports the resolved side width while a side panel is presented', async () => {
    const { result } = setup();
    await act(async () => {
      await result.current.openArtifact(first);
    });
    expect(result.current.previewMode).toBe('side');
    expect(result.current.sideWidth).toBe(PREVIEW_MIN_WIDTH);
  });

  it('reports no side width when the host does not render the panel', async () => {
    const { result } = setup({ enabled: false });
    await act(async () => {
      await result.current.openArtifact(first);
    });
    expect(result.current.sideWidth).toBe(0);
  });

  it('announces an open before the grid changes, and a close after the exit', async () => {
    const { result, calls } = setup();
    await act(async () => {
      await result.current.openArtifact(first);
    });
    expect(calls).toHaveLength(1);
    expect(calls[0].change).toEqual({
      presented: true,
      layout: 'side',
      sideWidth: PREVIEW_MIN_WIDTH,
      windowGrowing: false,
    });
    // Called before the state update, so the previous layout was still rendered.
    expect(calls[0].artifactAtCall).toBeNull();

    act(() => result.current.closePanel());
    expect(calls).toHaveLength(1);
    act(() => {
      vi.advanceTimersByTime(ARTIFACT_PANEL_EXIT_MS);
    });
    expect(calls).toHaveLength(2);
    expect(calls[1].change).toEqual({
      presented: false,
      layout: 'side',
      sideWidth: PREVIEW_MIN_WIDTH,
      windowGrowing: false,
    });
    expect(calls[1].artifactAtCall).toBe(first);
    expect(result.current.artifact).toBeNull();
    expect(result.current.sideWidth).toBe(0);
  });

  it('does not report a switch inside a presented panel, or a reset', async () => {
    const { result, calls } = setup();
    await act(async () => {
      await result.current.openArtifact(first);
    });
    await act(async () => {
      await result.current.openArtifact(second);
    });
    expect(result.current.artifact).toBe(second);
    expect(calls).toHaveLength(1);
    act(() => result.current.reset());
    act(() => {
      vi.advanceTimersByTime(ARTIFACT_PANEL_EXIT_MS * 2);
    });
    expect(calls).toHaveLength(1);
  });

  it('does not report a close that an open overtook', async () => {
    const { result, calls } = setup();
    await act(async () => {
      await result.current.openArtifact(first);
    });
    act(() => result.current.closePanel());
    await act(async () => {
      await result.current.openArtifact(second);
    });
    act(() => {
      vi.advanceTimersByTime(ARTIFACT_PANEL_EXIT_MS * 2);
    });
    expect(result.current.artifact).toBe(second);
    expect(calls.map((call) => call.change.presented)).toEqual([true]);
  });

  it('says when the open asked the window to grow', async () => {
    vi.stubGlobal('innerWidth', 600);
    vi.stubGlobal('electron', { ensureWindowContentWidth: vi.fn(() => Promise.resolve()) });
    const calls: ArtifactPanelPresentedChange[] = [];
    const { result } = renderHook(() =>
      useArtifactPanel({
        isMobile: false,
        allowWindowResize: true,
        onPresentedChange: (change) => calls.push(change),
      })
    );
    await act(async () => {
      await result.current.openArtifact(first);
    });
    expect(calls).toHaveLength(1);
    expect(calls[0].windowGrowing).toBe(true);
  });
});
