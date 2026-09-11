import { useCallback, useEffect, useState } from 'react';
import { listHistory, restoreState } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';
import type { HistoryEntry, RestoreResponse } from '../../../api/types.gen';

export interface UseHistoryResult {
  history: HistoryEntry[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  restore: (commitSha: string) => Promise<string>; // returns new commit sha
}

export function useHistory(kbId: string | null): UseHistoryResult {
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!kbId) {
      setHistory([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const res = await listHistory({
        path: { id: kbId },
        query: { limit: 200 },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      // ListHistoryResponses[200] is typed `unknown` in the generated SDK,
      // but the route returns `Vec<HistoryEntry>`.
      const data = (res.data ?? []) as HistoryEntry[];
      setHistory(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setHistory([]);
    } finally {
      setLoading(false);
    }
  }, [kbId]);

  const restore = useCallback(
    async (commitSha: string) => {
      if (!kbId) throw new Error('no active KB');
      const res = await restoreState({
        path: { id: kbId },
        body: { commit_sha: commitSha },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const sha = (res.data as RestoreResponse | undefined)?.new_commit_sha ?? '';
      await refresh();
      return sha;
    },
    [kbId, refresh]
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { history, loading, error, refresh, restore };
}
