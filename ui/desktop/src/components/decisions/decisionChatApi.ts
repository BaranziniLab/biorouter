/**
 * The daemon calls the chat's decision surfaces make that the shared module
 * (`utils/sessionDecisions.ts`) does not: one decision-log row per card, read once
 * per window, and a decision chat's question (`decision_chat.v1`, through
 * `GET` and `PUT /sessions/{id}/decision-chat`).
 *
 * Every call carries the user-action proof (`userActionHeaders()`): a private
 * chat's decisions are refused without it (`session_reach`), and a missing proof
 * shows as a row that never loads, not as an error the renderer can see.
 */
import { useEffect, useState } from 'react';
import { client } from '../../api/client.gen';
import { userActionHeaders } from '../../utils/userAction';
import {
  fetchDecisionDetail,
  parseQuestion,
  questionToWire,
  refusalText,
  type DecisionRecord,
  type Question,
} from '../../utils/sessionDecisions';

type Loaded = { status: 'ready'; record: DecisionRecord } | { status: 'missing' };

/**
 * A row never changes once written and its id is a fresh UUID, so a row read once
 * is kept for the life of the window: a card that scrolls out and back, or a chat
 * reopened in another tab, draws without a second read. A failed read is not
 * kept, so the next mount asks again.
 */
const cache = new Map<string, Loaded>();
const inFlight = new Map<string, Promise<Loaded>>();

const cacheKey = (sessionId: string, decisionId: string) => `${sessionId}\u0000${decisionId}`;

export function resetDecisionRecordCacheForTests(): void {
  cache.clear();
  inFlight.clear();
}

/** `GET /sessions/{session_id}/decisions/{decision_id}`, once per row. */
export function fetchDecisionRecord(sessionId: string, decisionId: string): Promise<Loaded> {
  const key = cacheKey(sessionId, decisionId);
  const known = cache.get(key);
  if (known) return Promise.resolve(known);
  const pending = inFlight.get(key);
  if (pending) return pending;
  const read = fetchDecisionDetail(sessionId, decisionId).then(
    (detail): Loaded =>
      detail ? { status: 'ready', record: detail.record } : { status: 'missing' }
  );
  inFlight.set(key, read);
  read.then(
    (loaded) => {
      cache.set(key, loaded);
      inFlight.delete(key);
    },
    () => inFlight.delete(key)
  );
  return read;
}

export type DecisionRecordState =
  | { status: 'loading' }
  | { status: 'ready'; record: DecisionRecord }
  | { status: 'missing' }
  | { status: 'error' };

/** One decision-log row for a card: read once, then drawn from the window's cache. */
export function useDecisionRecord(sessionId: string, decisionId: string): DecisionRecordState {
  const key = cacheKey(sessionId, decisionId);
  const [state, setState] = useState<{ key: string; value: DecisionRecordState }>(() => ({
    key,
    value: cache.get(key) ?? { status: 'loading' },
  }));
  useEffect(() => {
    const known = cache.get(key);
    if (known) {
      setState({ key, value: known });
      return;
    }
    let cancelled = false;
    fetchDecisionRecord(sessionId, decisionId).then(
      (loaded) => {
        if (!cancelled) setState({ key, value: loaded });
      },
      () => {
        if (!cancelled) setState({ key, value: { status: 'error' } });
      }
    );
    return () => {
      cancelled = true;
    };
  }, [sessionId, decisionId, key]);
  return state.key === key ? state.value : (cache.get(key) ?? { status: 'loading' });
}

// ---------------------------------------------------------------------------
// A decision chat's question (`decision_chat.v1`)
// ---------------------------------------------------------------------------

const SECURITY = [{ name: 'X-Secret-Key', type: 'apiKey' as const }];

/** The stored question in a `DecisionChatResponse` (`{ settings: { question } | null }`). */
export function decisionChatQuestionOf(body: unknown): Question | null {
  if (!body || typeof body !== 'object') return null;
  const settings = (body as Record<string, unknown>).settings;
  if (!settings || typeof settings !== 'object') return null;
  const question = (settings as Record<string, unknown>).question;
  if (!question || typeof question !== 'object') return null;
  return parseQuestion(question);
}

/**
 * `GET /sessions/{session_id}/decision-chat`: the question the chat asks of each
 * message, or `null` when it has none. A daemon that predates the route (404)
 * reads as none. Any other failure throws.
 */
export async function fetchDecisionChatQuestion(
  sessionId: string,
  signal?: AbortSignal
): Promise<Question | null> {
  const headers = await userActionHeaders();
  const result = (await client.get({
    security: SECURITY,
    url: '/sessions/{session_id}/decision-chat',
    path: { session_id: sessionId },
    headers,
    signal,
  })) as { data?: unknown; error?: unknown; response?: Response };
  if (result.response?.status === 404) return null;
  if (result.error !== undefined) {
    throw new Error(refusalText(result.error) ?? 'Decision question unavailable');
  }
  return decisionChatQuestionOf(result.data);
}

export type SaveDecisionChatResult =
  | { ok: true }
  | { ok: false; status: number | null; message: string | null };

/**
 * `PUT /sessions/{session_id}/decision-chat` with `{ question }`, carrying the
 * user-action proof, which a daemon that holds a user-action key requires.
 * These two calls go through the generated client's own `client`, by URL, until
 * `npm run generate-api` declares the routes.
 */
export async function saveDecisionChatQuestion(
  sessionId: string,
  question: Question
): Promise<SaveDecisionChatResult> {
  const headers = await userActionHeaders();
  const result = (await client.put({
    security: SECURITY,
    url: '/sessions/{session_id}/decision-chat',
    path: { session_id: sessionId },
    body: { question: questionToWire(question) },
    headers: { 'Content-Type': 'application/json', ...headers },
  })) as { error?: unknown; response?: Response };
  const status = result.response?.status ?? null;
  if (result.error === undefined && status !== null && status < 400) return { ok: true };
  return { ok: false, status, message: refusalText(result.error) };
}
