/**
 * The daemon's session reach gate, as a renderer test meets it on
 * `GET /knowledge/active` — one model of it, so the tests that need it cannot
 * drift into three different ideas of what the daemon does.
 *
 * The real gate is `refuse_unless_reachable` in
 * `crates/biorouter-server/src/routes/session_reach.rs`: a request naming a
 * PRIVATE chat is answered only for a caller whose stated capability covers it
 * or that carries the user's proof. The desktop app never states a capability
 * — it proves the person, with the `X-User-Action` header `userActionHeaders()`
 * builds from the preload bridge — so on the desktop the proof is the only way
 * through, and that is all this models. A chat that is not private, and a
 * request naming no chat at all, pass untouched: the gate is inert there.
 *
 * A refusal arrives the way the generated client delivers this route's
 * `text/plain` 403 (`api/client/client.gen.ts`): THROWN as a bare string under
 * `throwOnError: true`, and returned as `{ error }` with no `data` otherwise.
 */

/** What the stubbed preload bridge hands `userActionHeaders()`. */
export const USER_ACTION_KEY = 'renderer-test-user-action-key';

/**
 * The first sentence of `SESSION_OUT_OF_REACH` in `session_reach.rs`. The rest
 * of that paragraph is addressed to a model and never matters to a renderer
 * assertion; this sentence is the one the console line keeps.
 */
export const SESSION_OUT_OF_REACH = 'That chat is private, or there is no chat with that id.';

type GetActiveOptions = {
  query?: { session_id?: string };
  headers?: Record<string, string>;
  throwOnError?: boolean;
};

/**
 * A `getActive` implementation that refuses a private chat to a request without
 * the user's proof, and otherwise answers with `answer(sessionId)`.
 */
export function reachGatedGetActive(
  privateSessionIds: readonly string[],
  answer: (sessionId: string | undefined) => unknown
) {
  return (options?: GetActiveOptions) => {
    const sessionId = options?.query?.session_id;
    const namesPrivateChat = sessionId !== undefined && privateSessionIds.includes(sessionId);
    const proven = options?.headers?.['X-User-Action'] === USER_ACTION_KEY;
    if (namesPrivateChat && !proven) {
      return options?.throwOnError
        ? Promise.reject(SESSION_OUT_OF_REACH)
        : Promise.resolve({ data: undefined, error: SESSION_OUT_OF_REACH });
    }
    return Promise.resolve({ data: answer(sessionId) });
  };
}
