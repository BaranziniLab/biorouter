import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { useCrew } from '../state/CrewControllerContext';
import type { AccessRow } from './accessRows';
import { announceGrantsChanged } from './useCrewGrants';

/** The ordinary chat a session opens in. */
export const chatRoute = (sessionId: string) =>
  `/pair?resumeSessionId=${encodeURIComponent(sessionId)}`;

/**
 * Open and Stop for access rows, through the routes the rest of the app already uses: Open goes to
 * the conversation, Stop to the owned-run cancel route (as the timeline's Stop does). Revoke is the
 * grant list's own `revoke`.
 */
export function useAccessActions(): {
  onOpen(row: AccessRow): void;
  onStop(row: AccessRow): Promise<void>;
} {
  const { cancelRun } = useCrew();
  const navigate = useNavigate();
  const onOpen = useCallback((row: AccessRow) => navigate(chatRoute(row.sessionId)), [navigate]);
  const onStop = useCallback(
    async (row: AccessRow) => {
      // `cancelRun` records its own error for the connection bar and never throws; the list is
      // read again either way.
      await cancelRun(row.runId);
      announceGrantsChanged({
        connectionId: row.connectionId,
        sessionId: row.sessionId,
        change: 'stopped',
      });
    },
    [cancelRun]
  );
  return { onOpen, onStop };
}
