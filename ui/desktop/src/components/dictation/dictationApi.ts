/**
 * The renderer's only HTTP door to the daemon's `/dictation/*` routes.
 *
 * Written by hand against the request and response shapes in
 * `docs/dictation/engine-and-models.md` ("Daemon routes and transport"), because
 * the routes (PR-15) and the generated client did not exist when the renderer
 * side landed. Everything above this module (the stream client in
 * `transport.ts`, the status store) sees only the normalized results declared
 * here, so moving the JSON calls onto the generated client changes this one
 * file. The audio `PUT` stays a hand written `fetch` either way: its body is raw
 * bytes, and a packaged `file://` page sends it with no CORS preflight
 * (measured).
 *
 * Headers: every route takes `X-Secret-Key`. Stream creation and the status
 * read also carry `userActionHeaders()`, the proof of a person on the desktop
 * and the host's provider on `serve`; per request, never as a default, for the
 * reason `utils/userAction.ts` gives. On a daemon with a user action key the
 * proof decides only which dictionary entries may bias and whether this caller
 * may install a model; streams take the secret alone (O-17).
 */
import { getApiUrl } from '../../config';
import { userActionHeaders } from '../../utils/userAction';

export type DictationTargetKind = 'chat' | 'new_chat' | 'editor' | 'crew' | 'external';

export interface StreamRequest {
  /** 16000, or the declared native rate. */
  sample_rate: number;
  format: 's16le';
  language?: string;
  target_kind: DictationTargetKind;
  /** `chat` and `editor` only. */
  session_id?: string;
  /** `new_chat` only; honoured only with the user action proof. */
  new_chat_provider?: string;
  /** `crew` and `external` only; sent only after MSG-PRIVATE-TERMS, for one recording. */
  include_private_terms?: boolean;
  /** At most 2 KiB, ranking only. */
  context?: string;
}

export interface StreamCreated {
  stream_id: string;
  max_chunk_bytes: number;
  idle_timeout_s: number;
  max_seconds: number;
}

export interface AudioAck {
  /** Seconds of audio the stream holds after this chunk. */
  received_s: number;
  /** Corrected text of the segments decoded so far, when it changed. */
  partial?: string | null;
  queue_position?: number | null;
}

export interface TranscriptChange {
  start: number;
  end: number;
  from: string;
  to: string;
  stage: string;
  entry_id?: string;
}

export interface Transcript {
  text: string;
  raw_text: string;
  changes: TranscriptChange[];
  segments: { t0: number; t1: number; text: string }[];
  model: string;
  audio_s: number;
  elapsed_ms: number;
  bias_scope: 'public' | 'private';
  retried: null | { reason: string };
  no_speech: boolean;
}

/** `202 {pending: true}` from finish or a poll. */
export interface TranscriptPending {
  pending: true;
}

export type Allowance = { allowed: boolean; reason: string | null };

export interface DictationStatus {
  engine: {
    compiled: boolean;
    state: 'no_engine' | 'unsupported_cpu' | 'stopped' | 'starting' | 'ready' | 'error';
    backend: 'metal' | 'cpu' | null;
    device: string | null;
    threads: number;
    missing_cpu_features: string[];
    rss_bytes: number | null;
  };
  models: Array<{
    id: string;
    display_name: string;
    installed: boolean;
    bytes: number;
    location: 'user' | 'system' | 'managed' | 'env' | null;
    superseded: boolean;
    suitability:
      | 'recommended'
      | 'suitable'
      | 'too_slow'
      | 'below_minimum'
      | 'unsupported_cpu'
      | 'unknown';
    recommended: boolean;
  }>;
  active_model: string | null;
  language: string;
  install: Allowance;
  install_job: {
    model: string;
    bytes_done: number;
    bytes_total: number;
    bytes_per_second: number;
    phase: 'downloading' | 'verifying' | 'installed' | 'failed';
    error: string | null;
  } | null;
  policy: {
    managed: boolean;
    enabled: Allowance;
    capture: Allowance;
    install: Allowance;
    external_engine: Allowance;
    dictionary: Allowance;
    agent_writes: Allowance;
    allowed_models: string[] | null;
  };
  limits: { max_seconds: number; max_chunk_bytes: number; max_streams: number };
  queue: { streams: number; segments: number };
  account: string;
  host: string;
  models_dir: string;
}

/** Every refusal's body: `{code, message, expected_seq?}`. */
export interface DictationErrorBody {
  code: string;
  message: string;
  expected_seq?: number;
}

export type DictationResult<T> =
  | { ok: true; status: number; data: T }
  | {
      ok: false;
      status: number;
      error: DictationErrorBody;
      /** `Retry-After` in seconds, when the answer carried one. */
      retryAfterS: number | null;
    };

export interface DictationApi {
  status(signal?: AbortSignal): Promise<DictationResult<DictationStatus>>;
  createStream(body: StreamRequest, signal?: AbortSignal): Promise<DictationResult<StreamCreated>>;
  putAudio(
    streamId: string,
    seq: number,
    body: Uint8Array,
    signal?: AbortSignal
  ): Promise<DictationResult<AudioAck>>;
  /** 200 with the transcript, or 202 with `{pending: true}`. */
  finish(
    streamId: string,
    signal?: AbortSignal
  ): Promise<DictationResult<Transcript | TranscriptPending>>;
  /** As `finish`, answered at once. */
  poll(
    streamId: string,
    signal?: AbortSignal
  ): Promise<DictationResult<Transcript | TranscriptPending>>;
  /** `DELETE`; best effort, never throws. */
  drop(streamId: string): Promise<void>;
}

/**
 * The secret, read once and kept. `getSecretKey` is an IPC round trip on the
 * desktop, and the stop path should not pay it twice; a 401 forgets it so a
 * reconnected daemon's new secret is read again.
 */
let secretKey: Promise<string> | null = null;
function secret(): Promise<string> {
  secretKey ??= Promise.resolve()
    .then(() => window.electron.getSecretKey())
    .catch((error: unknown) => {
      secretKey = null;
      throw error;
    });
  return secretKey;
}

/** For tests: forget the cached secret. */
export function resetDictationApiForTests(): void {
  secretKey = null;
}

function retryAfter(response: Response): number | null {
  const raw = response.headers.get('Retry-After');
  if (!raw) return null;
  const seconds = Number(raw);
  if (Number.isFinite(seconds) && seconds >= 0) return seconds;
  const date = Date.parse(raw);
  return Number.isFinite(date) ? Math.max(0, Math.round((date - Date.now()) / 1000)) : null;
}

async function readBody(response: Response): Promise<unknown> {
  const text = await response.text().catch(() => '');
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function errorBody(status: number, body: unknown): DictationErrorBody {
  if (body && typeof body === 'object') {
    const record = body as Record<string, unknown>;
    return {
      code: typeof record.code === 'string' ? record.code : `http${status}`,
      message: typeof record.message === 'string' ? record.message : '',
      ...(typeof record.expected_seq === 'number' ? { expected_seq: record.expected_seq } : {}),
    };
  }
  return { code: `http${status}`, message: '' };
}

async function call<T>(
  path: string,
  init: RequestInit & { userAction?: boolean },
  okStatuses: readonly number[] = [200]
): Promise<DictationResult<T>> {
  const [key, proof] = await Promise.all([
    secret(),
    init.userAction ? userActionHeaders() : Promise.resolve({}),
  ]);
  const response = await fetch(getApiUrl(path), {
    ...init,
    headers: { ...(init.headers as Record<string, string>), 'X-Secret-Key': key, ...proof },
  });
  if (response.status === 401) secretKey = null;
  const body = await readBody(response);
  if (okStatuses.includes(response.status)) {
    return { ok: true, status: response.status, data: body as T };
  }
  return {
    ok: false,
    status: response.status,
    error: errorBody(response.status, body),
    retryAfterS: retryAfter(response),
  };
}

const streamPath = (streamId: string): string =>
  `/dictation/streams/${encodeURIComponent(streamId)}`;

export const dictationApi: DictationApi = {
  status: (signal) =>
    call<DictationStatus>('/dictation/status', { method: 'GET', signal, userAction: true }),
  createStream: (body, signal) =>
    call<StreamCreated>(
      '/dictation/streams',
      {
        method: 'POST',
        signal,
        userAction: true,
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      },
      [200, 201]
    ),
  putAudio: (streamId, seq, body, signal) =>
    call<AudioAck>(`${streamPath(streamId)}/audio?seq=${seq}`, {
      method: 'PUT',
      signal,
      headers: { 'Content-Type': 'application/octet-stream' },
      body: body as unknown as NonNullable<RequestInit['body']>,
    }),
  finish: (streamId, signal) =>
    call<Transcript | TranscriptPending>(
      `${streamPath(streamId)}/finish`,
      { method: 'POST', signal },
      [200, 202]
    ),
  poll: (streamId, signal) =>
    call<Transcript | TranscriptPending>(
      streamPath(streamId),
      { method: 'GET', signal },
      [200, 202]
    ),
  drop: async (streamId) => {
    try {
      await call<void>(streamPath(streamId), { method: 'DELETE' }, [200, 204]);
    } catch {
      // Best effort: the daemon drops an idle stream after 60 s anyway.
    }
  },
};

export const isTranscriptPending = (
  data: Transcript | TranscriptPending | null | undefined
): data is TranscriptPending => !!data && (data as TranscriptPending).pending === true;
