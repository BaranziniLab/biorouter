import React, { useEffect, useState, useRef, KeyboardEvent } from 'react';
import { ArrowDown, ArrowUp, Search as SearchIcon, X } from '../icons/app-icons';
import debounce from 'lodash/debounce';
import { Button } from '../ui/button';

/**
 * Props for the SearchBar component
 */
interface SearchBarProps {
  /** Callback fired when search term or case sensitivity changes */
  onSearch: (term: string, caseSensitive: boolean) => void;
  /** Callback fired when the search bar is closed */
  onClose: () => void;
  /** Optional callback for navigating between search results */
  onNavigate?: (direction: 'next' | 'prev') => void;
  /** Current search results state */
  searchResults?: {
    count: number;
    currentIndex: number;
  };
  /** Optional ref for the search input element */
  inputRef?: React.RefObject<HTMLInputElement>;
  /** Initial search term */
  initialSearchTerm?: string;
  /** Placeholder text for the search input */
  placeholder?: string;
  /**
   * How many characters this surface needs before it will search. Below it the
   * bar reports an EMPTY term and says so in as many words — see
   * {@link DEFAULT_MIN_SEARCH_LENGTH}.
   */
  minSearchLength?: number;
}

/**
 * The floor the HIGHLIGHTING surfaces keep — the chat and a saved transcript.
 *
 * ⚠ **It is a measured cost, not a taste.** `SearchView` answers a term by
 * walking its container's text nodes and creating a positioned overlay element
 * per match, and every match costs a `range.getClientRects()`, which is a forced
 * layout. Measured 2026-09-12 by replaying that loop in the running app over a
 * SHORT chat (2,170 characters of transcript): `e` found 217 matches and took
 * 432 ms to build the overlay and `a` 115 matches / 223 ms, against 40 ms for
 * the two-character `er` and 31 ms for `the`. The work is linear in matches at
 * ~2 ms each and a real transcript is two orders of magnitude longer, so a
 * one-character find there is seconds of blocked layout.
 *
 * A surface that only FILTERS A LIST pays none of that — its rows are filtered
 * before the highlighter ever sees them — so it passes `minSearchLength={1}`
 * and a one-character query reaches its matcher.
 */
export const DEFAULT_MIN_SEARCH_LENGTH = 2;

/**
 * SearchBar provides a search input with case-sensitive toggle and result navigation.
 */
export const SearchBar: React.FC<SearchBarProps> = ({
  onSearch,
  onClose,
  onNavigate,
  searchResults,
  inputRef: externalInputRef,
  initialSearchTerm = '',
  placeholder = 'Search chat...',
  minSearchLength = DEFAULT_MIN_SEARCH_LENGTH,
}: SearchBarProps) => {
  const [searchTerm, setSearchTerm] = useState(initialSearchTerm);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [isExiting, setIsExiting] = useState(false);
  const internalInputRef = React.useRef<HTMLInputElement>(null);
  const inputRef = externalInputRef || internalInputRef;
  const debouncedSearchRef = useRef<ReturnType<typeof debounce> | null>(null);
  const closeTimerRef = useRef<number | null>(null);

  // Create debounced search function
  useEffect(() => {
    const debouncedFn = debounce((term: string, caseSensitive: boolean) => {
      onSearch(term, caseSensitive);
    }, 200);

    debouncedSearchRef.current = debouncedFn;

    return () => {
      debouncedFn.cancel();
    };
  }, [onSearch]);

  useEffect(() => {
    inputRef.current?.focus();
  }, [inputRef]);

  useEffect(() => {
    return () => {
      if (closeTimerRef.current !== null) window.clearTimeout(closeTimerRef.current);
    };
  }, []);

  // Handle changes to initialSearchTerm
  useEffect(() => {
    if (initialSearchTerm) {
      setSearchTerm(initialSearchTerm);
      if (initialSearchTerm.length >= minSearchLength) {
        debouncedSearchRef.current?.(initialSearchTerm, caseSensitive);
      }
    }
  }, [initialSearchTerm, caseSensitive, debouncedSearchRef, minSearchLength]);

  const [localSearchResults, setLocalSearchResults] = useState<typeof searchResults>(undefined);

  // Sync external search results with local state
  useEffect(() => {
    // Only set results if we have a search term
    if (!searchTerm) {
      setLocalSearchResults(undefined);
    } else {
      setLocalSearchResults(searchResults);
    }
  }, [searchResults, searchTerm]);

  const handleSearch = (event: React.ChangeEvent<HTMLInputElement>) => {
    const value = event.target.value;

    // Always cancel pending searches first
    if (debouncedSearchRef.current) {
      debouncedSearchRef.current.cancel();
    }

    // Update display term immediately for UI feedback
    setSearchTerm(value);

    // Only trigger a search once the surface's minimum is reached. Below it the
    // term reported is EMPTY, which every list consumer reads as "no filter" —
    // so the bar owes the user the sentence below rather than a list that looks
    // filtered and is not.
    if (value.length >= minSearchLength) {
      debouncedSearchRef.current?.(value, caseSensitive);
    } else {
      onSearch('', caseSensitive);
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowUp') {
      handleNavigate('prev', event);
    } else if (event.key === 'ArrowDown' || event.key === 'Enter') {
      handleNavigate('next', event);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      handleClose();
    }
  };

  const handleNavigate = (direction: 'next' | 'prev', e?: React.MouseEvent | KeyboardEvent) => {
    e?.preventDefault();
    if (searchResults && searchResults.count > 0) {
      inputRef.current?.focus();
      onNavigate?.(direction);
    }
  };

  const toggleCaseSensitive = () => {
    const newCaseSensitive = !caseSensitive;
    setCaseSensitive(newCaseSensitive);
    // Immediately trigger a new search with updated case sensitivity. Guarded on
    // the SAME floor as typing: on `if (searchTerm)` a below-minimum term that
    // `handleSearch` had refused was searched anyway the moment `Aa` was clicked.
    if (searchTerm.length >= minSearchLength) {
      debouncedSearchRef.current?.(searchTerm, newCaseSensitive);
    }
    inputRef.current?.focus();
  };

  const cancelClose = React.useCallback(() => {
    if (!isExiting) return;
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    setIsExiting(false);
  }, [isExiting]);

  useEffect(() => {
    if (!isExiting) return;

    const handleFindAgain = (event: globalThis.KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === 'f') {
        cancelClose();
      }
    };

    window.addEventListener('keydown', handleFindAgain);
    return () => window.removeEventListener('keydown', handleFindAgain);
  }, [cancelClose, isExiting]);

  const handleClose = () => {
    if (isExiting) return;
    setIsExiting(true);
    inputRef.current?.blur();
    debouncedSearchRef.current?.cancel();

    if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
      onClose();
      return;
    }

    closeTimerRef.current = window.setTimeout(() => {
      closeTimerRef.current = null;
      onClose();
    }, 150);
  };

  const hasResults = searchResults && searchResults.count > 0;

  // The typed term is short of this surface's floor, so nothing was searched.
  // An empty box is not short of it: that is browsing, and the full list is the
  // honest answer there.
  const belowMinimum = searchTerm.length > 0 && searchTerm.length < minSearchLength;

  return (
    <div
      className={`pointer-events-none sticky top-0 z-[60] mb-4 flex justify-center px-3 pt-2 ${isExiting ? 'search-bar-exit' : 'search-bar-enter'}${belowMinimum ? ' search-bar-has-note' : ''}`}
    >
      <div
        data-testid="conversation-search-bar"
        className="pointer-events-auto flex w-full max-w-[720px] flex-col overflow-hidden rounded-2xl border border-border-subtle bg-background-default shadow-popover"
      >
        <div className="flex w-full items-center">
          <div className="relative flex flex-1 items-center h-full min-w-0">
            <SearchIcon className="no-drag h-4 w-4 text-text-muted absolute left-3" />
            <div className="w-full">
              <input
                ref={inputRef}
                id="search-input"
                type="text"
                value={searchTerm}
                onChange={handleSearch}
                onKeyDown={handleKeyDown}
                onFocus={cancelClose}
                placeholder={placeholder}
                className="no-drag w-full text-sm pl-9 pr-24 py-3 bg-background-default text-text-default placeholder:text-text-muted"
              />
            </div>

            <div className="absolute right-3 flex h-full items-center justify-end">
              <div className="flex items-center gap-1">
                <div className="w-16 text-right text-sm text-text-muted flex items-center justify-end">
                  {(() => {
                    return localSearchResults?.count && localSearchResults.count > 0 && searchTerm
                      ? `${localSearchResults.currentIndex}/${localSearchResults.count}`
                      : null;
                  })()}
                </div>
              </div>
            </div>
          </div>

          <div className="flex items-center justify-center h-auto px-3 gap-1.5 flex-shrink-0">
            <Button
              onClick={toggleCaseSensitive}
              variant="ghost"
              className={`no-drag flex items-center justify-center min-w-[32px] h-[28px] rounded transition-colors duration-[var(--motion-fast)] ${caseSensitive ? 'bg-background-medium text-text-default hover:bg-background-strong' : 'text-text-muted hover:text-text-default hover:bg-background-medium'}`}
              title="Case sensitive"
            >
              <span className="text-sm font-normal">Aa</span>
            </Button>

            <div className="flex items-center gap-2">
              <Button
                onClick={(e) => handleNavigate('prev', e)}
                variant="ghost"
                className="no-drag flex items-center justify-center min-w-[32px] h-[28px] rounded transition-colors duration-[var(--motion-fast)] text-text-muted hover:text-text-default hover:bg-background-medium"
                title="Previous (↑)"
              >
                <ArrowUp
                  className={`h-5 w-5 transition-opacity ${!hasResults ? 'opacity-30' : ''}`}
                />
              </Button>
              <Button
                onClick={(e) => handleNavigate('next', e)}
                variant="ghost"
                className="no-drag flex items-center justify-center min-w-[32px] h-[28px] rounded transition-colors duration-[var(--motion-fast)] text-text-muted hover:text-text-default hover:bg-background-medium"
                title="Next (↓ or Enter)"
              >
                <ArrowDown
                  className={`h-5 w-5 transition-opacity ${!hasResults ? 'opacity-30' : ''}`}
                />
              </Button>
            </div>

            <Button
              onClick={handleClose}
              variant="ghost"
              className="no-drag flex items-center justify-center min-w-[32px] h-[28px] rounded transition-colors duration-[var(--motion-fast)] text-text-muted hover:text-text-default hover:bg-background-medium"
              title="Close (Esc)"
            >
              <X className="h-5 w-5" />
            </Button>
          </div>
        </div>

        {/* ⚠ **A control that looks like it filtered and did not is the
            defect.** Below the floor the bar reports an empty term, so a list
            surface renders its WHOLE catalog under its browse headings — which
            reads as "your filter matched everything". This line is the surface
            saying, in as many words, that it has not filtered yet. */}
        {belowMinimum && (
          <div
            data-testid="conversation-search-bar-minimum"
            role="status"
            className="border-t border-border-subtle px-4 py-2 text-supporting text-text-muted"
          >
            {`Type at least ${minSearchLength} characters to search — everything is still showing.`}
          </div>
        )}
      </div>
    </div>
  );
};

export default SearchBar;
