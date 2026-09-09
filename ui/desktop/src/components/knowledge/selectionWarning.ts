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
 * else.** Opening a private chat while a public model is bound refuses
 * `GET /knowledge/active` with ~900 characters addressed to an AI AGENT — "Do
 * not retry as you are… If this task genuinely needs that chat, stop and ask
 * the user to open it for you." That text is privacy-critical, pinned by
 * repo-grep tests in `crates/biorouter-server/src/routes/session_reach.rs`, and
 * nothing here changes it or should.
 *
 * What was wrong is where it landed: in the devtools console of a **desktop**
 * app, where the chat had opened correctly and rendered in full, and where the
 * only reader is a person who has no task, is not an agent, and cannot open the
 * chat "for" anybody. A normal, correct outcome printed as a wall of red-flag
 * prose reads as a crash.
 *
 * So the console keeps a one-line record — the read did not settle, and which
 * one — and the user-facing answer lives where a person will actually see it:
 * the composer's own note about the model this chat is pinned to
 * (`privacy/PinnedModelNote`).
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
