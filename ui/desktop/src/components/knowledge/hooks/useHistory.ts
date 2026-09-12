import { useCallback, useEffect, useRef, useState } from 'react';
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

/**
 * A knowledge base's commit history, as the Change log shows it.
 *
 * @param enabled read only while this is true, and read AGAIN every time it
 * turns true. The Change log passes its `open` state.
 *
 * ⚠ **It used to read once per base, at mount, and never again.** The drawer
 * is mounted with the Knowledge view and merely hidden, so its one read ran
 * when the base became the subject — before anything had been digested into
 * it — and every ingest, lint, merge and restore after that was invisible until
 * the base changed. QA (2026-09-10 F13) digested a paragraph into a new base and
 * the log listed only `create knowledge base`, while `git log` held the two
 * `[ingest]` commits that wrote all ten pages. The route was never wrong: it
 * reads git. What the drawer showed was simply old.
 */
export function useHistory(kbId: string | null, enabled = true): UseHistoryResult {
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only the newest read may land. Reopening the log, or the subject changing
  // while a read is out, must not let an older answer — another base's — win.
  const requestRef = useRef(0);

  const refresh = useCallback(async () => {
    const request = ++requestRef.current;
    if (!kbId) {
      setHistory([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const res = await listHistory({
        path: { id: kbId },
        query: { limit: 200 },
        // The Knowledge view is the person at the keyboard, and says so the way
        // every other Knowledge request does (`KnowledgeContext.readSelection`).
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      if (request !== requestRef.current) return;
      // ListHistoryResponses[200] is typed `unknown` in the generated SDK,
      // but the route returns `Vec<HistoryEntry>`.
      setHistory((res.data ?? []) as HistoryEntry[]);
    } catch (err) {
      if (request !== requestRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
      setHistory([]);
    } finally {
      if (request === requestRef.current) setLoading(false);
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
    if (!enabled) return;
    void refresh();
  }, [enabled, refresh]);

  return { history, loading, error, refresh, restore };
}
