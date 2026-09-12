import { useCallback, useRef, useState } from 'react';
import { buildKnowledgeUrl, getSecretKey } from './knowledgeRequest';
import { userActionHeaders } from '../../../utils/userAction';

export type SubAgentEvent =
  | { kind: 'step'; index: number; assistant_text: string }
  | { kind: 'tool_call'; name: string; args: unknown }
  | { kind: 'tool_result'; name: string; ok: boolean; summary: string }
  | { kind: 'done'; reason: string; final_text: string };

export interface StreamState {
  events: SubAgentEvent[];
  status: 'idle' | 'starting' | 'streaming' | 'stopping' | 'done' | 'error';
  finalResult: unknown;
  error?: string;
}

export interface StreamRunResult {
  status: 'done' | 'error' | 'aborted';
  error?: string;
}

/**
 * Shown when the response body ends before the backend sent `event: done` or
 * `event: error`. The client cannot infer whether backend work committed from a
 * broken stream, so the message directs the user back to authoritative state.
 */
export const STREAM_ENDED_WITHOUT_TERMINAL =
  'The digest stopped without reporting a result because the connection to the Biorouter backend ended mid-stream. The outcome is uncertain: curation may have committed or rolled back, and the raw source may have been retained. Refresh the knowledge base and history before retrying.';

function extractErrorMessage(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return '';

  try {
    const parsed = JSON.parse(trimmed) as { message?: string; error?: string };
    return parsed.message ?? parsed.error ?? trimmed;
  } catch {
    return trimmed;
  }
}

/**
 * SSE ingest stream hook.
 *
 * Uses raw fetch + ReadableStream because EventSource does not support POST bodies.
 * The backend emits `data: {json}\n\n` blocks, with terminal `event: done` or `event: error`.
 */
export function useIngestStream() {
  const [state, setState] = useState<StreamState>({
    events: [],
    status: 'idle',
    finalResult: null,
  });
  const abortRef = useRef<AbortController | null>(null);

  /**
   * Start the SSE stream. Resolves when the stream closes.
   * Returns the terminal status so callers don't need to read stale state.
   */
  const runStream = useCallback(
    async (
      path: string,
      requestInit: Pick<RequestInit, 'body' | 'headers'>
    ): Promise<StreamRunResult> => {
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;
      setState({ events: [], status: 'starting', finalResult: null });

      const url = await buildKnowledgeUrl(path);

      // Read the secret key directly from the Electron preload bridge,
      // matching how renderer.tsx sets up the API client. The cast
      // `cfg.headers as Record<string,string>` is unreliable because
      // HeadersInit can be a Headers instance or a [string,string][] array.
      const xSecretKey = await getSecretKey();
      // The user-action proof, as `knowledgeFetch` sends it: a macro names a
      // base, and since issue #56's QA sweep (2026-09-10) a request without
      // the proof is answered as a public model, which a private base refuses.
      const proof = await userActionHeaders();

      try {
        const res = await fetch(url, {
          method: 'POST',
          headers: {
            'X-Secret-Key': xSecretKey,
            ...proof,
            ...(requestInit.headers ?? {}),
          },
          body: requestInit.body,
          signal: controller.signal,
        });

        if (!res.ok || !res.body) {
          const bodyText = await res.text().catch(() => '');
          const details = extractErrorMessage(bodyText);
          throw new Error(details ? `HTTP ${res.status}: ${details}` : `HTTP ${res.status}`);
        }

        setState((s) => ({ ...s, status: s.status === 'stopping' ? 'stopping' : 'streaming' }));

        const reader = res.body.getReader();
        const decoder = new window.TextDecoder();
        let buf = '';
        // The backend closes every macro stream with `event: done` or
        // `event: error`. Until one of them arrives the outcome is unknown, and
        // "unknown" is a failure, not a success: a body that simply stops means
        // the digest died on the way out — the daemon exited, the socket
        // dropped, a proxy cut it. Defaulting to 'done' told the user their
        // source had been digested on the strength of nothing at all, and
        // IngestPanel then marked it ingested and cleared it off the staged
        // list, so the evidence went with it (issue #71).
        let terminalStatus: 'done' | 'error' = 'error';
        let terminalError: string | undefined = STREAM_ENDED_WITHOUT_TERMINAL;

        outer: while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += decoder.decode(value, { stream: true });
          const blocks = buf.split('\n\n');
          buf = blocks.pop() ?? '';

          for (const block of blocks) {
            const lines = block.split('\n');
            let eventName = 'message';
            let data = '';

            for (const line of lines) {
              if (line.startsWith('event: ')) eventName = line.substring(7).trim();
              else if (line.startsWith('data: ')) data += line.substring(6);
            }

            if (eventName === 'done') {
              const parsed = data ? (JSON.parse(data) as unknown) : null;
              setState((s) => ({ ...s, status: 'done', finalResult: parsed }));
              terminalStatus = 'done';
              terminalError = undefined;
              break outer;
            } else if (eventName === 'error') {
              const parsed = data
                ? (JSON.parse(data) as { message?: string })
                : { message: 'unknown' };
              const msg = parsed.message ?? 'unknown';
              setState((s) => ({ ...s, status: 'error', error: msg }));
              terminalStatus = 'error';
              terminalError = msg;
              break outer;
            } else if (data) {
              try {
                const ev = JSON.parse(data) as SubAgentEvent;
                setState((s) => ({ ...s, events: [...s.events, ev] }));
              } catch {
                // ignore malformed SSE data frames
              }
            }
          }
        }

        // The stream closed with no terminal frame: surface it as the failure it
        // is, naming what we know, rather than inventing a completion.
        if (terminalStatus === 'error' && terminalError === STREAM_ENDED_WITHOUT_TERMINAL) {
          setState((s) => ({ ...s, status: 'error', error: STREAM_ENDED_WITHOUT_TERMINAL }));
        }

        return { status: terminalStatus, error: terminalError };
      } catch (e) {
        if ((e as Error).name !== 'AbortError') {
          const msg = e instanceof Error ? e.message : String(e);
          setState((s) => ({ ...s, status: 'error', error: msg }));
          return { status: 'error', error: msg };
        }
        setState((s) => ({
          ...s,
          status: 'done',
          finalResult: { reason: 'cancelled' },
          events:
            s.events[s.events.length - 1]?.kind === 'done'
              ? s.events
              : [...s.events, { kind: 'done', reason: 'cancelled', final_text: '' }],
        }));
        return { status: 'aborted' };
      }
    },
    []
  );

  const start = useCallback(
    async (path: string, body: unknown): Promise<StreamRunResult> =>
      runStream(path, {
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(body),
      }),
    [runStream]
  );

  const startMultipart = useCallback(
    async (path: string, formData: FormData): Promise<StreamRunResult> =>
      runStream(path, {
        body: formData,
      }),
    [runStream]
  );

  /**
   * Drop the log and return to `idle`, aborting anything still in flight.
   *
   * ⚠ **This is a per-SUBJECT reset, not a cosmetic one.** The hook's state is
   * the log of one run, and the panel that renders it is bound to whichever
   * knowledge base is primary. Switching bases left the previous base's
   * "Digest complete · 38 events" on screen — attached, to the reader, to the
   * base now named above it — and it stayed there through the model check of
   * the next digest, so the first thing a new run showed was the old run's
   * verdict. A stale log is worse than none: it is a claim about the wrong base.
   */
  const reset = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setState({ events: [], status: 'idle', finalResult: null });
  }, []);

  const abort = useCallback(() => {
    setState((s) => {
      if (s.status === 'idle' || s.status === 'done' || s.status === 'error') {
        return s;
      }
      return { ...s, status: 'stopping' };
    });
    abortRef.current?.abort();
  }, []);

  return { ...state, start, startMultipart, abort, reset };
}
