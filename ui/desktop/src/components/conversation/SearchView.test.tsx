import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SearchView } from './SearchView';

const highlighterMocks = vi.hoisted(() => ({
  highlight: vi.fn(() => []),
  clearHighlights: vi.fn(),
  destroy: vi.fn(),
  setCurrentMatch: vi.fn(),
  releaseClearance: vi.fn(),
  /** The clearance each highlighter was built with (the real one takes a bar finder). */
  clearances: [] as { bar: () => Element | null }[],
}));

vi.mock('./SearchBar', () => ({
  default: ({
    onSearch,
    onClose,
    minSearchLength,
  }: {
    onSearch: (term: string, caseSensitive: boolean) => void;
    onClose: () => void;
    minSearchLength?: number;
  }) => (
    <div data-search-ui="">
      <button onClick={() => onSearch('alpha', false)}>Search alpha</button>
      <button onClick={() => onSearch('beta', false)}>Search beta</button>
      <button onClick={onClose}>Close find</button>
      <span data-testid="min-search-length">{String(minSearchLength)}</span>
    </div>
  ),
}));

vi.mock('../../utils/searchHighlighter', () => ({
  SEARCH_UI_ATTRIBUTE: 'data-search-ui',
  FindClearance: class {
    bar: () => Element | null;
    release = highlighterMocks.releaseClearance;
    constructor(bar: () => Element | null) {
      this.bar = bar;
    }
  },
  SearchHighlighter: class {
    highlight = highlighterMocks.highlight;
    clearHighlights = highlighterMocks.clearHighlights;
    destroy = highlighterMocks.destroy;
    setCurrentMatch = highlighterMocks.setCurrentMatch;
    constructor(
      _container: HTMLElement,
      _onChange?: unknown,
      clearance?: { bar: () => Element | null }
    ) {
      if (clearance) highlighterMocks.clearances.push(clearance);
    }
  },
}));

describe('SearchView', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    highlighterMocks.clearances.length = 0;
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        on: vi.fn(() => vi.fn()),
      },
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // The floor is the BAR's, and every surface's choice of it arrives through
  // here. Unforwarded, a list view asking for a one-character search silently
  // got the two-character default and its query never ran.
  it("hands the surface's minimum search length to the bar", () => {
    render(
      <SearchView minSearchLength={1}>
        <p>alpha beta</p>
      </SearchView>
    );

    fireEvent.keyDown(window, { key: 'f', metaKey: true });

    expect(screen.getByTestId('min-search-length')).toHaveTextContent('1');
  });

  // Vision V2-02: rendered inside the scrolled content, the bar scrolled away
  // with it once the highlighter made the viewport its containing block.
  it('mounts the bar in the transcript cell, outside the scrolling viewport', () => {
    render(
      <div data-testid="cell" data-preview-transcript="">
        <div data-testid="viewport" data-radix-scroll-area-viewport="">
          <SearchView>
            <p>alpha beta</p>
          </SearchView>
        </div>
      </div>
    );

    fireEvent.keyDown(window, { key: 'f', metaKey: true });

    const bar = screen.getByTestId('min-search-length').parentElement as HTMLElement;
    expect(screen.getByTestId('viewport').contains(bar)).toBe(false);
    expect(bar.parentElement).toBe(screen.getByTestId('cell'));
  });

  it('renders the bar in place when no transcript cell holds it', () => {
    const { container } = render(
      <SearchView>
        <p>alpha beta</p>
      </SearchView>
    );

    fireEvent.keyDown(window, { key: 'f', metaKey: true });

    const searchContainer = container.querySelector('.search-container') as HTMLElement;
    expect(searchContainer.contains(screen.getByTestId('min-search-length'))).toBe(true);
  });

  it('cancels a pending highlight when a newer term arrives', () => {
    render(
      <SearchView>
        <p>alpha beta</p>
      </SearchView>
    );

    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    fireEvent.click(screen.getByText('Search alpha'));
    fireEvent.click(screen.getByText('Search beta'));
    act(() => vi.advanceTimersByTime(150));

    expect(highlighterMocks.highlight).toHaveBeenCalledTimes(1);
    expect(highlighterMocks.highlight).toHaveBeenCalledWith('beta', false);
  });
  // Vision V2-R5-4: the current match was left under the bar. The clearance is
  // the find session's hold on the bar and on the room it asks of the
  // transcript, so it is one object for the whole session and it is what finds
  // the bar that was portalled into the cell.
  describe('the find bar clearance', () => {
    const renderInCell = () =>
      render(
        <div data-testid="cell" data-preview-transcript="">
          <div data-testid="viewport" data-radix-scroll-area-viewport="">
            <SearchView>
              <p>alpha beta</p>
            </SearchView>
          </div>
        </div>
      );

    it('builds every highlighter with one clearance that finds the portalled bar', () => {
      renderInCell();
      fireEvent.keyDown(window, { key: 'f', metaKey: true });
      fireEvent.click(screen.getByText('Search alpha'));
      fireEvent.click(screen.getByText('Search beta'));
      act(() => vi.advanceTimersByTime(150));

      // A new term makes a new highlighter; the clearance must outlive it, or
      // each keystroke would drop the room and ask for it again.
      expect(highlighterMocks.clearances).toHaveLength(2);
      expect(highlighterMocks.clearances[1]).toBe(highlighterMocks.clearances[0]);
      expect(highlighterMocks.releaseClearance).not.toHaveBeenCalled();

      const bar = screen.getByTestId('min-search-length').parentElement as HTMLElement;
      expect(highlighterMocks.clearances[0].bar()).toBe(bar);
      expect(screen.getByTestId('cell').contains(bar)).toBe(true);
    });

    it('finds no bar when it renders in place: there is no position to dodge', () => {
      render(
        <SearchView>
          <p>alpha beta</p>
        </SearchView>
      );
      fireEvent.keyDown(window, { key: 'f', metaKey: true });
      fireEvent.click(screen.getByText('Search alpha'));

      expect(highlighterMocks.clearances).toHaveLength(1);
      expect(highlighterMocks.clearances[0].bar()).toBeNull();
    });

    it('gives the room back when the bar closes, once', () => {
      renderInCell();
      fireEvent.keyDown(window, { key: 'f', metaKey: true });
      fireEvent.click(screen.getByText('Search alpha'));
      fireEvent.click(screen.getByText('Close find'));

      expect(highlighterMocks.releaseClearance).toHaveBeenCalledTimes(1);
      expect(highlighterMocks.destroy).toHaveBeenCalled();
    });

    it('gives the room back when the transcript unmounts with the bar open', () => {
      const { unmount } = renderInCell();
      fireEvent.keyDown(window, { key: 'f', metaKey: true });
      fireEvent.click(screen.getByText('Search alpha'));
      expect(highlighterMocks.releaseClearance).not.toHaveBeenCalled();

      unmount();

      expect(highlighterMocks.releaseClearance).toHaveBeenCalled();
    });
  });
});
