import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SearchView } from './SearchView';

const highlighterMocks = vi.hoisted(() => ({
  highlight: vi.fn(() => []),
  clearHighlights: vi.fn(),
  destroy: vi.fn(),
  setCurrentMatch: vi.fn(),
}));

vi.mock('./SearchBar', () => ({
  default: ({
    onSearch,
    minSearchLength,
  }: {
    onSearch: (term: string, caseSensitive: boolean) => void;
    minSearchLength?: number;
  }) => (
    <div>
      <button onClick={() => onSearch('alpha', false)}>Search alpha</button>
      <button onClick={() => onSearch('beta', false)}>Search beta</button>
      <span data-testid="min-search-length">{String(minSearchLength)}</span>
    </div>
  ),
}));

vi.mock('../../utils/searchHighlighter', () => ({
  SearchHighlighter: class {
    highlight = highlighterMocks.highlight;
    clearHighlights = highlighterMocks.clearHighlights;
    destroy = highlighterMocks.destroy;
    setCurrentMatch = highlighterMocks.setCurrentMatch;
  },
}));

describe('SearchView', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
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
});
