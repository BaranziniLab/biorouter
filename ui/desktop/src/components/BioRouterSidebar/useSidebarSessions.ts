import { useCallback, useEffect, useRef, useState } from 'react';
import { listSidebarSessions, type SessionSummary } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { subscribeSessionNameChanges } from '../../utils/sessionNameSync';
import { subscribeSessionListChanges } from '../../utils/sessionListCache';

export const SIDEBAR_SESSION_PAGE_SIZE = 10;

export interface SidebarSessionsState {
  sessions: SessionSummary[];
  hasMore: boolean;
  isLoading: boolean;
  loadMore: () => void;
}

export function appendSessionPage(
  current: SessionSummary[],
  incoming: SessionSummary[]
): SessionSummary[] {
  const merged = [...current];
  const positions = new Map(merged.map((session, index) => [session.id, index]));

  for (const session of incoming) {
    const existingIndex = positions.get(session.id);
    if (existingIndex === undefined) {
      positions.set(session.id, merged.length);
      merged.push(session);
    } else {
      merged[existingIndex] = session;
    }
  }

  return merged;
}

export default function useSidebarSessions(): SidebarSessionsState {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [hasMore, setHasMore] = useState(true);
  const [isLoading, setIsLoading] = useState(true);
  const sessionsRef = useRef<SessionSummary[]>([]);
  const nextCursorRef = useRef<string | null>(null);
  const hasMoreRef = useRef(true);
  const hasLoadedRef = useRef(false);
  const loadingRef = useRef(false);

  const loadPage = useCallback(async (reset: boolean) => {
    if (loadingRef.current || (!reset && !hasMoreRef.current)) return;

    const cursor = reset ? null : nextCursorRef.current;
    loadingRef.current = true;
    setIsLoading(true);

    try {
      // With the user's proof: without it the daemon pages a view with every
      // private chat omitted (issue #56, QA 2026-09-10 M1).
      const response = await listSidebarSessions<true>({
        query: { limit: SIDEBAR_SESSION_PAGE_SIZE, ...(cursor ? { cursor } : {}) },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const page = response.data;
      const mergedSessions = appendSessionPage(sessionsRef.current, page.sessions);
      const pageHasMore =
        reset && hasLoadedRef.current ? hasMoreRef.current || page.has_more : page.has_more;

      sessionsRef.current = mergedSessions;
      setSessions(mergedSessions);
      // `next_cursor` is opaque and names the last row of the page that
      // returned it, so — unlike the offset this replaced — it cannot be
      // recomputed from the list we hold. A refresh re-reads the HEAD of the
      // list; the tail we already paged through is still held, and the cursor we
      // already have still points just past it. So a reset keeps it, and only a
      // list that has none (first load, or one that had reached the end) adopts
      // the one this page carries.
      nextCursorRef.current = reset
        ? (nextCursorRef.current ?? page.next_cursor ?? null)
        : (page.next_cursor ?? null);
      hasMoreRef.current = pageHasMore;
      hasLoadedRef.current = true;
      setHasMore(pageHasMore);
    } catch {
      hasMoreRef.current = false;
      setHasMore(false);
    } finally {
      loadingRef.current = false;
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadPage(true);

    let refreshTimer: number | undefined;
    const scheduleRefresh = () => {
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
      refreshTimer = window.setTimeout(() => {
        refreshTimer = undefined;
        void loadPage(true);
      }, 250);
    };

    const unsubscribeNames = subscribeSessionNameChanges(({ sessionId, name, userSetName }) => {
      const renamedSessions = sessionsRef.current.map((session) =>
        session.id === sessionId ? { ...session, name, user_set_name: userSetName } : session
      );
      sessionsRef.current = renamedSessions;
      setSessions(renamedSessions);
    });

    // Membership changes — a session created, DIVERGED, deleted or imported, in
    // THIS window or any sibling. Diverge creates a session purely over HTTP and
    // dispatches no `session-created` window event (and those are per-renderer
    // anyway), so before this the branch never entered the Recents list until an
    // unrelated turn happened to finish. Now every list change re-reads.
    const unsubscribeList = subscribeSessionListChanges(scheduleRefresh);

    window.addEventListener('session-created', scheduleRefresh);
    window.addEventListener('message-stream-finished', scheduleRefresh);

    return () => {
      unsubscribeNames();
      unsubscribeList();
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
      window.removeEventListener('session-created', scheduleRefresh);
      window.removeEventListener('message-stream-finished', scheduleRefresh);
    };
  }, [loadPage]);

  const loadMore = useCallback(() => {
    void loadPage(false);
  }, [loadPage]);

  return {
    sessions,
    hasMore,
    isLoading,
    loadMore,
  };
}
