import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import SearchBar from './SearchBar';

describe('SearchBar', () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    Reflect.deleteProperty(window, 'matchMedia');
  });

  it('renders as a centered floating search surface', () => {
    render(<SearchBar onSearch={vi.fn()} onClose={vi.fn()} />);

    const surface = screen.getByTestId('conversation-search-bar');

    // A floating search panel is a popover: 16px radius, --shadow-popover, and an
    // opaque fill (no backdrop blur). See design.md §3.4, §3.5 and the anti-patterns.
    expect(surface).toHaveClass('max-w-[720px]');
    expect(surface).toHaveClass('rounded-2xl');
    expect(surface).toHaveClass('shadow-popover');
    expect(surface).not.toHaveClass('backdrop-blur-md');
  });

  it('keeps search behavior while using the compact surface', () => {
    const onSearch = vi.fn();
    render(<SearchBar onSearch={onSearch} onClose={vi.fn()} />);

    fireEvent.change(screen.getByPlaceholderText('Search chat...'), {
      target: { value: 'agent' },
    });

    expect(screen.getByDisplayValue('agent')).toBeInTheDocument();
  });

  it('cancels an in-progress close when the user invokes find again', () => {
    vi.useFakeTimers();
    const onClose = vi.fn();
    render(<SearchBar onSearch={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByTitle('Close (Esc)'));
    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    act(() => vi.advanceTimersByTime(150));

    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByTestId('conversation-search-bar').parentElement).toHaveClass(
      'search-bar-enter'
    );
  });

  it('closes immediately when reduced motion is requested', () => {
    const onClose = vi.fn();
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({ matches: true }) as never),
    });
    render(<SearchBar onSearch={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByTitle('Close (Esc)'));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('clears the delayed close when it unmounts', () => {
    vi.useFakeTimers();
    const onClose = vi.fn();
    const { unmount } = render(<SearchBar onSearch={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByTitle('Close (Esc)'));
    unmount();
    act(() => vi.advanceTimersByTime(150));

    expect(onClose).not.toHaveBeenCalled();
  });

  /**
   * The minimum length used to be the literal `2`, the same on every surface
   * that mounts this bar. Two of them highlight a transcript, where a
   * one-character term is hundreds of forced layouts; the rest only filter a
   * short list of rows, where it is the whole point of the matcher's
   * short-term rule. The floor is a prop now, and this is both halves of it.
   */
  describe('minimum search length', () => {
    const type = (value: string) =>
      fireEvent.change(screen.getByPlaceholderText('Search chat...'), { target: { value } });

    it('searches a one-character term on a surface whose floor is one', () => {
      vi.useFakeTimers();
      const onSearch = vi.fn();
      render(<SearchBar onSearch={onSearch} onClose={vi.fn()} minSearchLength={1} />);

      type('r');
      act(() => vi.advanceTimersByTime(200));

      expect(onSearch).toHaveBeenCalledWith('r', false);
      expect(screen.queryByTestId('conversation-search-bar-minimum')).not.toBeInTheDocument();
    });

    it('says it has not searched yet when the term is below the floor', () => {
      vi.useFakeTimers();
      const onSearch = vi.fn();
      render(<SearchBar onSearch={onSearch} onClose={vi.fn()} />);

      type('r');
      act(() => vi.advanceTimersByTime(200));

      // The term it reports is empty — a list consumer draws its whole catalog
      // — so the bar has to SAY that, or the control looks like it filtered.
      expect(onSearch).toHaveBeenCalledWith('', false);
      expect(onSearch).not.toHaveBeenCalledWith('r', false);
      expect(screen.getByTestId('conversation-search-bar-minimum')).toHaveTextContent(
        'Type at least 2 characters to search'
      );
    });

    it('says nothing when the box is empty, which is browsing and not a short query', () => {
      render(<SearchBar onSearch={vi.fn()} onClose={vi.fn()} />);

      type('r');
      type('');

      expect(screen.queryByTestId('conversation-search-bar-minimum')).not.toBeInTheDocument();
    });

    it('asks the reveal animation for room for the second row', () => {
      // The bar opens under a `max-height` transition with `overflow: hidden`,
      // so the ceiling clips rather than scrolls: without this class the
      // sentence above is cut through its middle. jsdom cannot see that — the
      // stylesheet's half is asserted in `styles/searchBarNote.test.ts`.
      render(<SearchBar onSearch={vi.fn()} onClose={vi.fn()} />);

      type('r');

      expect(screen.getByTestId('conversation-search-bar').parentElement).toHaveClass(
        'search-bar-has-note'
      );
    });

    it('does not let the case toggle search a term typing was refused', () => {
      vi.useFakeTimers();
      const onSearch = vi.fn();
      render(<SearchBar onSearch={onSearch} onClose={vi.fn()} />);

      type('r');
      fireEvent.click(screen.getByTitle('Case sensitive'));
      act(() => vi.advanceTimersByTime(200));

      expect(onSearch).not.toHaveBeenCalledWith('r', true);
      expect(onSearch).not.toHaveBeenCalledWith('r', false);
    });
  });
});
