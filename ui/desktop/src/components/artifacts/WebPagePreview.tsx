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
import { previewOpenTarget } from '../../utils/openInBrowser';

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
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center text-body text-text-muted">
        <p className="max-w-xs leading-relaxed">
          This page can’t be opened here. Open it in your browser instead.
        </p>
        <button
          type="button"
          onClick={() => onOpenExternal(url)}
          className="inline-flex items-center gap-1.5 rounded-element border border-border-strong px-3 py-1.5 text-label text-text-default transition-colors hover:bg-overlay-hover"
        >
          <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
          Open in browser
        </button>
      </div>
    );
  }

  const busy = state?.isLoading ?? true;

  return (
    <div className="flex h-full min-h-0 flex-col bg-background-default">
      <div className="flex h-tab flex-none items-center gap-1 border-b border-border-subtle px-1.5">
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
            className="w-full min-w-0 truncate rounded-element bg-background-muted px-2 py-1 text-supporting text-text-default outline-none focus:bg-background-medium"
          />
        </form>
        {updateReady && (
          <button
            type="button"
            disabled={busy || isSuspended}
            onClick={() => control('reload')}
            className="shrink-0 rounded-element px-2 py-1 text-supporting text-text-muted hover:bg-overlay-hover"
          >
            Update ready
          </button>
        )}
        <ToolbarButton
          label={sharedWithAgent ? 'Stop sharing with agent' : 'Share with agent'}
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
          // Not `state.url` as it stands: the page sets it (`previewOpenTarget`).
          onClick={() => onOpenExternal(previewOpenTarget(url, state))}
          icon={<ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />}
        />
      </div>

      {state?.error && (
        <p
          data-testid="embedded-browser-error"
          className="flex-none border-b border-border-subtle bg-background-muted px-3 py-1.5 text-supporting text-text-muted"
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
          isSuspended && 'bg-background-muted'
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
}: {
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className="flex h-6 w-6 flex-none items-center justify-center rounded-element text-text-muted transition-colors hover:bg-overlay-hover hover:text-text-default disabled:pointer-events-none disabled:opacity-35"
    >
      {icon}
    </button>
  );
}
