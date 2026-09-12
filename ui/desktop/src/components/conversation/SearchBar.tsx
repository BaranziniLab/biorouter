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
}

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
      if (initialSearchTerm.length >= 2) {
        debouncedSearchRef.current?.(initialSearchTerm, caseSensitive);
      }
    }
  }, [initialSearchTerm, caseSensitive, debouncedSearchRef]);

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

    // Only trigger search if we have 2 or more characters
    if (value.length >= 2) {
      debouncedSearchRef.current?.(value, caseSensitive);
    } else {
      // Clear results if less than 2 characters
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
    // Immediately trigger a new search with updated case sensitivity
    if (searchTerm) {
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

  return (
    <div
      className={`pointer-events-none sticky top-0 z-[60] mb-4 flex justify-center px-3 pt-2 ${isExiting ? 'search-bar-exit' : 'search-bar-enter'}`}
    >
      <div
        data-testid="conversation-search-bar"
        className="pointer-events-auto flex w-full max-w-[720px] items-center overflow-hidden rounded-2xl border border-border-subtle bg-background-default shadow-popover"
      >
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
    </div>
  );
};

export default SearchBar;
