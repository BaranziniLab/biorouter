import { useCallback, useEffect, useRef, useState } from 'react';
import { listSidebarSessions, type SessionSummary } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { subscribeSessionNameChanges } from '../../utils/sessionNameSync';
import { subscribeSessionListChanges, subscribeSessionRemoved } from '../../utils/sessionListCache';
import {
  settleRowsReadDuringFetch,
  subscribeSessionRowChanges,
  type SessionRowFacts,
} from '../../utils/sessionRowSync';

export const SIDEBAR_SESSION_PAGE_SIZE = 10;
/** The server's page maximum (`routes/session.rs` clamps `limit` to 1..=50). */
export const SIDEBAR_SESSION_FULL_PAGE_SIZE = 50;
/** How many chats a non-default view reads at most: 40 requests at the full page size. */
export const SIDEBAR_SESSION_LOAD_ALL_CAP = 2_000;

export interface SidebarSessionsOptions {
  /**
   * Read every chat, not just the pages scrolled into view. The server pages in
   * `updated_at` order only, so any view that sorts by name or creation, or
   * groups by folder, must hold the whole list before it arranges it: over the
   * first 10 rows it would be wrong in a way nobody could see. Pages are read one
   * after another at the server's maximum, up to {@link SIDEBAR_SESSION_LOAD_ALL_CAP}.
   *
   * ⚠ The pages are read into a buffer and shown ONCE, when the last has landed
   * (vision V1-H2). Published page by page, a name-sorted list reshuffled its top
   * rows with every arrival, under the pointer, for as long as the read took, and
   * each arrival re-rendered every row already held, which is most of why it took
   * a minute and a half for 2,000 chats.
   */
  loadAll?: boolean;
}

export interface SidebarSessionsState {
  sessions: SessionSummary[];
  hasMore: boolean;
  isLoading: boolean;
  /** True while a `loadAll` read is still paging through the list. */
  isLoadingAll: boolean;
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

export default function useSidebarSessions(
  options: SidebarSessionsOptions = {}
): SidebarSessionsState {
  const loadAll = options.loadAll ?? false;
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [hasMore, setHasMore] = useState(true);
  const [isLoading, setIsLoading] = useState(true);
  const sessionsRef = useRef<SessionSummary[]>([]);
  const nextCursorRef = useRef<string | null>(null);
  const hasMoreRef = useRef(true);
  const hasLoadedRef = useRef(false);
  const loadingRef = useRef(false);
  const mountedRef = useRef(true);
  // `loadAll`'s reader. `pendingAllRef` holds the rows it has read and not yet
  // shown (null when no read is running), and the subscriptions below patch it
  // exactly as they patch the shown list, so a rename, a removal or a privacy
  // change that lands mid-read is not undone when the buffer is shown. A head
  // refresh asked for mid-read cannot run beside it (one request at a time) and
  // must not be lost, so it waits in `refreshQueuedRef` and runs after.
  const loadAllRef = useRef(loadAll);
  loadAllRef.current = loadAll;
  const drainingRef = useRef(false);
  const pendingAllRef = useRef<SessionSummary[] | null>(null);
  const refreshQueuedRef = useRef(false);
  // Row reads delivered while a page request was in flight, settled against
  // that page's answer. See `settleRowsReadDuringFetch`: without it, a head
  // refresh issued before a turn raised a chat and answered after the raise was
  // patched in would draw the chat public again.
  const rowsReadDuringLoadRef = useRef(new Map<string, SessionRowFacts>());
  // Read by `loadPage` at request time, so a view change takes effect on the
  // next page without re-creating the loader (and its subscriptions).
  const pageSizeRef = useRef(loadAll ? SIDEBAR_SESSION_FULL_PAGE_SIZE : SIDEBAR_SESSION_PAGE_SIZE);
  pageSizeRef.current = loadAll ? SIDEBAR_SESSION_FULL_PAGE_SIZE : SIDEBAR_SESSION_PAGE_SIZE;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const loadPage = useCallback(async (reset: boolean) => {
    if (loadingRef.current) {
      if (reset && drainingRef.current) refreshQueuedRef.current = true;
      return;
    }
    if (!reset && !hasMoreRef.current) return;

    const cursor = reset ? null : nextCursorRef.current;
    loadingRef.current = true;
    rowsReadDuringLoadRef.current.clear();
    setIsLoading(true);

    try {
      // With the user's proof: without it the daemon pages a view with every
      // private chat omitted (issue #56, QA 2026-09-10 M1). Every page carries
      // it, the `loadAll` pages included: a page without it would read as the
      // person's private chats vanishing when they changed the view.
      const response = await listSidebarSessions<true>({
        query: { limit: pageSizeRef.current, ...(cursor ? { cursor } : {}) },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const page = response.data;
      const mergedSessions = appendSessionPage(
        sessionsRef.current,
        settleRowsReadDuringFetch(page.sessions, rowsReadDuringLoadRef.current, false)
      );
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
      const rename = (rows: SessionSummary[]) =>
        rows.map((session) =>
          session.id === sessionId ? { ...session, name, user_set_name: userSetName } : session
        );
      if (pendingAllRef.current) pendingAllRef.current = rename(pendingAllRef.current);
      const renamedSessions = rename(sessionsRef.current);
      sessionsRef.current = renamedSessions;
      setSessions(renamedSessions);
    });

    // Membership changes — a session created, DIVERGED, deleted or imported, in
    // THIS window or any sibling. Diverge creates a session purely over HTTP and
    // dispatches no `session-created` window event (and those are per-renderer
    // anyway), so before this the branch never entered the sidebar list until an
    // unrelated turn happened to finish. Now every list change re-reads.
    const unsubscribeList = subscribeSessionListChanges(scheduleRefresh);

    // M11. A removal is the one membership change the nudge above cannot carry:
    // `appendSessionPage` merges a re-read INTO what we hold and has no removal
    // branch, so a deleted chat survived every refresh and cleared only on a
    // renderer reload. Splice it out by id instead — exact, and with no risk of
    // evicting a live chat that merely fell out of the first page.
    //
    // ⚠ **Nothing is adjusted alongside it, and that is the keyset's doing.**
    // This handler arrived while the page resumed from an OFFSET, and it had to
    // decrement that offset by one: the server's list had lost the same row, so
    // a position left alone would make the next `loadMore` skip a chat. A
    // cursor names the sort key of the last row a page RETURNED, so it is a
    // boundary compared against values, not a count of rows — the row it names
    // does not have to exist for the comparison to put the next page in the
    // right place, including when the deleted row is that very one.
    const unsubscribeRemoved = subscribeSessionRemoved((sessionId) => {
      if (pendingAllRef.current) {
        pendingAllRef.current = pendingAllRef.current.filter((session) => session.id !== sessionId);
      }
      const remaining = sessionsRef.current.filter((session) => session.id !== sessionId);
      if (remaining.length === sessionsRef.current.length) return;
      sessionsRef.current = remaining;
      setSessions(remaining);
    });

    // A chat's classification changed in place — a declassification, or a raise
    // a chat store announced, here or in another window (`sessionRowSync`).
    // Patched by id, for the same reason the removal above is: `loadPage(true)`
    // re-reads only the HEAD of the keyset, and a declassified chat is usually
    // months old and nowhere near it. Until 2026-09-13 the daemon stamped
    // `updated_at` on a declassification, so the next head refresh happened to
    // carry the chat (moved to the top, which was the bug); with the chat left
    // where it belongs, only this reaches its row.
    const unsubscribeRows = subscribeSessionRowChanges((facts) => {
      const { sessionId, privacy_tier } = facts;
      if (loadingRef.current) rowsReadDuringLoadRef.current.set(sessionId, facts);
      if (pendingAllRef.current) {
        pendingAllRef.current = pendingAllRef.current.map((session) =>
          session.id === sessionId && session.privacy_tier !== privacy_tier
            ? { ...session, privacy_tier }
            : session
        );
      }
      let changed = false;
      const next = sessionsRef.current.map((session) => {
        if (session.id !== sessionId || session.privacy_tier === privacy_tier) return session;
        changed = true;
        return { ...session, privacy_tier };
      });
      if (!changed) return;
      sessionsRef.current = next;
      setSessions(next);
    });

    window.addEventListener('session-created', scheduleRefresh);
    window.addEventListener('message-stream-finished', scheduleRefresh);

    return () => {
      unsubscribeNames();
      unsubscribeList();
      unsubscribeRemoved();
      unsubscribeRows();
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
      window.removeEventListener('session-created', scheduleRefresh);
      window.removeEventListener('message-stream-finished', scheduleRefresh);
    };
  }, [loadPage]);

  const loadMore = useCallback(() => {
    void loadPage(false);
  }, [loadPage]);

  // `loadAll`: one page after another until the server has no more or the cap
  // is reached, read into a buffer and shown once (see `loadAll`). Sequential by
  // construction: a page's cursor names the last row of the page before it. A
  // page never asks for more rows than the cap leaves room for, so the read
  // stops at the cap rather than a page past it (V1-L9).
  const drainAllPages = useCallback(async () => {
    if (loadingRef.current || !hasMoreRef.current) return;
    loadingRef.current = true;
    drainingRef.current = true;
    rowsReadDuringLoadRef.current.clear();
    pendingAllRef.current = [];
    setIsLoading(true);

    let cursor = nextCursorRef.current;
    let more: boolean = hasMoreRef.current;
    try {
      while (more && loadAllRef.current && mountedRef.current) {
        const read = sessionsRef.current.length + (pendingAllRef.current?.length ?? 0);
        const room = SIDEBAR_SESSION_LOAD_ALL_CAP - read;
        if (room <= 0) break;
        // With the user's proof on every page, as `loadPage` sends it.
        const response = await listSidebarSessions<true>({
          query: {
            limit: Math.min(SIDEBAR_SESSION_FULL_PAGE_SIZE, room),
            ...(cursor ? { cursor } : {}),
          },
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        const page = response.data;
        pendingAllRef.current = appendSessionPage(
          pendingAllRef.current ?? [],
          settleRowsReadDuringFetch(page.sessions, rowsReadDuringLoadRef.current, false)
        );
        cursor = page.next_cursor ?? null;
        more = page.has_more;
      }
    } catch {
      // As `loadPage` does: a failed page ends the list where it is.
      more = false;
    } finally {
      const fetched = pendingAllRef.current ?? [];
      pendingAllRef.current = null;
      loadingRef.current = false;
      drainingRef.current = false;
      if (mountedRef.current) {
        const merged = appendSessionPage(sessionsRef.current, fetched);
        sessionsRef.current = merged;
        nextCursorRef.current = cursor;
        hasMoreRef.current = more;
        setSessions(merged);
        setHasMore(more);
        setIsLoading(false);
        if (refreshQueuedRef.current) {
          refreshQueuedRef.current = false;
          void loadPage(true);
        }
      }
    }
  }, [loadPage]);

  const wantsMoreForAll = loadAll && hasMore && sessions.length < SIDEBAR_SESSION_LOAD_ALL_CAP;
  useEffect(() => {
    if (!wantsMoreForAll || isLoading) return;
    void drainAllPages();
  }, [wantsMoreForAll, isLoading, drainAllPages]);

  return {
    sessions,
    hasMore,
    isLoading,
    isLoadingAll: wantsMoreForAll,
    loadMore,
  };
}
