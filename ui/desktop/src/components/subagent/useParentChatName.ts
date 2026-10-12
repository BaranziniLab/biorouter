/**
 * The parent chat's display name, for the subagent line's "Subagent of {parent}"
 * (spec 3.6; principle 10, names not IDs; vision V2-R2-2).
 *
 * The spawn-context record names the parent by id only
 * (`persist_spawn_context` writes `from_session_name: None`), and a subagent's
 * own row carries `parent_session_id` and nothing more, so the name has to come
 * from the parent's row:
 *
 * 1. the list the sidebar and History already hold (`sessionListCache`), when
 *    the parent is in it, which costs nothing;
 * 2. otherwise one `GET /sessions/{id}?metadata_only=true`, the row without the
 *    transcript, carrying the proof-of-user, because the reach gate refuses a
 *    private chat's row without it (a subagent shares its parent's tier);
 * 3. and a rename of the parent anywhere in the app (`sessionNameSync`'s
 *    broadcast) replaces it while the line is up.
 *
 * A read is a read: it binds nothing and moves nothing (R-30). Every answer is
 * tagged with the id it was read for, so a tab rebound to another subagent can
 * never show the previous parent's name.
 */
import { useEffect, useState } from 'react';
import { getSession } from '../../api';
import { getCachedSessionList } from '../../utils/sessionListCache';
import { subscribeSessionNameChanges } from '../../utils/sessionNameSync';
import { userActionHeaders } from '../../utils/userAction';

type Answer = {
  sessionId: string;
  /** `null` when the row could not be read or holds no name. */
  name: string | null;
};

function cachedName(sessionId: string): string | null {
  const row = getCachedSessionList()?.find((session) => session.id === sessionId);
  return row?.name ? row.name : null;
}

export type ParentChatName = {
  /** The parent's name, once known. */
  name?: string;
  /** True once the lookup has answered, with a name or without one. */
  settled: boolean;
};

export function useParentChatName(parentSessionId: string | undefined): ParentChatName {
  const [answer, setAnswer] = useState<Answer | null>(() => {
    const name = parentSessionId ? cachedName(parentSessionId) : null;
    return parentSessionId && name ? { sessionId: parentSessionId, name } : null;
  });

  useEffect(() => {
    if (!parentSessionId) return;
    let cancelled = false;
    // A rename seen while the read is out is newer than whatever the read
    // brings back, so the read must not put the old name back over it.
    let renamed = false;

    const unsubscribe = subscribeSessionNameChanges((change) => {
      if (change.sessionId !== parentSessionId || !change.name) return;
      renamed = true;
      setAnswer({ sessionId: parentSessionId, name: change.name });
    });

    const cached = cachedName(parentSessionId);
    if (cached) {
      setAnswer({ sessionId: parentSessionId, name: cached });
    } else {
      void (async () => {
        let name: string | null = null;
        try {
          const response = await getSession({
            path: { session_id: parentSessionId },
            // A name is all this wants: the row, not the transcript.
            query: { metadata_only: true },
            // Not optional: a private parent's row is refused without it.
            headers: await userActionHeaders(),
          });
          const row = response.data;
          name = row && row.id === parentSessionId && row.name ? row.name : null;
        } catch {
          // A failed read leaves the line naming no chat; it never breaks it.
        }
        if (cancelled || renamed) return;
        setAnswer({ sessionId: parentSessionId, name });
      })();
    }

    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [parentSessionId]);

  if (!parentSessionId || answer?.sessionId !== parentSessionId) return { settled: false };
  return { name: answer.name ?? undefined, settled: true };
}
