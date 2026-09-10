export async function safeJsonParse<T>(
  response: Response,
  errorMessage: string = 'Failed to parse server response'
): Promise<T> {
  try {
    return (await response.json()) as T;
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error(errorMessage);
    }
    throw error;
  }
}

export function errorMessage(err: Error | unknown, default_value?: string) {
  if (err instanceof Error) {
    return err.message;
  } else if (typeof err === 'object' && err !== null && 'message' in err) {
    return String(err.message);
  } else {
    return default_value || String(err);
  }
}

// A fetch to a down/unreachable backend rejects with `TypeError: Failed to fetch`
// (Chromium/Electron) *before* any HTTP response exists — distinct from a real
// HTTP error (a 4xx/5xx throws with a parsed response body, not a TypeError).
// Used to give backend-disconnected UX its own copy without swallowing the
// message. A real HTTP failure returns false so it keeps its own error text.
export function isConnectionError(err: Error | unknown): boolean {
  if (err instanceof TypeError) return true;
  const msg = errorMessage(err).toLowerCase();
  return (
    msg.includes('failed to fetch') ||
    msg.includes('fetch failed') ||
    msg.includes('networkerror') ||
    msg.includes('load failed') || // WebKit wording
    msg.includes('err_connection')
  );
}

/**
 * A human-readable one-line description of a failed generated-client call.
 *
 * The generated client does NOT throw an `Error`. With `throwOnError: true` it
 * throws the RESPONSE BODY — the parsed JSON when the body is JSON, the raw
 * text when it is not, and a literal `{}` when there is no body at all
 * (`finalError = finalError || {}` in `api/client/client.gen.ts`) — and it
 * discards the `Response`, so the status goes with it.
 *
 * That is why `console.warn('…:', error)` printed a bare `{}` on the Stop path
 * and told the reader nothing: `/agent/cancel` answers several of its failures
 * with a bare status and no body — `CancelTurnFailure::SettlementTimeout` is a
 * naked 504, the two continuation-argument refusals are naked 400s — so the
 * body genuinely IS empty and the status is the only thing that says which
 * failure this was. Pass it in whenever the call site can still see it.
 *
 * Always log the STRING this returns rather than the raw value: a raw object
 * argument is what produced the empty `{}` in the first place.
 */
export function describeRequestFailure(err: unknown, status?: number): string {
  const parts: string[] = [];
  if (typeof status === 'number') parts.push(`HTTP ${status}`);

  if (err instanceof Error) {
    parts.push(err.name && err.name !== 'Error' ? `${err.name}: ${err.message}` : err.message);
    return parts.join(' — ');
  }

  if (typeof err === 'string') {
    parts.push(err.trim() || 'empty response body');
    return parts.join(' — ');
  }

  if (err && typeof err === 'object') {
    const record = err as Record<string, unknown>;
    // The shapes biorouterd actually returns: a `{ error }` / `{ message }`
    // envelope, a typed conflict (`{ mismatch, expected_turn_id, … }`), or a
    // validation `{ detail }`. Named first so they read as prose; anything
    // else falls through to the whole payload.
    const named = ['error', 'message', 'detail', 'reason', 'code']
      .filter((key) => record[key] != null && record[key] !== '')
      .map(
        (key) =>
          `${key}=${typeof record[key] === 'string' ? record[key] : JSON.stringify(record[key])}`
      );
    if (named.length > 0) {
      parts.push(named.join(' '));
      return parts.join(' — ');
    }
    let serialised: string;
    try {
      serialised = JSON.stringify(record);
    } catch {
      serialised = '[unserialisable payload]';
    }
    // `{}` here is not an error object that lost its fields — it is the
    // generated client's stand-in for "the response carried no body".
    parts.push(serialised === '{}' ? 'empty response body' : serialised);
    return parts.join(' — ');
  }

  parts.push(err === undefined ? 'no error value' : String(err));
  return parts.join(' — ');
}
