import { useState, useEffect } from 'react';
import { getCallableToolCount } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { CATALOG_CHANGED_EVENT } from '../../utils/catalogSubscription';
import {
  SESSION_TOOLS_CHANGED_EVENT,
  type SessionToolEventDetail,
} from '../../utils/sessionToolEvents';

// TODO(Douwe): return this as part of the start agent request
/**
 * The tools callable by the session's bound model.
 *
 * `agentReady` is load-bearing, not decorative. Tools come from extensions, and
 * with progressive conversation loading the transcript paints ~4.6s before the
 * extensions are up — so a fetch keyed on `sessionId` alone reliably reads zero
 * tools and, with no other dependency to re-trigger it, caches that zero for the
 * life of the chat. The "too many tools" alert would then simply never fire.
 * Refetch when readiness, the live catalog, or session-local tool state changes;
 * discard superseded responses so a slow pre-install query cannot overwrite the
 * current count.
 */
export const useToolCount = (sessionId: string, agentReady: boolean = true) => {
  const [toolCount, setToolCount] = useState<number | null>(null);

  useEffect(() => {
    setToolCount(null);
    // Nothing to count until the extensions that provide the tools exist.
    if (!agentReady || !sessionId) return;

    let cancelled = false;
    let revision = 0;
    let controller: AbortController | undefined;
    const fetchTools = async () => {
      const requestRevision = ++revision;
      controller?.abort();
      controller = new AbortController();
      try {
        // With the user's proof: a private chat refuses this read to a caller
        // without it (issue #56, QA 2026-09-10 M2).
        const response = await getCallableToolCount({
          query: { session_id: sessionId },
          headers: await userActionHeaders(),
          signal: controller.signal,
        });
        if (cancelled || requestRevision !== revision) return;
        if (!response.error && response.data) setToolCount(response.data.count);
      } catch (err) {
        if (cancelled || requestRevision !== revision) return;
        console.error('Error fetching tools:', err);
      }
    };

    fetchTools();
    const handleSessionToolsChanged = (event: Event) => {
      const detail = (event as CustomEvent<SessionToolEventDetail>).detail;
      if (detail?.sessionId === sessionId) void fetchTools();
    };
    window.addEventListener(CATALOG_CHANGED_EVENT, fetchTools);
    window.addEventListener('message-stream-finished', fetchTools);
    window.addEventListener(SESSION_TOOLS_CHANGED_EVENT, handleSessionToolsChanged);
    // Closing the tab while extensions are still starting must not land a
    // setState on an unmounted component.
    return () => {
      cancelled = true;
      controller?.abort();
      window.removeEventListener(CATALOG_CHANGED_EVENT, fetchTools);
      window.removeEventListener('message-stream-finished', fetchTools);
      window.removeEventListener(SESSION_TOOLS_CHANGED_EVENT, handleSessionToolsChanged);
    };
  }, [sessionId, agentReady]);

  return toolCount;
};
