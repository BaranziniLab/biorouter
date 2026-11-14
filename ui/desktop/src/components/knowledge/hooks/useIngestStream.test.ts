import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  STREAM_ENDED_DETAIL,
  STREAM_ENDED_WITHOUT_TERMINAL,
  useIngestStream,
} from './useIngestStream';

vi.mock('./knowledgeRequest', () => ({
  buildKnowledgeUrl: async (path: string) => `http://backend.test${path}`,
  getSecretKey: async () => 'secret-123',
}));

/** A `fetch` that replies 200 with the given SSE text, delivered in one chunk. */
function sseResponse(body: string): Response {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(body));
      controller.close();
    },
  });
  // jsdom's Response does not accept a stream body, so hand the hook the two
  // fields it actually reads.
  return { ok: true, status: 200, body: stream } as unknown as Response;
}

describe('useIngestStream terminal frames', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('reports a backend error frame as an error, with its message', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          sseResponse(
            'data: {"kind":"step","index":0,"assistant_text":""}\n\n' +
              'event: error\ndata: {"message":"ingest wrote no knowledge pages for source hrv-note"}\n\n'
          )
        )
    );

    const { result } = renderHook(() => useIngestStream());

    let run!: Awaited<ReturnType<typeof result.current.start>>;
    await act(async () => {
      run = await result.current.start('/knowledge/bases/kb/ingest', {});
    });

    expect(run.status).toBe('error');
    expect(run.error).toContain('no knowledge pages');
    expect(result.current.status).toBe('error');
    expect(result.current.error).toContain('no knowledge pages');
  });

  it('reports a done frame as done', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(sseResponse('event: done\ndata: {"source_id":"hrv","steps":3}\n\n'))
    );

    const { result } = renderHook(() => useIngestStream());

    let run!: Awaited<ReturnType<typeof result.current.start>>;
    await act(async () => {
      run = await result.current.start('/knowledge/bases/kb/ingest', {});
    });

    expect(run.status).toBe('done');
    expect(result.current.status).toBe('done');
  });

  // Issue #71. The backend closes the stream with either `event: done` or
  // `event: error`; a body that just stops means the digest died on the way —
  // the daemon crashed, the socket dropped, a proxy cut it. Defaulting that to
  // "done" told the user a digest had completed when nothing had come back to
  // say so, and `IngestPanel` then marked the source ingested and cleared it off
  // the staged list.
  it('treats a stream that ends without a terminal frame as a failure', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          sseResponse(
            'data: {"kind":"step","index":0,"assistant_text":"reading the source"}\n\n' +
              'data: {"kind":"tool_call","name":"kb_read_page","args":{}}\n\n'
          )
        )
    );

    const { result } = renderHook(() => useIngestStream());

    let run!: Awaited<ReturnType<typeof result.current.start>>;
    await act(async () => {
      run = await result.current.start('/knowledge/bases/kb/ingest', {});
    });

    expect(run.status).toBe('error');
    expect(result.current.status).toBe('error');
    // The error is one line; what it means and what to do is the detail the digest log shows
    // under it (`DispatchProgress`, asserted in IngestPanel.streamFailure.test.tsx).
    expect(run.error).toBe(STREAM_ENDED_WITHOUT_TERMINAL);
    expect(result.current.error).toBe(STREAM_ENDED_WITHOUT_TERMINAL);
    expect(STREAM_ENDED_DETAIL).toContain('outcome is uncertain');
    expect(STREAM_ENDED_DETAIL).toContain('Refresh the knowledge base and history');
    expect(`${STREAM_ENDED_WITHOUT_TERMINAL} ${STREAM_ENDED_DETAIL}`).not.toContain(
      'Nothing was added'
    );
  });

  it('treats an entirely empty stream as a failure too', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseResponse('')));

    const { result } = renderHook(() => useIngestStream());

    let run!: Awaited<ReturnType<typeof result.current.start>>;
    await act(async () => {
      run = await result.current.start('/knowledge/bases/kb/ingest', {});
    });

    expect(run.status).toBe('error');
    expect(result.current.status).toBe('error');
    expect(result.current.error).toBe(STREAM_ENDED_WITHOUT_TERMINAL);
  });

  // The other side of the same change. A stream with no terminal frame is now a
  // failure — but the user pressing Stop produces exactly that shape, and it is
  // not a failure. `IngestPanel` distinguishes them only by the returned status:
  // 'aborted' puts the source back to pending with "Stopped before completion",
  // while 'error' would tell someone who stopped a digest on purpose that the
  // connection to the backend had dropped.
  it('reports a user-initiated stop as aborted, not as a failure', async () => {
    // Faithful to what an aborted fetch does, on whichever side of the abort the
    // request happens to be: reject outright if the signal already fired,
    // otherwise deliver a body that errors the moment it does.
    const abortError = () => Object.assign(new Error('aborted'), { name: 'AbortError' });
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (_url: string, init: RequestInit) => {
        const signal = init.signal!;
        if (signal.aborted) throw abortError();
        const body = new ReadableStream<Uint8Array>({
          start(controller) {
            signal.addEventListener('abort', () => controller.error(abortError()));
          },
        });
        return { ok: true, status: 200, body } as unknown as Response;
      })
    );

    const { result } = renderHook(() => useIngestStream());

    let run!: Awaited<ReturnType<typeof result.current.start>>;
    await act(async () => {
      const pending = result.current.start('/knowledge/bases/kb/ingest', {});
      result.current.abort();
      run = await pending;
    });

    expect(run.status).toBe('aborted');
    expect(result.current.status).not.toBe('error');
  });
});

/**
 * The log is the record of ONE run against ONE knowledge base, so the surface
 * holding it needs a way to say "that run is no longer about anything".
 */
describe('useIngestStream reset', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('returns a finished run to idle and drops its events', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          sseResponse(
            'data: {"kind":"step","index":0,"assistant_text":"working"}\n\n' +
              'event: done\ndata: {"source_id":"hrv"}\n\n'
          )
        )
    );

    const { result } = renderHook(() => useIngestStream());
    await act(async () => {
      await result.current.start('/knowledge/bases/kb-1/ingest', {});
    });
    expect(result.current.status).toBe('done');
    expect(result.current.events.length).toBeGreaterThan(0);

    act(() => result.current.reset());

    expect(result.current.status).toBe('idle');
    expect(result.current.events).toEqual([]);
    expect(result.current.finalResult).toBeNull();
    expect(result.current.error).toBeUndefined();
  });
});
