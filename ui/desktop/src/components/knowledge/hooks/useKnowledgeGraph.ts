import { useCallback, useEffect, useState } from 'react';
import { getGraph } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';
import type { Graph } from '../../../api/types.gen';

export interface UseKnowledgeGraphResult {
  graph: Graph | null;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export function useKnowledgeGraph(kbId: string | null): UseKnowledgeGraphResult {
  const [graph, setGraph] = useState<Graph | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!kbId) {
      setGraph(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      // With the user's proof: a private base is refused to any caller without
      // it (issue #56, QA 2026-09-10 H2), and this view is the user.
      const res = await getGraph({
        path: { id: kbId },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      setGraph(res.data ?? null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setGraph(null);
    } finally {
      setLoading(false);
    }
  }, [kbId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { graph, loading, error, refresh };
}
