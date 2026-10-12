import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import { getSession, type Session } from '../../api';
import { getCachedSessionList, subscribeSessionList } from '../../utils/sessionListCache';
import { userActionHeaders } from '../../utils/userAction';

/** The listing's row for one chat, if the listing has been read and holds it. */
export function listedChatRow(
  rows: readonly Session[] | null,
  sessionId: string
): Session | undefined {
  return rows?.find((row) => row.id === sessionId);
}

const noSubscription = () => () => {};
const noRows = () => null;

/**
 * What the app already knows about a chat while its transcript loads.
 *
 * A chat that is opened for the first time in this window (a tab restored after
 * a reload, a chat opened from History or by its link) loads its transcript
 * first, and for that whole time BaseChat holds no `session`: up to 3.7 s
 * measured on a long chat (vision V2-R5-6). Without a row, the chat looked new:
 * no summary rail, so the column re-centred 140px to the right and came back
 * when the transcript landed; the composer said "Ask Biorouter anything…"; and
 * the model chip named the model for NEW chats, padlock and all, on a public
 * chat bound to another model.
 *
 * Two sources, both reads the app already makes:
 *
 * - the session listing (`utils/sessionListCache`), which History and the tab
 *   strip keep warm, so most opens know the row in the first frame;
 * - the row itself, `GET /sessions/{id}?metadata_only=true`, with the person's
 *   proof, for the open the listing cannot answer yet (the tab that was active
 *   when the window reloaded). It goes out in the same moment as the composer's
 *   own read of the same row, so `utils/sessionReadCoalescing` sends one
 *   request for both. The row wins over the listing when it lands: it is the
 *   fresher of the two.
 *
 * `undefined` whenever the chat is not loading: from then on BaseChat has the
 * session itself, and nothing here may outlive the load.
 */
export function useLoadingChatRow(sessionId: string, loading: boolean): Session | undefined {
  const wanted = loading && !!sessionId;
  // Subscribed only while loading, so a rename elsewhere does not re-render a
  // chat that has its own row.
  const subscribe = useCallback(
    (onChange: () => void) => (wanted ? subscribeSessionList(onChange) : noSubscription()),
    [wanted]
  );
  const rows = useSyncExternalStore(
    subscribe,
    wanted ? getCachedSessionList : noRows,
    wanted ? getCachedSessionList : noRows
  );

  const [read, setRead] = useState<Session | undefined>(undefined);
  useEffect(() => {
    if (!wanted) return;
    const controller = new AbortController();
    void (async () => {
      try {
        const headers = await userActionHeaders();
        if (controller.signal.aborted) return;
        const response = await getSession({
          path: { session_id: sessionId },
          // The row, not the transcript: the transcript is what is loading.
          query: { metadata_only: true },
          // Issue #56: reading a private chat needs the proof of the person.
          headers,
          signal: controller.signal,
        });
        if (controller.signal.aborted) return;
        if (response.data?.id === sessionId) setRead(response.data);
      } catch {
        // The load reports its own failure; this only ever adds what it knows.
      }
    })();
    return () => controller.abort();
  }, [wanted, sessionId]);

  if (!wanted) return undefined;
  if (read?.id === sessionId) return read;
  return listedChatRow(rows, sessionId);
}
