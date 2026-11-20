import { useCallback, useEffect, useId, useRef, useState } from 'react';
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Mic,
  MoreHorizontal,
  RefreshCw,
  Share2,
  Square,
  X,
} from '../icons/app-icons';
import { cn } from '../../utils';
import { Button } from '../ui/button';
import { PREVIEW_COPY } from './copy';
import { PreviewIconButton } from './PreviewBar';
import { previewOpenTarget } from '../../utils/openInBrowser';
import { isBrowserSurface } from '../../utils/surface';
import { copyToClipboard } from '../../utils/clipboard';
import { addressFieldTarget, type SearchEngine } from '../../browser/addressBar';
import { ChromeImportDialog } from '../browser/chromeImport/ChromeImportDialog';
import { ChromeImportOffer } from '../browser/chromeImport/ChromeImportOffer';
import { CHROME_IMPORT_COPY } from '../browser/chromeImport/copy';
import { useChromeImportStatus } from '../browser/chromeImport/useChromeImportStatus';
import type {
  EmbeddedBrowserBookmark,
  EmbeddedBrowserState,
  EmbeddedBrowserSuggestion,
} from '../../browser/types';
import { crewPrivateTermsAllowed } from '../dictation/CrewDictationButton';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '../ui/context-menu';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { DICTATION_ACTIONS, dictationMessage } from '../dictation/copy';

export type { EmbeddedBrowserState } from '../../browser/types';

export type LiveBrowserShare = {
  viewId: string;
  state: EmbeddedBrowserState;
};

/** The last "load this link again" each browser tab has acted on (see `navigateRevision`). */
const consumedNavigation = new Map<string, number>();

/**
 * The Biorouter Browser's page in the artifact panel (`docs/desktop-ui/built-in-browser.md`).
 *
 * **This component renders no page content.** The page lives in a
 * `WebContentsView` owned by the main process — a real top-level browsing
 * context, so clicking, typing, scrolling, JavaScript and ordinary site cookies
 * behave as they do in Chrome, and sites that refuse to be framed (about half
 * the ones this audience opens) load normally.
 *
 * The tab is the main process's, not this component's (BB-01): mounting attaches to it and
 * unmounting only hides it, so a panel tab keeps its page, history and scroll position while
 * another tab is shown. The panel destroys the tab when its panel tab closes.
 *
 * What this component owns is the *hole* the native view is painted into, and
 * the chrome around it. The native view has no shared z-index with the DOM, so
 * two rules follow and both are load-bearing:
 *
 * 1. The slot must not live inside a scrolling container — the view cannot clip
 *    to one. The page scrolls itself instead.
 * 2. Anything that needs to sit on top (a modal, a dropdown, the resize shield,
 *    this bar's own ⋯ menu and address suggestions) requires hiding the view,
 *    which is what `isSuspended` and `coveredBySelf` are for.
 */
export default function WebPagePreview({
  url,
  viewId: viewIdProp,
  requireManagedApp = false,
  isSuspended = false,
  snapshotDataUrl,
  onOpenExternal,
  onOpenInNewTab,
  onCloseTab,
  onViewIdChange,
  onAgentShareChange,
  onStopAgent,
  onStateChange,
  refreshRevision = 0,
  navigateRevision,
  allowChromeImport = false,
}: {
  url: string;
  /** The browser tab to show. Stable for the panel tab; a page-opened tab names its own. */
  viewId?: string;
  /** Fail closed if this URL is no longer owned by the active BioRouter daemon. */
  requireManagedApp?: boolean;
  /** Hide the native view because something must paint above it. */
  isSuspended?: boolean;
  /** A compositor snapshot shown while the native view is hidden for annotation. */
  snapshotDataUrl?: string | null;
  onOpenExternal: (url: string) => void;
  /** Open `url` in a new browser tab of this panel (a blocked popup's note). */
  onOpenInNewTab?: (url: string) => void;
  /** Cmd or Ctrl W pressed while the page had focus. */
  onCloseTab?: () => void;
  onViewIdChange?: (viewId: string | null) => void;
  onAgentShareChange?: (share: LiveBrowserShare | null) => void;
  /**
   * BB-15: Stop, offered in the bar while the agent is acting on this tab. The same revoke as
   * the agent control above the composer: it ends the browser's and Copilot's grants.
   */
  onStopAgent?: () => void;
  /** The page's state as it changes, for the panel tab's title. */
  onStateChange?: (state: EmbeddedBrowserState | null) => void;
  refreshRevision?: number;
  /**
   * The time the person last clicked the link this tab was opened for. A newer value than the
   * tab has acted on loads `url` again, so a second click on a chat link goes back to it.
   */
  navigateRevision?: number;
  /**
   * BB-21: a tab the person opened may offer **Import from Chrome** (the first use card and the
   * ⋯ menu). Never an app preview or a tab the agent opened.
   */
  allowChromeImport?: boolean;
}) {
  const slotRef = useRef<HTMLDivElement | null>(null);
  const addressRef = useRef<HTMLInputElement | null>(null);
  const findRef = useRef<HTMLInputElement | null>(null);
  const reactId = useId();
  const fallbackViewId = useRef(`embedded-browser-${reactId}`);
  const viewId = viewIdProp ?? fallbackViewId.current;
  const [state, setState] = useState<EmbeddedBrowserState | null>(null);
  const [addressDraft, setAddressDraft] = useState(url);
  const [isEditingAddress, setIsEditingAddress] = useState(false);
  // L2: why the address field did not open what was typed (a `file:` or `javascript:` address).
  const [addressRefusal, setAddressRefusal] = useState<string | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [sharedWithAgent, setSharedWithAgent] = useState(false);
  // BB-15: an agent operation is running on this tab right now (main reports it).
  const [agentActive, setAgentActive] = useState(false);
  const [updateReady, setUpdateReady] = useState(false);
  const [editableFocused, setEditableFocused] = useState(false);
  const [privateAllowed, setPrivateAllowed] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [bookmarks, setBookmarks] = useState<EmbeddedBrowserBookmark[]>([]);
  const [suggestions, setSuggestions] = useState<EmbeddedBrowserSuggestion[]>([]);
  const [highlighted, setHighlighted] = useState(-1);
  const [findOpen, setFindOpen] = useState(false);
  const [findText, setFindText] = useState('');
  const selectOnMouseUp = useRef(false);
  const refreshRef = useRef({ url, consumed: refreshRevision, generation: 0 });
  const latestRevision = useRef(refreshRevision);
  latestRevision.current = refreshRevision;
  const urlRef = useRef(url);
  urlRef.current = url;
  const navigateRevisionRef = useRef(navigateRevision);
  navigateRevisionRef.current = navigateRevision;
  const listboxId = useId();

  const browser = window.electron?.embeddedBrowser;
  const showingSuggestions = isEditingAddress && suggestions.length > 0;
  const importStatus = useChromeImportStatus();
  const [importOpen, setImportOpen] = useState(false);
  const [offerAnswered, setOfferAnswered] = useState(false);
  const [searchEngine, setSearchEngine] = useState<SearchEngine | undefined>(undefined);
  const showImportOffer =
    allowChromeImport &&
    !requireManagedApp &&
    !state?.managedApp &&
    !offerAnswered &&
    importStatus?.offerFirstUse === true;
  // The bar's own surfaces that would sit over the page hide it while they are up, and so do the
  // Chrome import's card and dialog: the page is a native view above every DOM layer.
  const coveredBySelf = menuOpen || showingSuggestions || importOpen || showImportOffer;

  // BB-22: the address field searches with Chrome's engine once it was imported.
  useEffect(() => {
    if (!browser?.searchEngine || importOpen) return;
    let active = true;
    void browser.searchEngine().then((engine) => {
      if (active) setSearchEngine(engine ?? undefined);
    });
    return () => {
      active = false;
    };
  }, [browser, importOpen]);

  // Dictate into this app is shown only while an editable field in the app has focus
  // (owner decision, 2026-11-10). It does not depend on Dictate into other apps.
  // BB-15: main reports when an agent operation starts and ends on this tab.
  useEffect(() => {
    const agent = window.electron?.browserAgent;
    if (!agent?.onActivity) return;
    setAgentActive(false);
    return agent.onActivity((activity) => {
      if (activity.viewId === viewId) setAgentActive(activity.active);
    });
  }, [viewId]);

  useEffect(() => {
    if (
      isBrowserSurface() ||
      !state?.managedApp ||
      !window.electron?.dictationAnywherePreviewFocused
    ) {
      setEditableFocused(false);
      return;
    }
    let active = true;
    const refresh = () =>
      void window.electron.dictationAnywherePreviewFocused?.(viewId).then((focused) => {
        if (active) setEditableFocused(focused);
      });
    refresh();
    const timer = window.setInterval(refresh, 500);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [state?.managedApp, viewId]);

  // Attach to the tab (creating it the first time) and hide it again on unmount. The view is a
  // child of the *window*, not of this tree; the panel destroys it when its tab closes.
  useEffect(() => {
    if (!browser) {
      setUnavailable(true);
      return;
    }
    let disposed = false;
    setUnavailable(false);
    setSharedWithAgent(false);
    setState(null);
    setUpdateReady(false);
    refreshRef.current = {
      url: urlRef.current,
      consumed: latestRevision.current,
      generation: refreshRef.current.generation + 1,
    };

    const stopListening = browser.onState((payload) => {
      if (payload.viewId === viewId && !disposed) setState(payload.state);
    });

    const target = urlRef.current;
    const creation = requireManagedApp
      ? browser.create(viewId, target, true)
      : browser.create(viewId, target);
    void creation.then((initial) => {
      if (disposed) return;
      if (!initial) {
        setUnavailable(true);
        return;
      }
      setState(initial);
      onViewIdChange?.(viewId);
      const requested = navigateRevisionRef.current;
      const consumed = consumedNavigation.get(viewId);
      if (requested !== undefined) {
        if (consumed !== undefined && consumed !== requested && initial.url !== target) {
          void browser.navigate(viewId, target);
        }
        consumedNavigation.set(viewId, requested);
      }
    });

    return () => {
      disposed = true;
      refreshRef.current.generation += 1;
      stopListening();
      onViewIdChange?.(null);
      onAgentShareChange?.(null);
      void (browser.release ?? browser.destroy)(viewId);
    };
  }, [browser, onAgentShareChange, onViewIdChange, requireManagedApp, viewId]);

  // A second click on the link this tab was opened for, while the tab is shown.
  useEffect(() => {
    if (!browser || navigateRevision === undefined || !state) return;
    const consumed = consumedNavigation.get(viewId);
    if (consumed === undefined || consumed === navigateRevision) return;
    consumedNavigation.set(viewId, navigateRevision);
    if (state.url !== url) void browser.navigate(viewId, url);
  }, [browser, navigateRevision, state, url, viewId]);

  useEffect(() => {
    onStateChange?.(state);
  }, [onStateChange, state]);

  useEffect(() => {
    const current = refreshRef.current;
    if (!browser || !state || unavailable || current.consumed === refreshRevision) return;
    if (!state.managedApp) {
      current.consumed = refreshRevision;
      return;
    }
    if (isSuspended || isEditingAddress || state.isLoading) return;
    current.consumed = refreshRevision;
    const generation = ++current.generation;
    void browser
      .control(viewId, 'reload-if-idle')
      .then((reloaded) => {
        if (refreshRef.current.generation === generation) setUpdateReady(!reloaded);
      })
      .catch(() => {
        if (refreshRef.current.generation === generation) setUpdateReady(true);
      });
  }, [browser, isEditingAddress, isSuspended, refreshRevision, state, unavailable, viewId]);

  useEffect(() => {
    onAgentShareChange?.(sharedWithAgent && !unavailable && state ? { viewId, state } : null);
  }, [onAgentShareChange, sharedWithAgent, state, unavailable, viewId]);

  useEffect(() => () => onAgentShareChange?.(null), [onAgentShareChange]);

  // Keep the native view over the slot. A ResizeObserver alone is not enough:
  // the panel can move without changing size (the window moves, the chat pane
  // resizes beside it), so scroll and window resize are watched too.
  useEffect(() => {
    const slot = slotRef.current;
    if (!slot || !browser || unavailable) return;

    let frame = 0;
    const sync = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const rect = slot.getBoundingClientRect();
        void browser.setBounds(viewId, {
          x: rect.left,
          y: rect.top,
          width: rect.width,
          height: rect.height,
        });
      });
    };

    sync();
    // Guarded like the panel's other previews: jsdom implements no
    // ResizeObserver, and the window/scroll listeners below still give correct
    // (if less responsive) behaviour without it.
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(sync);
    observer?.observe(slot);
    window.addEventListener('resize', sync);
    // Capture phase: a scroll anywhere in an ancestor moves the slot.
    window.addEventListener('scroll', sync, true);
    return () => {
      cancelAnimationFrame(frame);
      observer?.disconnect();
      window.removeEventListener('resize', sync);
      window.removeEventListener('scroll', sync, true);
    };
  }, [browser, unavailable, viewId]);

  // Visibility is separate from bounds so suspending never loses the geometry.
  useEffect(() => {
    if (!browser || unavailable) return;
    void browser.setVisible(viewId, !isSuspended && !coveredBySelf);
    return () => {
      void browser.setVisible(viewId, false);
    };
  }, [browser, coveredBySelf, isSuspended, unavailable, viewId]);

  useEffect(() => {
    if (!isEditingAddress && state?.url) setAddressDraft(state.url);
  }, [isEditingAddress, state?.url]);

  // Keys pressed while the page had focus (main maps them, BB toolbar and keyboard).
  useEffect(() => {
    if (!browser?.onShortcut) return;
    return browser.onShortcut((payload) => {
      if (payload.viewId !== viewId) return;
      if (payload.action === 'focus-address') {
        addressRef.current?.focus();
        addressRef.current?.select();
      } else if (payload.action === 'find') {
        setFindOpen(true);
        window.requestAnimationFrame(() => {
          findRef.current?.focus();
          findRef.current?.select();
        });
      } else if (payload.action === 'close-tab') {
        onCloseTab?.();
      }
    });
  }, [browser, onCloseTab, viewId]);

  // Suggestions from the browser's own history and bookmarks, for the person only.
  useEffect(() => {
    if (!browser?.suggest || !isEditingAddress) {
      setSuggestions([]);
      return;
    }
    const query = addressDraft.trim();
    if (!query || query === state?.url) {
      setSuggestions([]);
      return;
    }
    let active = true;
    const timer = window.setTimeout(() => {
      void browser.suggest(query).then((found) => {
        if (active) {
          setSuggestions(found.filter((item) => item.url !== state?.url).slice(0, 8));
          setHighlighted(-1);
        }
      });
    }, 120);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [addressDraft, browser, isEditingAddress, state?.url]);

  const control = useCallback(
    (action: 'back' | 'forward' | 'reload' | 'stop') => {
      if (action === 'reload') {
        refreshRef.current.generation += 1;
        refreshRef.current.consumed = refreshRevision;
        setUpdateReady(false);
      }
      if (browser) void browser.control(viewId, action);
    },
    [browser, refreshRevision, viewId]
  );

  const openAddress = useCallback(
    (target: string) => {
      if (!browser) return;
      void browser.navigate(viewId, target);
      setIsEditingAddress(false);
      setSuggestions([]);
      addressRef.current?.blur();
    },
    [browser, viewId]
  );

  const submitAddress = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();
      if (highlighted >= 0 && suggestions[highlighted]) {
        openAddress(suggestions[highlighted].url);
        return;
      }
      const target = addressFieldTarget(addressDraft, searchEngine);
      if (!target) return;
      if (target.kind === 'refused') {
        setAddressRefusal(target.reason);
        setSuggestions([]);
        return;
      }
      setAddressRefusal(null);
      openAddress(target.url);
    },
    [addressDraft, highlighted, openAddress, searchEngine, suggestions]
  );

  const runFind = useCallback(
    (text: string, findNext: boolean, forward = true) => {
      if (!browser?.find) return;
      void browser.find(viewId, text, { forward, findNext });
    },
    [browser, viewId]
  );

  const closeFind = useCallback(() => {
    setFindOpen(false);
    setFindText('');
    if (browser?.stopFind) void browser.stopFind(viewId);
  }, [browser, viewId]);

  if (unavailable) {
    return (
      <div className="br-preview-state" data-preview-native="">
        <p className="br-preview-state__line text-body text-text-muted">
          {PREVIEW_COPY.pageUnavailable}
        </p>
        <Button type="button" variant="secondary" size="sm" onClick={() => onOpenExternal(url)}>
          {PREVIEW_COPY.openInBrowserShort}
        </Button>
      </div>
    );
  }

  const busy = state?.isLoading ?? true;
  const managed = Boolean(state?.managedApp);
  const zoomPercent = Math.round((state?.zoomFactor ?? 1) * 100);
  const notice = state?.notice ?? null;
  const download = state?.download ?? null;
  const openTarget = previewOpenTarget(url, state);

  const runNoticeAction = () => {
    const action = notice?.action;
    if (!action || !browser) return;
    if (action.kind === 'open-tab') onOpenInNewTab?.(action.url);
    else if (action.kind === 'open-here') void browser.navigate(viewId, action.url);
    else onOpenExternal(action.url);
    if (action.kind !== 'open-here') void browser.dismissNotice?.(viewId);
  };

  return (
    // `data-preview-native`: a native view cannot move with the DOM, so the
    // panel's switch fades this layer and never slides it (preview-panel.css).
    <div className="flex h-full min-h-0 flex-col" data-preview-native="">
      {/* The page's own toolbar stands in for the panel's 32px bar: same height,
          same hairline, same 24px icon controls. */}
      <div className="br-preview-bar br-preview-bar--browser">
        <ToolbarButton
          label={BROWSER_COPY.back}
          disabled={!state?.canGoBack}
          onClick={() => control('back')}
          icon={<ChevronLeft className="h-3.5 w-3.5" aria-hidden="true" />}
        />
        <ToolbarButton
          label={BROWSER_COPY.forward}
          disabled={!state?.canGoForward}
          onClick={() => control('forward')}
          icon={<ChevronRight className="h-3.5 w-3.5" aria-hidden="true" />}
        />
        <ToolbarButton
          label={busy ? BROWSER_COPY.stop : BROWSER_COPY.reload}
          onClick={() => control(busy ? 'stop' : 'reload')}
          icon={
            busy ? (
              <X className="h-3.5 w-3.5" aria-hidden="true" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
            )
          }
        />
        {findOpen ? (
          <form
            className="flex min-w-0 flex-1 items-center gap-1"
            onSubmit={(event) => {
              event.preventDefault();
              runFind(findText, true);
            }}
          >
            <input
              ref={findRef}
              aria-label={BROWSER_COPY.findInPage}
              placeholder={BROWSER_COPY.findInPage}
              value={findText}
              spellCheck={false}
              onChange={(event) => {
                setFindText(event.target.value);
                runFind(event.target.value, false);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  event.preventDefault();
                  closeFind();
                } else if (event.key === 'Enter' && event.shiftKey) {
                  event.preventDefault();
                  runFind(findText, true, false);
                }
              }}
              className="br-field h-6 w-full min-w-0 truncate rounded-element border px-2 text-secondary text-text-default placeholder:text-text-muted"
            />
            <span
              className="flex-none text-supporting tabular-nums text-text-muted"
              aria-live="polite"
            >
              {findText && state?.find
                ? BROWSER_COPY.matches(state.find.activeMatchOrdinal, state.find.matches)
                : ''}
            </span>
            <ToolbarButton
              label={BROWSER_COPY.previousMatch}
              disabled={!findText}
              onClick={() => runFind(findText, true, false)}
              icon={<ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />}
            />
            <ToolbarButton
              label={BROWSER_COPY.nextMatch}
              disabled={!findText}
              onClick={() => runFind(findText, true)}
              icon={<ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />}
            />
            <ToolbarButton
              label={BROWSER_COPY.closeFind}
              onClick={closeFind}
              icon={<X className="h-3.5 w-3.5" aria-hidden="true" />}
            />
          </form>
        ) : (
          <form
            onSubmit={submitAddress}
            className="relative flex min-w-0 flex-1 items-center gap-1"
          >
            {state?.isLocal && (
              <span
                className="flex-none rounded-inner bg-background-fill px-1.5 text-supporting text-text-muted"
                title={BROWSER_COPY.localHint}
                data-testid="browser-local-chip"
              >
                {BROWSER_COPY.local}
              </span>
            )}
            <input
              ref={addressRef}
              aria-label={BROWSER_COPY.address}
              value={addressDraft}
              spellCheck={false}
              autoComplete="off"
              role="combobox"
              aria-expanded={showingSuggestions}
              aria-controls={showingSuggestions ? listboxId : undefined}
              aria-autocomplete="list"
              aria-activedescendant={
                showingSuggestions && highlighted >= 0 ? `${listboxId}-${highlighted}` : undefined
              }
              onMouseDown={(event) => {
                // The first click into the field selects the whole address, as in Chrome.
                selectOnMouseUp.current = document.activeElement !== event.currentTarget;
              }}
              onMouseUp={(event) => {
                if (selectOnMouseUp.current) {
                  event.preventDefault();
                  event.currentTarget.select();
                }
                selectOnMouseUp.current = false;
              }}
              onFocus={(event) => {
                setIsEditingAddress(true);
                event.currentTarget.select();
              }}
              onChange={(event) => {
                setAddressDraft(event.target.value);
                setIsEditingAddress(true);
                setAddressRefusal(null);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  event.preventDefault();
                  setAddressDraft(state?.url ?? url);
                  setSuggestions([]);
                  event.currentTarget.blur();
                } else if (event.key === 'ArrowDown' && suggestions.length > 0) {
                  event.preventDefault();
                  setHighlighted((index) => Math.min(suggestions.length - 1, index + 1));
                } else if (event.key === 'ArrowUp' && suggestions.length > 0) {
                  event.preventDefault();
                  setHighlighted((index) => Math.max(-1, index - 1));
                }
              }}
              onBlur={() => {
                setIsEditingAddress(false);
                setSuggestions([]);
              }}
              // The app's filled field (R-27): a fill at rest, the strong step under
              // the pointer, the accent edge only while it is being typed in.
              className="br-field h-6 w-full min-w-0 truncate rounded-element border px-2 text-secondary text-text-default transition-[color,background-color,border-color] placeholder:text-text-muted"
            />
          </form>
        )}
        {zoomPercent !== 100 && !managed && (
          <button
            type="button"
            className="br-preview-icon-button no-drag flex-none rounded-inner px-1 text-supporting tabular-nums text-text-muted hover:bg-overlay-hover"
            aria-label={BROWSER_COPY.resetZoom(zoomPercent)}
            title={BROWSER_COPY.resetZoom(zoomPercent)}
            onClick={() => void browser?.zoom?.(viewId, 'reset')}
          >
            {zoomPercent}%
          </button>
        )}
        {updateReady && (
          <PreviewIconButton
            compact
            label={PREVIEW_COPY.updateReady}
            hint={PREVIEW_COPY.updateReadyHint}
            disabled={busy || isSuspended}
            onClick={() => control('reload')}
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
            <span
              className="br-preview-update-dot br-preview-update-dot--compact"
              aria-hidden="true"
            />
          </PreviewIconButton>
        )}
        {!isBrowserSurface() && state?.managedApp && editableFocused && (
          <ContextMenu
            onOpenChange={(open) => {
              setPrivateAllowed(false);
              if (open) void crewPrivateTermsAllowed().then(setPrivateAllowed);
            }}
          >
            <ContextMenuTrigger asChild>
              <span>
                <ToolbarButton
                  label={DICTATION_ACTIONS.dictateIntoThisApp}
                  disabled={isSuspended}
                  onClick={() =>
                    void window.electron.dictationAnywhereStartPreview?.(viewId, false)
                  }
                  icon={<Mic className="h-3.5 w-3.5" aria-hidden="true" />}
                />
              </span>
            </ContextMenuTrigger>
            {privateAllowed && (
              <ContextMenuContent>
                <ContextMenuItem
                  onSelect={() =>
                    void window.electron
                      .showMessageBox({
                        type: 'warning',
                        title: DICTATION_ACTIONS.includePrivateTerms,
                        message: dictationMessage('MSG-PRIVATE-TERMS'),
                        buttons: [DICTATION_ACTIONS.include, DICTATION_ACTIONS.publicTermsOnly],
                        defaultId: 1,
                      })
                      .then(({ response }) => {
                        void window.electron.dictationAnywhereStartPreview?.(
                          viewId,
                          response === 0
                        );
                      })
                  }
                >
                  {DICTATION_ACTIONS.includePrivateTerms}
                </ContextMenuItem>
              </ContextMenuContent>
            )}
          </ContextMenu>
        )}
        {agentActive ? (
          <>
            <span
              role="status"
              data-testid="browser-agent-active"
              className="flex-none whitespace-nowrap text-supporting text-text-muted"
            >
              {BROWSER_COPY.agentBrowsing}
            </span>
            {onStopAgent && (
              <ToolbarButton
                label={BROWSER_COPY.stopAgent}
                onClick={onStopAgent}
                icon={<Square className="h-3 w-3" aria-hidden="true" />}
              />
            )}
          </>
        ) : (
          <ToolbarButton
            label={sharedWithAgent ? BROWSER_COPY.stopSharing : BROWSER_COPY.share}
            pressed={sharedWithAgent}
            onClick={() => setSharedWithAgent((current) => !current)}
            icon={<Share2 className="h-3.5 w-3.5" aria-hidden="true" />}
          />
        )}
        <DropdownMenu
          open={menuOpen}
          onOpenChange={(open) => {
            setMenuOpen(open);
            if (open && !managed && browser?.bookmarks) {
              void browser.bookmarks().then(setBookmarks);
            }
          }}
        >
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label={BROWSER_COPY.more}
              title={BROWSER_COPY.more}
              data-testid="browser-more"
              className="br-preview-icon-button no-drag inline-flex h-6 w-6 flex-none items-center justify-center rounded-inner text-text-muted hover:bg-overlay-hover hover:text-text-default"
            >
              <MoreHorizontal className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-[200px] max-w-[320px]">
            <DropdownMenuItem onSelect={() => onOpenExternal(openTarget)}>
              {BROWSER_COPY.openInDefaultBrowser}
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() => void copyToClipboard(state?.url || url, slotRef.current)}
            >
              {BROWSER_COPY.copyLink}
            </DropdownMenuItem>
            {!managed && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => {
                    setFindOpen(true);
                    window.requestAnimationFrame(() => findRef.current?.focus());
                  }}
                >
                  {BROWSER_COPY.findInPage}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => void browser?.zoom?.(viewId, 'in')}>
                  {BROWSER_COPY.zoomIn}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => void browser?.zoom?.(viewId, 'out')}>
                  {BROWSER_COPY.zoomOut}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => void browser?.zoom?.(viewId, 'reset')}>
                  {BROWSER_COPY.actualSize(zoomPercent)}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => void browser?.setBookmark?.(viewId, !state?.bookmarked)}
                >
                  {state?.bookmarked ? BROWSER_COPY.removeBookmark : BROWSER_COPY.addBookmark}
                </DropdownMenuItem>
                {bookmarks.length > 0 && (
                  <DropdownMenuSub>
                    <DropdownMenuSubTrigger>{BROWSER_COPY.bookmarks}</DropdownMenuSubTrigger>
                    <DropdownMenuSubContent className="max-h-[60vh] w-64 overflow-y-auto">
                      {bookmarks
                        .filter((bookmark) => !bookmark.folder)
                        .slice(0, 50)
                        .map((bookmark) => (
                          <BookmarkItem
                            key={bookmark.url}
                            bookmark={bookmark}
                            onOpen={openAddress}
                          />
                        ))}
                      {/* An import's bookmarks, in its own folder (BB-22). */}
                      {importedBookmarkGroups(bookmarks).map(([label, items]) => (
                        <DropdownMenuSub key={label}>
                          <DropdownMenuSubTrigger>{label}</DropdownMenuSubTrigger>
                          <DropdownMenuSubContent className="max-h-[60vh] w-64 overflow-y-auto">
                            {items.slice(0, 50).map((bookmark) => (
                              <BookmarkItem
                                key={`${bookmark.folder}\u0000${bookmark.url}`}
                                bookmark={bookmark}
                                onOpen={openAddress}
                              />
                            ))}
                          </DropdownMenuSubContent>
                        </DropdownMenuSub>
                      ))}
                    </DropdownMenuSubContent>
                  </DropdownMenuSub>
                )}
              </>
            )}
            <DropdownMenuSeparator />
            {allowChromeImport && !managed && importStatus?.available && (
              <DropdownMenuItem onSelect={() => setImportOpen(true)}>
                {CHROME_IMPORT_COPY.open}
              </DropdownMenuItem>
            )}
            <DropdownMenuItem
              onSelect={() => {
                if (!browser) return;
                void browser.clearData(viewId).then((cleared) => {
                  if (cleared) control('reload');
                });
              }}
            >
              {BROWSER_COPY.clearSiteData}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {addressRefusal && (
        <BrowserLine
          testId="embedded-browser-address-refused"
          text={addressRefusal}
          onDismiss={() => setAddressRefusal(null)}
        />
      )}
      {state?.error && (
        <BrowserLine testId="embedded-browser-error" text={state.error}>
          {state.errorOffersExternal && (
            <LineAction onClick={() => onOpenExternal(openTarget)}>
              {BROWSER_COPY.openInDefaultBrowser}
            </LineAction>
          )}
        </BrowserLine>
      )}
      {notice && (
        <BrowserLine
          testId="embedded-browser-notice"
          text={notice.text}
          onDismiss={() => void browser?.dismissNotice?.(viewId)}
        >
          {notice.action && (
            <LineAction onClick={runNoticeAction}>{notice.action.label}</LineAction>
          )}
        </BrowserLine>
      )}
      {download && (
        <BrowserLine
          testId="embedded-browser-download"
          text={BROWSER_COPY.download(download)}
          onDismiss={
            download.state === 'progressing'
              ? undefined
              : () => void browser?.dismissNotice?.(viewId)
          }
        >
          {download.state === 'completed' && (
            <LineAction onClick={() => void browser?.showDownload?.(viewId)}>
              {window.electron?.platform === 'darwin'
                ? PREVIEW_COPY.revealInFinder
                : PREVIEW_COPY.showInFolder}
            </LineAction>
          )}
        </BrowserLine>
      )}

      {/*
        The hole. `overflow-hidden` and no scrolling ancestor: a native view
        cannot clip to a scroll container, so the page scrolls itself.
        `data-testid` is how tests assert the slot exists without a real view.
      */}
      <div
        ref={slotRef}
        data-testid="embedded-browser-slot"
        aria-label={state?.title || url}
        className={cn(
          'relative min-h-0 flex-1 overflow-hidden',
          (isSuspended || coveredBySelf) && 'bg-background-fill'
        )}
        // The native view paints over the slot, so the accent edge sits just outside it. An
        // inline style rather than a utility: nothing load-bearing may depend on a newly written
        // Tailwind class being generated (CLAUDE.md, the composer focus edge).
        style={
          agentActive
            ? { outline: '2px solid var(--border-accent)', outlineOffset: '1px' }
            : undefined
        }
      >
        {showImportOffer && (
          <ChromeImportOffer
            onImport={() => {
              setOfferAnswered(true);
              setImportOpen(true);
            }}
            onNotNow={() => setOfferAnswered(true)}
          />
        )}
        {snapshotDataUrl && (
          <img
            src={snapshotDataUrl}
            alt="Snapshot of the live page for region selection"
            className="absolute inset-0 h-full w-full object-fill"
          />
        )}
        {showingSuggestions && (
          // The page is hidden while this list is up (a native view would paint over it).
          <ul
            id={listboxId}
            role="listbox"
            aria-label={BROWSER_COPY.suggestions}
            className="absolute inset-x-0 top-0 max-h-full overflow-y-auto bg-background-default py-1"
          >
            {suggestions.map((suggestion, index) => (
              <li
                key={suggestion.url}
                id={`${listboxId}-${index}`}
                role="option"
                aria-selected={index === highlighted}
                // A mouse down would blur the address field before the click lands.
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => openAddress(suggestion.url)}
                className={cn(
                  'flex cursor-default flex-col px-3 py-1.5',
                  index === highlighted ? 'bg-background-fill-strong' : 'hover:bg-overlay-hover'
                )}
              >
                <span className="truncate text-secondary text-text-default">
                  {suggestion.title}
                </span>
                <span className="truncate text-supporting text-text-muted">{suggestion.url}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
      {allowChromeImport && (
        <ChromeImportDialog
          open={importOpen}
          onOpenChange={setImportOpen}
          // The page picks up logins it was just given, or loses ones just removed.
          onImported={() => control('reload')}
        />
      )}
    </div>
  );
}

export const BROWSER_COPY = {
  back: 'Back',
  forward: 'Forward',
  reload: 'Reload',
  stop: 'Stop',
  address: 'Address',
  suggestions: 'Suggestions from your bookmarks and history',
  local: 'Local',
  localHint: 'A page on this computer or your local network',
  share: 'Share with agent',
  agentBrowsing: 'Biorouter is browsing',
  stopAgent: 'Stop Biorouter',
  stopSharing: 'Stop sharing with agent',
  more: 'More browser actions',
  openInDefaultBrowser: 'Open in default browser',
  copyLink: 'Copy link',
  findInPage: 'Find in page',
  previousMatch: 'Previous match',
  nextMatch: 'Next match',
  closeFind: 'Close find',
  matches: (active: number, total: number) =>
    total === 0 ? 'No matches' : `${active} of ${total}`,
  zoomIn: 'Zoom in',
  zoomOut: 'Zoom out',
  actualSize: (percent: number) => (percent === 100 ? 'Actual size' : `Actual size (${percent}%)`),
  resetZoom: (percent: number) => `Zoom ${percent}%. Reset to actual size`,
  addBookmark: 'Add bookmark',
  removeBookmark: 'Remove bookmark',
  bookmarks: 'Bookmarks',
  clearSiteData: 'Clear site data',
  dismiss: 'Dismiss',
  download: (download: NonNullable<EmbeddedBrowserState['download']>) => {
    if (download.state === 'completed') return `Downloaded ${download.filename}`;
    if (download.state === 'cancelled') return `Download of ${download.filename} cancelled`;
    if (download.state === 'interrupted') return `Download of ${download.filename} failed`;
    if (download.totalBytes > 0) {
      const percent = Math.min(
        100,
        Math.floor((download.receivedBytes / download.totalBytes) * 100)
      );
      return `Downloading ${download.filename}, ${percent}%`;
    }
    return `Downloading ${download.filename}…`;
  },
} as const;

function BookmarkItem({
  bookmark,
  onOpen,
}: {
  bookmark: EmbeddedBrowserBookmark;
  onOpen: (url: string) => void;
}) {
  return (
    <DropdownMenuItem onSelect={() => onOpen(bookmark.url)} className="flex-col items-start gap-0">
      <span className="w-full min-w-0 truncate">{bookmark.title}</span>
      <span className="w-full min-w-0 truncate text-supporting text-text-muted">
        {bookmark.url}
      </span>
    </DropdownMenuItem>
  );
}

/** Imported bookmarks grouped by their import's folder ("Imported from Chrome (Work)"). */
function importedBookmarkGroups(
  bookmarks: EmbeddedBrowserBookmark[]
): Array<[string, EmbeddedBrowserBookmark[]]> {
  const groups = new Map<string, EmbeddedBrowserBookmark[]>();
  for (const bookmark of bookmarks) {
    if (!bookmark.folder) continue;
    const label = bookmark.folder.split(' / ')[0];
    groups.set(label, [...(groups.get(label) ?? []), bookmark]);
  }
  return [...groups];
}

/** One quiet line under the bar: an error, a note or a download, with its one action. */
function BrowserLine({
  testId,
  text,
  children,
  onDismiss,
}: {
  testId: string;
  text: string;
  children?: React.ReactNode;
  onDismiss?: () => void;
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-none items-center gap-2 bg-background-fill px-3 py-1 text-supporting text-text-muted"
    >
      <p className="min-w-0 flex-1 truncate" title={text}>
        {text}
      </p>
      {children}
      {onDismiss && (
        <button
          type="button"
          aria-label={BROWSER_COPY.dismiss}
          title={BROWSER_COPY.dismiss}
          onClick={onDismiss}
          className="inline-flex h-5 w-5 flex-none items-center justify-center rounded-inner text-text-subtle hover:bg-overlay-hover hover:text-text-default"
        >
          <X className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

function LineAction({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <Button type="button" variant="ghost" size="xs" className="flex-none" onClick={onClick}>
      {children}
    </Button>
  );
}

function ToolbarButton({
  label,
  icon,
  onClick,
  disabled,
  pressed,
}: {
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  /** A toggle (Share with agent) says it is on, by its pressed fill and to a screen reader. */
  pressed?: boolean;
}) {
  return (
    <PreviewIconButton
      compact
      label={label}
      disabled={disabled}
      onClick={onClick}
      aria-pressed={pressed}
      className={cn(pressed && 'br-preview-icon-button--pressed')}
    >
      {icon}
    </PreviewIconButton>
  );
}
