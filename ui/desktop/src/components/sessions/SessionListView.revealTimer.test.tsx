import { act, render } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

function renderPane() {
  return render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
}

/** The content layer, whose opacity class is driven by `showContent`. */
function contentLayer(container: HTMLElement): HTMLElement {
  const el = container.querySelector<HTMLElement>('div.relative.transition-opacity');
  if (!el) throw new Error('content layer not found');
  return el;
}

describe('SessionListView reveal timer', () => {
  it('clears the pending reveal timer when the pane unmounts before the tick', async () => {
    const { unmount } = renderPane();

    // Let the initial request settle so the skeleton-to-content effect runs and
    // arms its 10ms reveal.
    await act(async () => {});
    expect(vi.getTimerCount()).toBeGreaterThan(0);

    // Navigating away (or a test file finishing) inside that 10ms window used to
    // leave the reveal armed; it then fired `setShowContent` on an unmounted
    // tree — on CI, after jsdom had been torn down, as `window is not defined`.
    // Every timer this pane owns must be cancelled by its own cleanup.
    unmount();
    expect(vi.getTimerCount()).toBe(0);

    expect(() => {
      act(() => {
        vi.advanceTimersByTime(100);
      });
    }).not.toThrow();
  });

  it('still reveals the content layer one tick after the skeleton hides', async () => {
    const { container } = renderPane();

    await act(async () => {});
    expect(contentLayer(container).className).toContain('opacity-0');

    act(() => {
      vi.advanceTimersByTime(10);
    });
    expect(contentLayer(container).className).toContain('opacity-100');
  });
});
