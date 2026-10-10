import React, { useEffect, useState, useRef, KeyboardEvent } from 'react';
import { ArrowDown, ArrowUp, Search as SearchIcon, X } from '../icons/app-icons';
import debounce from 'lodash/debounce';
import { Button } from '../ui/button';
import { InfoTip } from '../ui/info-tip';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { SEARCH_UI_ATTRIBUTE } from '../../utils/searchHighlighter';
import { prefersReducedMotion } from '../../styles/motion';
import { findInChatCopy } from './searchCopy';
import '../../styles/search.css';

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

/** The chat's composer field (`ChatInput`). */
const COMPOSER_FIELD = 'textarea[data-testid="chat-input"]';

/**
 * The composer of the chat the bar belongs to: the nearest one up the tree from
 * the bar, so a split's other panes are never chosen.
 */
function nearestComposer(from: HTMLElement | null): HTMLElement | null {
  for (let el = from?.parentElement ?? null; el && el !== document.body; el = el.parentElement) {
    const field = el.querySelector<HTMLElement>(COMPOSER_FIELD);
    if (field) return field;
  }
  return null;
}

/**
 * Find in chat (spec 3.16): an OVERLAY pinned to the top right of the
 * transcript it searches, so opening it pushes nothing. It is positioned by
 * `.br-find-overlay` in `styles/search.css` against the transcript cell
 * (`BaseChat`'s and `SessionHistoryView`'s `ScrollArea` root), which
 * `SearchView` portals it into. That box does not scroll, so the bar stays put
 * while the conversation scrolls under it.
 *
 * Focus: closing it hands focus back to where it was when the bar opened, or
 * else to the chat's composer, never to the page (spec principle 12).
 *
 * Motion: it opens with opacity and an 8px drop over `--dur-fast-max`, and
 * closes with opacity and a 4px rise over `--dur-fast`; `onClose` runs on the
 * close keyframe's `animationend`, or at once under reduced motion. No timer.
 *
 * It is no longer a list filter: the views filter with the band's
 * `FilterInput`, and `SearchView` is the transcript's find only.
 */
export const SearchBar: React.FC<SearchBarProps> = ({
  onSearch,
  onClose,
  onNavigate,
  searchResults,
  inputRef: externalInputRef,
  initialSearchTerm = '',
  placeholder = findInChatCopy.placeholder,
  minSearchLength = DEFAULT_MIN_SEARCH_LENGTH,
}: SearchBarProps) => {
  const [searchTerm, setSearchTerm] = useState(initialSearchTerm);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [isExiting, setIsExiting] = useState(false);
  // Read once, in the first render, before the field takes focus: the element
  // the person was in when they asked for the bar (vision V2-15).
  const [focusedBeforeOpen] = useState<Element | null>(() =>
    typeof document === 'undefined' ? null : document.activeElement
  );
  const overlayRef = useRef<HTMLDivElement>(null);
  const internalInputRef = React.useRef<HTMLInputElement>(null);
  const inputRef = externalInputRef || internalInputRef;
  const debouncedSearchRef = useRef<ReturnType<typeof debounce> | null>(null);

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

  // Escape used to leave focus on <body>: the field blurred and nothing took it.
  // Back to where the person was, unless that is gone or was the page itself,
  // and then to this chat's composer.
  const returnFocus = () => {
    inputRef.current?.blur();
    const overlay = overlayRef.current;
    const previous =
      focusedBeforeOpen instanceof HTMLElement &&
      focusedBeforeOpen !== document.body &&
      focusedBeforeOpen.isConnected &&
      !overlay?.contains(focusedBeforeOpen)
        ? focusedBeforeOpen
        : null;
    (previous ?? nearestComposer(overlay))?.focus({ preventScroll: true });
  };

  const handleClose = () => {
    if (isExiting) return;
    returnFocus();
    debouncedSearchRef.current?.cancel();

    // Reduced motion: no exit to wait for.
    if (prefersReducedMotion()) {
      onClose();
      return;
    }
    // Otherwise the close keyframe runs and `onAnimationEnd` below closes.
    setIsExiting(true);
  };

  const handleAnimationEnd = (event: React.AnimationEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget || !isExiting) return;
    onClose();
  };

  const hasResults = searchResults && searchResults.count > 0;

  // The typed term is short of this surface's floor, so nothing was searched.
  // An empty box is not short of it: that is browsing, and the full list is the
  // honest answer there.
  const belowMinimum = searchTerm.length > 0 && searchTerm.length < minSearchLength;

  const iconButton = 'no-drag text-text-muted hover:text-text-default';

  return (
    // Mounted bare (no transcript cell to portal into), the bar renders INSIDE
    // the container it searches: without the marker its own "Aa" label and
    // counter are counted as matches. See SEARCH_UI_ATTRIBUTE.
    <div
      ref={overlayRef}
      {...{ [SEARCH_UI_ATTRIBUTE]: '' }}
      className="br-find-overlay"
      data-state={isExiting ? 'closed' : 'open'}
      data-has-note={belowMinimum ? 'true' : undefined}
      onAnimationEnd={handleAnimationEnd}
    >
      <div
        data-testid="conversation-search-bar"
        role="search"
        className="br-find-bar flex w-full max-w-measure-chat flex-col overflow-hidden rounded-container border border-border-subtle bg-background-default shadow-popover"
      >
        <div className="br-find-row">
          <SearchIcon aria-hidden="true" className="br-find-icon" />
          <input
            ref={inputRef}
            id="search-input"
            type="text"
            value={searchTerm}
            onChange={handleSearch}
            onKeyDown={handleKeyDown}
            onFocus={cancelClose}
            placeholder={placeholder}
            aria-label={findInChatCopy.inputLabel}
            spellCheck={false}
            autoComplete="off"
            className="br-find-input no-drag"
          />
          <span className="br-find-count" aria-live="polite">
            {localSearchResults?.count && localSearchResults.count > 0 && searchTerm
              ? findInChatCopy.position(localSearchResults.currentIndex, localSearchResults.count)
              : null}
          </span>
          <InfoTip
            label={findInChatCopy.helpLabel}
            help={findInChatCopy.help()}
            side="bottom"
            align="end"
            className="no-drag"
          />
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                onClick={toggleCaseSensitive}
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={findInChatCopy.matchCase}
                aria-pressed={caseSensitive}
                className={`${iconButton} br-find-case`}
              >
                <span aria-hidden="true">Aa</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">{findInChatCopy.matchCase}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                onClick={(e) => handleNavigate('prev', e)}
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={findInChatCopy.previous}
                disabled={!hasResults}
                className={iconButton}
              >
                <ArrowUp />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">{findInChatCopy.previousTip}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                onClick={(e) => handleNavigate('next', e)}
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={findInChatCopy.next}
                disabled={!hasResults}
                className={iconButton}
              >
                <ArrowDown />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">{findInChatCopy.nextTip}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                onClick={handleClose}
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={findInChatCopy.close}
                className={iconButton}
              >
                <X />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">{findInChatCopy.closeTip}</TooltipContent>
          </Tooltip>
        </div>

        {/* ⚠ **A control that looks like it searched and did not is the
            defect.** Below the floor the bar reports an empty term and nothing
            is highlighted, so it says, in as many words, that it has not
            searched yet. */}
        {belowMinimum && (
          <div
            data-testid="conversation-search-bar-minimum"
            role="status"
            className="br-find-note text-supporting text-text-muted"
          >
            {findInChatCopy.belowMinimum(minSearchLength)}
          </div>
        )}
      </div>
    </div>
  );
};

export default SearchBar;
