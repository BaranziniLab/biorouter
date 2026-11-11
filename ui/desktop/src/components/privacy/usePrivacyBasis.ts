import { useEffect, useState } from 'react';
import { getSession } from '../../api';
import { parsePrivacyBasis, type PrivacyBasis } from '../../utils/sessionDecisions';
import { userActionHeaders } from '../../utils/userAction';

/**
 * Why this chat is public when its own model is private (DR-33, design §5), read
 * from the single-session read (`GET /sessions/{id}`), which is the only answer
 * that carries it: a listing row shows its lock and nothing more, and the chat's
 * resume does not name it. The daemon computes the reason; nothing here folds.
 *
 * Read only while the chat is public, and again whenever `revision` changes (the
 * caller passes what can move it: its decision settings), with the person's proof
 * like every read that names a chat. `null` before an answer, for a chat that has
 * no basis, and when the read fails: a pill with no tip is the quiet fallback.
 */
export function usePrivacyBasis(
  sessionId: string | null | undefined,
  privacyTier: string | null | undefined,
  revision: unknown
): PrivacyBasis | null {
  const [basis, setBasis] = useState<PrivacyBasis | null>(null);
  const isPublic = privacyTier === 'public';
  useEffect(() => {
    if (!sessionId || !isPublic) {
      setBasis(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const response = await getSession({
          path: { session_id: sessionId },
          headers: await userActionHeaders(),
        });
        if (!cancelled) setBasis(parsePrivacyBasis(response.data));
      } catch {
        if (!cancelled) setBasis(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [sessionId, isPublic, revision]);
  return basis;
}
