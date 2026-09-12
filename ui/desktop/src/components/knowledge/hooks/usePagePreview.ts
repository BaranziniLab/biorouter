// ui/desktop/src/components/knowledge/hooks/usePagePreview.ts
import { useEffect, useState } from 'react';
import { getPageBody, previewState } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';

export interface UsePagePreviewResult {
  content: string | null;
  loading: boolean;
  error: string | null;
}

export function usePagePreview(
  kbId: string | null,
  path: string | null,
  previewSha?: string | null
): UsePagePreviewResult {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!kbId || !path) {
      setContent(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    (async () => {
      try {
        // With the user's proof: a private base's pages are refused to any
        // caller without it (issue #56, QA 2026-09-10 H2).
        const headers = await userActionHeaders();
        const res = previewSha
          ? await previewState({
              path: { id: kbId },
              body: { commit_sha: previewSha, path },
              headers,
              throwOnError: true,
            })
          : await getPageBody({
              path: { id: kbId },
              query: { path },
              headers,
              throwOnError: true,
            });
        if (!cancelled) setContent(res.data?.content ?? null);
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
          setContent(null);
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [kbId, path, previewSha]);

  return { content, loading, error };
}
