import { useCallback, useEffect, useRef, useState } from 'react';
import { listSidebarSessions, type SessionSummary } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { subscribeSessionNameChanges } from '../../utils/sessionNameSync';
import { subscribeSessionListChanges, subscribeSessionRemoved } from '../../utils/sessionListCache';

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
  const nextOffsetRef = useRef(0);
  const hasMoreRef = useRef(true);
  const hasLoadedRef = useRef(false);
  const loadingRef = useRef(false);

  const loadPage = useCallback(async (reset: boolean) => {
    if (loadingRef.current || (!reset && !hasMoreRef.current)) return;

    const offset = reset ? 0 : nextOffsetRef.current;
    loadingRef.current = true;
    setIsLoading(true);

    try {
      // With the user's proof: without it the daemon pages a view with every
      // private chat omitted (issue #56, QA 2026-09-10 M1).
      const response = await listSidebarSessions<true>({
        query: { limit: SIDEBAR_SESSION_PAGE_SIZE, offset },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const page = response.data;
      const mergedSessions = appendSessionPage(sessionsRef.current, page.sessions);
      const pageHasMore =
        reset && hasLoadedRef.current ? hasMoreRef.current || page.has_more : page.has_more;

      sessionsRef.current = mergedSessions;
      setSessions(mergedSessions);
      nextOffsetRef.current = reset
        ? mergedSessions.length
        : (page.next_offset ?? offset + page.sessions.length);
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

    // M11. A removal is the one membership change the nudge above cannot carry:
    // `appendSessionPage` merges a re-read INTO what we hold and has no removal
    // branch, so a deleted chat survived every refresh and cleared only on a
    // renderer reload. Splice it out by id instead — exact, and with no risk of
    // evicting a live chat that merely fell out of the first page.
    //
    // `nextOffsetRef` moves down with it: the server's list lost the same row,
    // so leaving the offset alone would make the next `loadMore` skip one.
    const unsubscribeRemoved = subscribeSessionRemoved((sessionId) => {
      const remaining = sessionsRef.current.filter((session) => session.id !== sessionId);
      if (remaining.length === sessionsRef.current.length) return;
      sessionsRef.current = remaining;
      setSessions(remaining);
      nextOffsetRef.current = Math.max(0, nextOffsetRef.current - 1);
    });

    window.addEventListener('session-created', scheduleRefresh);
    window.addEventListener('message-stream-finished', scheduleRefresh);

    return () => {
      unsubscribeNames();
      unsubscribeList();
      unsubscribeRemoved();
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
