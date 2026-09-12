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
