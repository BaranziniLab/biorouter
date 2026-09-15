import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useArtifactPanel } from './useArtifactPanel';
import type { ArtifactSource } from './artifactTypes';

const first: ArtifactSource = { kind: 'file', path: '/tmp/first.txt', title: 'First' };
const second: ArtifactSource = { kind: 'file', path: '/tmp/second.txt', title: 'Second' };

function setup() {
  const pending: Array<() => void> = [];
  vi.stubGlobal('innerWidth', 800);
  vi.stubGlobal('electron', {
    ensureWindowContentWidth: vi.fn(() => new Promise<void>((resolve) => pending.push(resolve))),
  });
  const hook = renderHook(() => useArtifactPanel({ isMobile: false, allowWindowResize: true }));
  return { ...hook, pending };
}

afterEach(() => vi.unstubAllGlobals());

describe('pending artifact opening', () => {
  it.each(['reset', 'closePanel'] as const)(
    'cannot resurrect a preview after %s',
    async (cancel) => {
      const { result, pending } = setup();
      let opening: Promise<void>;
      act(() => {
        opening = result.current.openArtifact(first);
      });
      expect(pending).toHaveLength(1);
      act(() => result.current[cancel]());
      await act(async () => {
        pending[0]();
        await opening;
      });
      expect(result.current.artifact).toBeNull();
      expect(result.current.viewerProps.isOpen).toBe(false);
    }
  );

  it('keeps the newest artifact when window-resize promises resolve out of order', async () => {
    const { result, pending } = setup();
    let older: Promise<void>;
    let newer: Promise<void>;
    act(() => {
      older = result.current.openArtifact(first);
    });
    act(() => {
      newer = result.current.openArtifact(second);
    });
    expect(pending).toHaveLength(2);
    await act(async () => {
      pending[1]();
      await newer;
    });
    expect(result.current.artifact).toBe(second);
    await act(async () => {
      pending[0]();
      await older;
    });
    expect(result.current.artifact).toBe(second);
  });

  it('keeps a new session preview when an earlier session resize finally completes', async () => {
    const { result, pending } = setup();
    let older: Promise<void>;
    let newer: Promise<void>;
    act(() => {
      older = result.current.openArtifact(first);
    });
    act(() => result.current.reset());
    act(() => {
      newer = result.current.openArtifact(second);
    });
    await act(async () => {
      pending[1]();
      await newer;
    });
    await act(async () => {
      pending[0]();
      await older;
    });
    expect(result.current.artifact).toBe(second);
  });
});
