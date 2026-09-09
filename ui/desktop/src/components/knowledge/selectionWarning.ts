import { errorMessage } from '../../utils/conversionUtils';

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
  const raw = errorMessage(err, 'unknown error').trim().replace(/\s+/g, ' ');
  if (!raw) return 'unknown error';
  // The first sentence, keeping its full stop. `. ` rather than `.` so a
  // version number or a file name is not mistaken for the end of one.
  const stop = raw.indexOf('. ');
  const first = stop === -1 ? raw : raw.slice(0, stop + 1);
  return first.length > MAX_REASON_CHARS ? `${first.slice(0, MAX_REASON_CHARS - 1)}…` : first;
}
