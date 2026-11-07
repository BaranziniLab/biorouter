/**
 * The renderer's only HTTP door to the daemon's `/dictation/*` routes.
 *
 * The JSON routes go through the client generated from the daemon's OpenAPI
 * document (PR-15): the status read, stream creation, the held read after a 202
 * and the delete. The two routes whose body is raw audio, `PUT …/audio` and
 * `finish` (which carries the last chunk, so a stop costs one round trip), stay a
 * hand written `fetch`, because their body is bytes the generated client would
 * have to be talked out of serializing. Everything above this module (the stream
 * client in `transport.ts`, the status store) sees only the normalized results
 * declared here, and the types they read are checked against the generated ones
 * below, so a change to a route's shape fails the type check here first.
 *
 * Headers: every route takes `X-Secret-Key`. Stream creation and the status read
 * also carry `userActionHeaders()`, the proof of a person on the desktop and the
 * host's provider on `serve`; per request, never as a default, for the reason
 * `utils/userAction.ts` gives. On a daemon with a user action key the proof
 * decides only which dictionary entries may bias and whether this caller may
 * install a model; streams take the secret alone (O-17).
 */
import {
  createDictationStream,
  deleteDictationStream,
  dictationStatus,
  getDictationStream,
} from '../../api';
import type {
  DictationAudioAck as GeneratedAudioAck,
  DictationStatus as GeneratedStatus,
  DictationStreamCreated as GeneratedStreamCreated,
  DictationStreamRequest as GeneratedStreamRequest,
  DictationTranscript as GeneratedTranscript,
} from '../../api';
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
  /** `new_chat` only, beside `new_chat_provider` and under the same rule. */
  new_chat_model?: string;
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

/** Where the stop path's time went, as the daemon saw it. */
export interface TranscriptTimings {
  segments: number;
  /** From `finish` to the tail's decode starting: a segment still decoding, or a load. */
  wait_ms: number;
  tail_ms: number;
  tail_audio_s: number;
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
  timings?: TranscriptTimings;
}

/** `202 {pending: true}` from finish or a read. */
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

/**
 * The module's shapes against the generated ones. A generated answer must be
 * readable as the shape declared here, and a request built here must be one the
 * route takes; a route that changes its shape fails the type check on this line
 * rather than in a renderer that reads a field that is gone. (The declared shapes
 * stay because they are narrower: `backend` is `'metal' | 'cpu'`, not any string.)
 */
type Readable<Generated, Declared> = keyof Declared extends keyof Generated ? true : false;
const shapesAgree: [
  Readable<GeneratedStatus, DictationStatus>,
  Readable<GeneratedTranscript, Transcript>,
  Readable<GeneratedStreamCreated, StreamCreated>,
  Readable<GeneratedAudioAck, AudioAck>,
  StreamRequest extends GeneratedStreamRequest ? true : false,
] = [true, true, true, true, true];
void shapesAgree;

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

/** The tail chunk `finish` carries: the next `seq` and its bytes. */
export interface FinishTail {
  seq: number;
  body: Uint8Array;
}

export interface DictationApi {
  status(signal?: AbortSignal): Promise<DictationResult<DictationStatus>>;
  createStream(body: StreamRequest, signal?: AbortSignal): Promise<DictationResult<StreamCreated>>;
  putAudio(
    streamId: string,
    seq: number,
    body: Uint8Array,
    signal?: AbortSignal
  ): Promise<DictationResult<AudioAck>>;
  /** 200 with the transcript, or 202 with `{pending: true}`; `tail` rides along. */
  finish(
    streamId: string,
    tail?: FinishTail,
    signal?: AbortSignal
  ): Promise<DictationResult<Transcript | TranscriptPending>>;
  /** As `finish`, held by the daemon up to `waitMs` (at most 3,000) for the transcript. */
  poll(
    streamId: string,
    waitMs?: number,
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

function retryAfter(response: Response | undefined): number | null {
  const raw = response?.headers.get('Retry-After');
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

function settle<T>(
  response: Response,
  body: unknown,
  okStatuses: readonly number[]
): DictationResult<T> {
  if (response.status === 401) secretKey = null;
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

/**
 * What every generated call is given: the secret and, where asked, the proof; and the
 * same daemon address the audio routes use (`getApiUrl`), so the two halves of this
 * module never reach two different daemons, also in a page that never configured the
 * shared client (the capture probe).
 */
async function request(
  userAction: boolean
): Promise<{ baseUrl: string; headers: Record<string, string> }> {
  return { baseUrl: getApiUrl('/').replace(/\/$/, ''), headers: await headers(userAction) };
}

async function headers(userAction: boolean): Promise<Record<string, string>> {
  const [key, proof] = await Promise.all([
    secret(),
    userAction ? userActionHeaders() : Promise.resolve({}),
  ]);
  return { 'X-Secret-Key': key, ...proof };
}

/** The generated client's answer as a result; a request that never got one throws. */
async function generated<T>(
  call: Promise<{ data?: unknown; error?: unknown; response?: Response }>,
  okStatuses: readonly number[]
): Promise<DictationResult<T>> {
  const { data, error, response } = await call;
  if (!response) throw error ?? new Error('no response');
  return settle<T>(response, data ?? error ?? null, okStatuses);
}

/** One of the two routes whose body is raw audio. */
async function sendBytes<T>(
  path: string,
  method: 'PUT' | 'POST',
  body: Uint8Array | undefined,
  signal: AbortSignal | undefined,
  okStatuses: readonly number[]
): Promise<DictationResult<T>> {
  const response = await fetch(getApiUrl(path), {
    method,
    signal,
    headers: {
      ...(await headers(false)),
      ...(body ? { 'Content-Type': 'application/octet-stream' } : {}),
    },
    body: body as unknown as RequestInit['body'],
  });
  return settle<T>(response, await readBody(response), okStatuses);
}

const streamPath = (streamId: string): string =>
  `/dictation/streams/${encodeURIComponent(streamId)}`;

export const dictationApi: DictationApi = {
  status: async (signal) =>
    generated<DictationStatus>(dictationStatus({ signal, ...(await request(true)) }), [200]),
  createStream: async (body, signal) =>
    generated<StreamCreated>(
      createDictationStream({ body, signal, ...(await request(true)) }),
      [200, 201]
    ),
  putAudio: (streamId, seq, body, signal) =>
    sendBytes<AudioAck>(`${streamPath(streamId)}/audio?seq=${seq}`, 'PUT', body, signal, [200]),
  finish: (streamId, tail, signal) =>
    sendBytes<Transcript | TranscriptPending>(
      tail ? `${streamPath(streamId)}/finish?seq=${tail.seq}` : `${streamPath(streamId)}/finish`,
      'POST',
      tail?.body,
      signal,
      [200, 202]
    ),
  poll: async (streamId, waitMs, signal) =>
    generated<Transcript | TranscriptPending>(
      getDictationStream({
        path: { id: streamId },
        query: waitMs ? { wait_ms: waitMs } : undefined,
        signal,
        ...(await request(false)),
      }),
      [200, 202]
    ),
  drop: async (streamId) => {
    try {
      await generated<void>(
        deleteDictationStream({ path: { id: streamId }, ...(await request(false)) }),
        [200, 204]
      );
    } catch {
      // Best effort: the daemon drops an idle stream after 60 s anyway.
    }
  },
};

export const isTranscriptPending = (
  data: Transcript | TranscriptPending | null | undefined
): data is TranscriptPending => !!data && (data as TranscriptPending).pending === true;
