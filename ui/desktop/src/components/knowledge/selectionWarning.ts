/**
 * How long a console warning is allowed to be before it stops being a warning.
 *
 * Chosen to fit the first sentence of the refusals this actually sees, and to
 * be visibly shorter than the paragraph it replaced.
 */
const MAX_REASON_CHARS = 160;

/**
 * A failed knowledge-selection read, reduced to one line for the console.
 *
 * ⚠ **The paragraph this trims is deliberate, correct, and written for somebody
 * else.** A request naming a private chat on `GET /knowledge/active` with
 * neither the user's proof nor a private model behind it is refused with ~900
 * characters addressed to an AI AGENT — "Do not retry as you are… If this task
 * genuinely needs that chat, stop and ask the user to open it for you." That
 * text is privacy-critical, pinned by repo-grep tests in
 * `crates/biorouter-server/src/routes/session_reach.rs`, and nothing here
 * changes it or should. In a console its only reader is a person, who has no
 * task, is not an agent and cannot open the chat "for" anybody, so the console
 * keeps a one-line record: the read did not settle, and which one.
 *
 * ⚠ **Until 2026-09-11 this said the refusal was "a normal, correct outcome"
 * for a private chat opened in the desktop app while a public model was bound.
 * It was neither.** The desktop's reads carried no proof, so the daemon refused
 * every private chat whatever model was bound (measured with a chat on its own
 * private model), and the renderer went on to show — and write back — the
 * selection it had cached. The reads carry `userActionHeaders()` now, as the
 * write always did (`KnowledgeContext.tsx`). The refusal still reaches a
 * console from a caller that genuinely has neither: a daemon started without a
 * user-action key, or a browser tab not running a private model.
 *
 * Trimming to the first sentence rather than to a fixed prefix is what keeps
 * this useful for the failures that are NOT the refusal: "Failed to fetch" and
 * a 500 both survive intact.
 */
export function briefSelectionFailure(err: unknown): string {
  const raw = rawText(err).trim().replace(/\s+/g, ' ');
  if (!raw) return 'unknown error';
  // The first sentence, keeping its full stop. `. ` rather than `.` so a
  // version number or a file name is not mistaken for the end of one.
  const stop = raw.indexOf('. ');
  const first = stop === -1 ? raw : raw.slice(0, stop + 1);
  return first.length > MAX_REASON_CHARS ? `${first.slice(0, MAX_REASON_CHARS - 1)}…` : first;
}

/**
 * Whatever text the thrown value carries.
 *
 * ⚠ **Not `utils/conversionUtils.errorMessage`, and the difference was measured
 * in the running app.** The generated API client with `throwOnError` throws the
 * *response body*, and this route's body is `text/plain` — so the thrown value
 * is a bare STRING. `errorMessage(err, 'unknown error')` reaches its last arm
 * for a string and returns the DEFAULT rather than the string, so routing
 * through it printed `Knowledge selection not hydrated: unknown error` and
 * threw away the one fact the line existed to carry. Quieting a warning is not
 * the same as emptying it.
 *
 * The object arms cover the JSON routes and a thrown `Error`; anything with no
 * text at all falls through to the caller's placeholder.
 */
function rawText(err: unknown): string {
  if (typeof err === 'string') return err;
  if (err instanceof Error) return err.message;
  if (typeof err === 'object' && err !== null) {
    const record = err as Record<string, unknown>;
    for (const key of ['message', 'error', 'detail']) {
      const value = record[key];
      if (typeof value === 'string' && value.trim()) return value;
      if (typeof value === 'object' && value !== null) {
        const nested = (value as Record<string, unknown>).message;
        if (typeof nested === 'string' && nested.trim()) return nested;
      }
    }
  }
  return '';
}
