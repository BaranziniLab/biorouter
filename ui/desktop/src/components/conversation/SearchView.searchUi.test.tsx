/**
 * The search bar is rendered INSIDE the container it searches, so the highlighter
 * has to know which text is the bar's. It did not: the walker skipped
 * `.search-bar, .search-results`, no element carried either class, and the bar's
 * own "Aa" toggle was counted. The tester's repro, in a chat: ⌘F, type `aa`, and
 * the counter reads "1/1" with nothing highlighted anywhere visible — the mark
 * sat under the button. On Settings → Skills `a` read 1/18, and the first two
 * ⌘G stops landed on "A" and "a" of that label.
 *
 * Everything here is real — SearchView, SearchBar, SearchHighlighter — except
 * layout, which jsdom does not have: every range gets a 10px rect and every box
 * is `overflow: visible`, so a match counts exactly when the walker accepts its
 * text. The first assertion of each test is the control that proves that: a
 * match in the content does reach the counter, so "no counter" afterwards is the
 * walker's verdict and not an instrument that counts nothing. The geometry
 * rules themselves are pinned in a real browser, in
 * `utils/searchHighlighter.browser.test.ts`.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SearchView } from './SearchView';
import { SearchHighlighter } from '../../utils/searchHighlighter';

class ResizeObserverStub {
  observe = vi.fn();
  disconnect = vi.fn();
}

/** The bar's debounce (200 ms) plus SearchView's highlight delay (150 ms), with room. */
const SEARCH_SETTLE_MS = 400;

const counter = () => screen.queryByText(/^\d+\/\d+$/);

function openSearch(content: React.ReactNode) {
  render(<SearchView>{content}</SearchView>);
  fireEvent.keyDown(window, { key: 'f', metaKey: true });
}

function search(term: string) {
  fireEvent.change(screen.getByPlaceholderText('Search chat...'), { target: { value: term } });
  act(() => vi.advanceTimersByTime(SEARCH_SETTLE_MS));
}

describe('SearchView — the search bar is not something it finds', () => {
  let style: HTMLStyleElement;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal('ResizeObserver', ResizeObserverStub);
    Object.assign(window, { electron: { platform: 'darwin', on: vi.fn(() => vi.fn()) } });
    // jsdom has no layout: give every range a box, and let no box clip it.
    Object.defineProperty(Range.prototype, 'getClientRects', {
      configurable: true,
      value: () => [{ left: 0, top: 0, right: 10, bottom: 10 }],
    });
    style = document.createElement('style');
    style.textContent = '* { overflow-x: visible; overflow-y: visible; }';
    document.head.appendChild(style);
  });

  afterEach(() => {
    style.remove();
    Reflect.deleteProperty(Range.prototype, 'getClientRects');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it('counts the match in the content and not the case toggle\'s "Aa" label', () => {
    openSearch(<p>An aardvark</p>);

    search('aa');

    // The label is on screen and matches `aa` case-insensitively. Before the
    // fix this read "1/2", and match 1 was the label.
    expect(screen.getByText('Aa')).toBeInTheDocument();
    expect(counter()).toHaveTextContent(/^1\/1$/);
  });

  it('finds nothing when only the bar holds the term', () => {
    openSearch(<p>alpha beta</p>);

    search('al');
    expect(counter()).toHaveTextContent(/^1\/1$/);

    search('aa');
    expect(counter()).toBeNull();
  });

  it('re-walks the content for new content, and not for the counter appearing', async () => {
    const highlight = vi.spyOn(SearchHighlighter.prototype, 'highlight');
    openSearch(<p id="content">alpha beta</p>);

    search('al');
    expect(counter()).toHaveTextContent(/^1\/1$/);
    const passes = highlight.mock.calls.length;
    expect(passes).toBeGreaterThan(0);

    // ⚠ A MutationObserver reports in a microtask, which a synchronous test
    // never yields to — without this the check below passes whatever the
    // observer does. The counter's text node went into the bar just now.
    await act(async () => {});
    act(() => vi.advanceTimersByTime(500));
    expect(highlight.mock.calls.length).toBe(passes);

    // Control: a message arriving in the content is re-searched, 100 ms later.
    document.getElementById('content')!.appendChild(document.createElement('span')).textContent =
      ' alpaca';
    await act(async () => {});
    act(() => vi.advanceTimersByTime(500));
    expect(highlight.mock.calls.length).toBeGreaterThan(passes);
    expect(counter()).toHaveTextContent(/^1\/2$/);
  });
});
