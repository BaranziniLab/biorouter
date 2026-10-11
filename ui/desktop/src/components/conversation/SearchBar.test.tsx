import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import SearchBar from './SearchBar';
import { findInChatCopy } from './searchCopy';

describe('SearchBar', () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    Reflect.deleteProperty(window, 'matchMedia');
  });

  it('renders as a floating overlay surface, not a row in the transcript', () => {
    render(<SearchBar onSearch={vi.fn()} onClose={vi.fn()} />);

    const surface = screen.getByTestId('conversation-search-bar');

    // A floating search surface: the container radius (12px; 16px is the
    // preview sheet's alone), the chat measure as its ceiling, the one
    // floating-surface recipe (hairline + --shadow-popover, lifted in dark),
    // and an opaque fill (no backdrop blur).
    expect(surface).toHaveClass('max-w-measure-chat');
    expect(surface).toHaveClass('rounded-container');
    expect(surface).not.toHaveClass('rounded-2xl');
    expect(surface).toHaveClass('biorouter-popover-surface');
    expect(surface).toHaveClass('bg-background-default');
    expect(surface).not.toHaveClass('backdrop-blur-md');
    // The overlay class positions it (styles/search.css); nothing sticky.
    const overlay = surface.parentElement as HTMLElement;
    expect(overlay).toHaveClass('br-find-overlay');
    expect(overlay.className).not.toMatch(/sticky|mb-4/);
    expect(overlay).toHaveAttribute('data-state', 'open');
  });

  it('keeps search behavior while using the compact surface', () => {
    const onSearch = vi.fn();
    render(<SearchBar onSearch={onSearch} onClose={vi.fn()} />);

    fireEvent.change(screen.getByPlaceholderText(findInChatCopy.placeholder), {
      target: { value: 'agent' },
    });

    expect(screen.getByDisplayValue('agent')).toBeInTheDocument();
  });

  it('names its controls and puts the shortcuts in tooltips, never in title=', () => {
    render(<SearchBar onSearch={vi.fn()} onClose={vi.fn()} />);

    expect(screen.getByRole('textbox', { name: findInChatCopy.inputLabel })).toBeInTheDocument();
    for (const name of [
      findInChatCopy.matchCase,
      findInChatCopy.previous,
      findInChatCopy.next,
      findInChatCopy.close,
    ]) {
      const button = screen.getByRole('button', { name });
      expect(button).not.toHaveAttribute('title');
    }
    // The ⌘F load-all tip is the field's InfoTip, read as a description.
    expect(
      screen.getByRole('button', { name: `About ${findInChatCopy.helpLabel}` })
    ).toHaveAccessibleDescription(findInChatCopy.help());
  });

  it('closes on the close keyframe ending, not on a timer', () => {
    vi.useFakeTimers();
    const onClose = vi.fn();
    render(<SearchBar onSearch={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByRole('button', { name: findInChatCopy.close }));
    const overlay = screen.getByTestId('conversation-search-bar').parentElement as HTMLElement;
    expect(overlay).toHaveAttribute('data-state', 'closed');

    // Time alone closes nothing.
    act(() => vi.advanceTimersByTime(1000));
    expect(onClose).not.toHaveBeenCalled();

    // A child's animation ending is not the bar's.
    fireEvent.animationEnd(screen.getByTestId('conversation-search-bar'));
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.animationEnd(overlay);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('cancels an in-progress close when the user invokes find again', () => {
    const onClose = vi.fn();
    render(<SearchBar onSearch={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByRole('button', { name: findInChatCopy.close }));
    fireEvent.keyDown(window, { key: 'f', metaKey: true });

    const overlay = screen.getByTestId('conversation-search-bar').parentElement as HTMLElement;
    expect(overlay).toHaveAttribute('data-state', 'open');
    // The opening keyframe ending closes nothing.
    fireEvent.animationEnd(overlay);
    expect(onClose).not.toHaveBeenCalled();
  });

  it('closes immediately when reduced motion is requested', () => {
    const onClose = vi.fn();
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({ matches: true }) as never),
    });
    render(<SearchBar onSearch={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByRole('button', { name: findInChatCopy.close }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Escape in the field starts the close', () => {
    const onClose = vi.fn();
    render(<SearchBar onSearch={vi.fn()} onClose={onClose} />);

    fireEvent.keyDown(screen.getByRole('textbox', { name: findInChatCopy.inputLabel }), {
      key: 'Escape',
    });
    const overlay = screen.getByTestId('conversation-search-bar').parentElement as HTMLElement;
    expect(overlay).toHaveAttribute('data-state', 'closed');
    fireEvent.animationEnd(overlay);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // Vision V2-15: closing the bar with Escape left focus on <body>.
  describe('focus after closing', () => {
    const field = () => screen.getByRole('textbox', { name: findInChatCopy.inputLabel });

    it('Escape gives focus back to the element that had it when the bar opened', () => {
      const before = document.createElement('button');
      document.body.appendChild(before);
      try {
        before.focus();
        render(<SearchBar onSearch={vi.fn()} onClose={vi.fn()} />);
        expect(document.activeElement).toBe(field());

        fireEvent.keyDown(field(), { key: 'Escape' });
        expect(document.activeElement).toBe(before);
      } finally {
        before.remove();
      }
    });

    it("falls back to this chat's composer when nothing had focus, and never another pane's", () => {
      render(
        <div>
          <div>
            <textarea data-testid="chat-input" aria-label="other pane" />
          </div>
          <div>
            <div data-preview-transcript="">
              <SearchBar onSearch={vi.fn()} onClose={vi.fn()} />
            </div>
            <textarea data-testid="chat-input" aria-label="this pane" />
          </div>
        </div>
      );

      fireEvent.click(screen.getByRole('button', { name: findInChatCopy.close }));
      expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'this pane' }));
    });

    it('falls back to the composer when the element it came from has gone', () => {
      const before = document.createElement('button');
      document.body.appendChild(before);
      before.focus();
      render(
        <div>
          <SearchBar onSearch={vi.fn()} onClose={vi.fn()} />
          <textarea data-testid="chat-input" aria-label="composer" />
        </div>
      );
      before.remove();

      fireEvent.keyDown(field(), { key: 'Escape' });
      expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'composer' }));
    });
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
      fireEvent.change(screen.getByPlaceholderText(findInChatCopy.placeholder), {
        target: { value },
      });

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

    it('marks the two-row state on the overlay, which grows to fit it', () => {
      // The overlay has no ceiling to clip the second row (it used to be a
      // `max-height` reveal); `styles/searchBarNote.test.ts` asserts the
      // stylesheet's half, which jsdom cannot see.
      render(<SearchBar onSearch={vi.fn()} onClose={vi.fn()} />);

      type('r');

      expect(screen.getByTestId('conversation-search-bar').parentElement).toHaveAttribute(
        'data-has-note',
        'true'
      );
      expect(screen.getByTestId('conversation-search-bar-minimum')).toHaveTextContent(
        findInChatCopy.belowMinimum(2)
      );
    });

    it('does not let the case toggle search a term typing was refused', () => {
      vi.useFakeTimers();
      const onSearch = vi.fn();
      render(<SearchBar onSearch={onSearch} onClose={vi.fn()} />);

      type('r');
      fireEvent.click(screen.getByRole('button', { name: findInChatCopy.matchCase }));
      act(() => vi.advanceTimersByTime(200));

      expect(onSearch).not.toHaveBeenCalledWith('r', true);
      expect(onSearch).not.toHaveBeenCalledWith('r', false);
    });
  });
});
