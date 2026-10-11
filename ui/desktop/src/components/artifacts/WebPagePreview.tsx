import { useCallback, useEffect, useId, useRef, useState } from 'react';
import {
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  RefreshCw,
  Share2,
  Trash2,
  X,
} from '../icons/app-icons';
import { cn } from '../../utils';
import { Button } from '../ui/button';
import { PREVIEW_COPY } from './copy';
import { PreviewIconButton } from './PreviewBar';

export type EmbeddedBrowserState = {
  url: string;
  title: string;
  managedApp?: boolean;
  sourceRevision: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
  error: string | null;
};

export type LiveBrowserShare = {
  viewId: string;
  state: EmbeddedBrowserState;
};

/**
 * A live, interactive web page in the artifact panel.
 *
 * **This component renders no page content.** The page lives in a
 * `WebContentsView` owned by the main process — a real top-level browsing
 * context, so clicking, typing, scrolling, JavaScript and ordinary site cookies
 * behave as they do in Chrome, and sites that refuse to be framed (about half
 * the ones this audience opens) load normally. Sign-in navigations leave for
 * the system browser because this isolated partition cannot share their callback
 * state safely or reliably.
 *
 * What this component owns is the *hole* the native view is painted into, and
 * the chrome around it. The native view has no shared z-index with the DOM, so
 * two rules follow and both are load-bearing:
 *
 * 1. The slot must not live inside a scrolling container — the view cannot clip
 *    to one. The page scrolls itself instead.
 * 2. Anything that needs to sit on top (a modal, a dropdown, the resize shield)
 *    requires hiding the view, which is what `isSuspended` is for.
 */
export default function WebPagePreview({
  url,
  requireManagedApp = false,
  isSuspended = false,
  snapshotDataUrl,
  onOpenExternal,
  onViewIdChange,
  onAgentShareChange,
  refreshRevision = 0,
}: {
  url: string;
  /** Fail closed if this URL is no longer owned by the active BioRouter daemon. */
  requireManagedApp?: boolean;
  /** Hide the native view because something must paint above it. */
  isSuspended?: boolean;
  /** A compositor snapshot shown while the native view is hidden for annotation. */
  snapshotDataUrl?: string | null;
  onOpenExternal: (url: string) => void;
  onViewIdChange?: (viewId: string | null) => void;
  onAgentShareChange?: (share: LiveBrowserShare | null) => void;
  refreshRevision?: number;
}) {
  const slotRef = useRef<HTMLDivElement | null>(null);
  const reactId = useId();
  const viewIdRef = useRef(`embedded-browser-${reactId}`);
  const [state, setState] = useState<EmbeddedBrowserState | null>(null);
  const [addressDraft, setAddressDraft] = useState(url);
  const [isEditingAddress, setIsEditingAddress] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const [sharedWithAgent, setSharedWithAgent] = useState(false);
  const [updateReady, setUpdateReady] = useState(false);
  const refreshRef = useRef({ url, consumed: refreshRevision, generation: 0 });
  const latestRevision = useRef(refreshRevision);
  latestRevision.current = refreshRevision;

  const viewId = viewIdRef.current;
  const browser = window.electron?.embeddedBrowser;

  // Create once per URL identity, and tear down on unmount. The view is a child
  // of the *window*, not of this tree, so nothing else will clean it up.
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
      url,
      consumed: latestRevision.current,
      generation: refreshRef.current.generation + 1,
    };

    const stopListening = browser.onState((payload) => {
      if (payload.viewId === viewId && !disposed) setState(payload.state);
    });

    const creation = requireManagedApp
      ? browser.create(viewId, url, true)
      : browser.create(viewId, url);
    void creation.then((initial) => {
      if (disposed) return;
      if (!initial) {
        setUnavailable(true);
        return;
      }
      setState(initial);
      onViewIdChange?.(viewId);
    });

    return () => {
      disposed = true;
      refreshRef.current.generation += 1;
      stopListening();
      onViewIdChange?.(null);
      onAgentShareChange?.(null);
      void browser.destroy(viewId);
    };
  }, [browser, onAgentShareChange, onViewIdChange, requireManagedApp, url, viewId]);

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
    void browser.setVisible(viewId, !isSuspended);
    return () => {
      void browser.setVisible(viewId, false);
    };
  }, [browser, isSuspended, unavailable, viewId]);

  useEffect(() => {
    if (!isEditingAddress && state?.url) setAddressDraft(state.url);
  }, [isEditingAddress, state?.url]);

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

  const submitAddress = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();
      if (!browser) return;
      const trimmed = addressDraft.trim();
      if (!trimmed) return;
      // A bare host is what people type. Default to https rather than refusing.
      const candidate = /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
      void browser.navigate(viewId, candidate);
      setIsEditingAddress(false);
    },
    [addressDraft, browser, viewId]
  );

  if (unavailable) {
    return (
      <div className="br-preview-state" data-preview-native="">
        <p className="br-preview-state__line text-body text-text-muted">
          {PREVIEW_COPY.pageUnavailable}
        </p>
        <Button type="button" variant="secondary" size="sm" onClick={() => onOpenExternal(url)}>
          <ExternalLink className="h-4 w-4" aria-hidden="true" />
          {PREVIEW_COPY.openInBrowserShort}
        </Button>
      </div>
    );
  }

  const busy = state?.isLoading ?? true;

  return (
    // `data-preview-native`: a native view cannot move with the DOM, so the
    // panel's switch fades this layer and never slides it (preview-panel.css).
    <div className="flex h-full min-h-0 flex-col" data-preview-native="">
      {/* The page's own toolbar stands in for the panel's 32px bar: same height,
          same hairline, same 24px icon controls. */}
      <div className="br-preview-bar br-preview-bar--browser">
        <ToolbarButton
          label="Back"
          disabled={!state?.canGoBack}
          onClick={() => control('back')}
          icon={<ChevronLeft className="h-3.5 w-3.5" aria-hidden="true" />}
        />
        <ToolbarButton
          label="Forward"
          disabled={!state?.canGoForward}
          onClick={() => control('forward')}
          icon={<ChevronRight className="h-3.5 w-3.5" aria-hidden="true" />}
        />
        <ToolbarButton
          label={busy ? 'Stop' : 'Reload'}
          onClick={() => control(busy ? 'stop' : 'reload')}
          icon={
            busy ? (
              <X className="h-3.5 w-3.5" aria-hidden="true" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
            )
          }
        />
        <form onSubmit={submitAddress} className="min-w-0 flex-1">
          <input
            aria-label="Address"
            value={addressDraft}
            spellCheck={false}
            onFocus={() => setIsEditingAddress(true)}
            onChange={(event) => {
              setAddressDraft(event.target.value);
              setIsEditingAddress(true);
            }}
            onBlur={() => setIsEditingAddress(false)}
            // The app's filled field (R-27): a fill at rest, the strong step under
            // the pointer, the accent edge only while it is being typed in.
            className="br-field h-6 w-full min-w-0 truncate rounded-element border px-2 text-secondary text-text-default transition-[color,background-color,border-color] placeholder:text-text-muted"
          />
        </form>
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
        <ToolbarButton
          label={sharedWithAgent ? 'Stop sharing with agent' : 'Share with agent'}
          pressed={sharedWithAgent}
          onClick={() => setSharedWithAgent((current) => !current)}
          icon={<Share2 className="h-3.5 w-3.5" aria-hidden="true" />}
        />
        <ToolbarButton
          label="Clear site data"
          onClick={() => {
            if (!browser) return;
            void browser.clearData(viewId).then((cleared) => {
              if (cleared) control('reload');
            });
          }}
          icon={<Trash2 className="h-3.5 w-3.5" aria-hidden="true" />}
        />
        <ToolbarButton
          label="Open in browser"
          onClick={() => onOpenExternal(state?.url || url)}
          icon={<ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />}
        />
      </div>

      {state?.error && (
        <p
          data-testid="embedded-browser-error"
          className="flex-none bg-background-fill px-3 py-1.5 text-supporting text-text-muted"
        >
          {state.error}
        </p>
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
          isSuspended && 'bg-background-fill'
        )}
      >
        {snapshotDataUrl && (
          <img
            src={snapshotDataUrl}
            alt="Snapshot of the live page for region selection"
            className="absolute inset-0 h-full w-full object-fill"
          />
        )}
      </div>
    </div>
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
