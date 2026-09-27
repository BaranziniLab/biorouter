/**
 * The one place attacker-controlled text is reduced to a label.
 *
 * Titles, locators and revisions arrive from places nobody vetted: a web page
 * picks its own `<title>`, a `.docx` carries its own metadata, and a filename
 * may hold a newline on every platform this ships to. Left raw such a string is
 * not a label at all — a newline writes extra *lines* into whatever prose
 * quotes it (which is how a page forges the fields describing itself, and the
 * trust notice beside them, without one markup character), and a bidi override
 * rewrites what the **user** sees in their own composer before they can review
 * it.
 *
 * `\p{Cc}` is the C0 block (so `\n`, `\r`, ESC, BEL), DEL and C1. `\p{Cf}` is
 * every format character: the bidi overrides `U+202A..U+202E`, the isolates
 * `U+2066..U+2069`, the zero-width run `U+200B..U+200F`, `U+2060..U+206F`,
 * `U+FEFF`, `U+061C`, and the invisible `U+E0000` tag block. Together they are
 * the whole class, which is why this is a category match and not a list of
 * ranges somebody has to remember to extend.
 *
 * {@link stripHiddenCharacters} is the ONE definition of that drop set in the
 * renderer. A surface that needs a stricter rule (a Crew display name also
 * refuses private use, unassigned code points and every default-ignorable)
 * removes its extra classes FIRST and then calls it, rather than restating
 * these two — `annotationChannel.test.ts` fails on a second copy, because a
 * copy is the one that drifts.
 *
 * ⚠ The order is not a matter of taste. Any lone-surrogate (`\p{Cs}`) removal
 * in particular must run before this one: deleting a format character can
 * leave a high and a low surrogate adjacent and fuse them into a real pair, so
 * `\uD800\u2066\uDC00` stripped here first becomes U+10000, which a `\p{Cs}`
 * pass run afterwards can no longer see. The reference implementation is
 * `components/crew/identity/displayText.ts`.
 *
 * Markup is deliberately left alone: callers frame these values into different
 * syntaxes and each owns the escaping its own syntax needs.
 *
 * Mirrors `sanitize_untrusted_label` in `crates/biorouter/src/utils.rs`, which
 * guards the same values on their way into a prompt. The two differ in one
 * detail worth knowing: this one caps the *output*, that one caps the input it
 * reads. Both bound the result; neither lets padding smuggle text past the cap.
 */

export const UNTRUSTED_LABEL_MAX_CHARS = 256;

/**
 * Removes every control (`\p{Cc}`) and format (`\p{Cf}`) character, and
 * nothing else — no trimming, no cap. The shared primitive; see the module
 * comment for what the two categories hold and why there is only one copy.
 */
export function stripHiddenCharacters(value: string): string {
  return value.replace(/[\p{Cc}\p{Cf}]/gu, '');
}

export function sanitizeUntrustedLabel(
  value: string,
  maxChars: number = UNTRUSTED_LABEL_MAX_CHARS
): string {
  return stripHiddenCharacters(value).trim().slice(0, maxChars);
}

/** A label that always names something, for surfaces with nowhere to put a blank. */
export function sanitizeArtifactTitle(title: string, fallback = 'Artifact'): string {
  return sanitizeUntrustedLabel(title) || sanitizeUntrustedLabel(fallback) || 'Artifact';
}
