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

// ---------------------------------------------------------------------------------------------
// Text shown as written, with its hidden characters made visible
// ---------------------------------------------------------------------------------------------

/**
 * One piece of a text split by {@link revealHiddenCharacters}: words to draw as they are, or one
 * hidden character to draw as its escape.
 */
export type RevealedSegment =
  | { kind: 'text'; text: string }
  | {
      kind: 'hidden';
      /** The character itself, for whatever copies the text: a copy keeps the raw bytes. */
      raw: string;
      /** What is drawn in its place: `\u{202e}`, the command line's escape for it. */
      escape: string;
      /** `U+202E`, for a tooltip or an accessible name. */
      codePoint: string;
    };

/**
 * The explicit embeddings, overrides and isolates (`U+202A..U+202E`, `U+2066..U+2069`): each
 * reorders what follows it, so a message could read `invoice_exe.png` while it holds
 * `invoice_{U+202E}gnp.exe`. Never legitimate in a message somebody else wrote; the marks
 * `U+200E`, `U+200F` and `U+061C` that right-to-left text does use are not among them.
 */
const DIRECTION_CONTROL = /^[\u{202A}-\u{202E}\u{2066}-\u{2069}]$/u;
/**
 * The direction marks (`U+200E`, `U+200F`, `U+061C`). Right-to-left text needs them to keep
 * punctuation and embedded Latin words where they belong, so they stay in text that has any
 * right-to-left letter. In text that has none they have nothing to do but reorder runs: a strong
 * right-to-left mark inside `invoice.exe` lays the pieces out right to left.
 */
const DIRECTION_MARK = /^[\u{200E}\u{200F}\u{061C}]$/u;
/** A letter of a script written right to left (a letter: the Arabic letter mark is Arabic too). */
const RIGHT_TO_LEFT_LETTER =
  /(?=\p{L})[\p{Script=Hebrew}\p{Script=Arabic}\p{Script=Syriac}\p{Script=Thaana}\p{Script=Nko}\p{Script=Samaritan}\p{Script=Mandaic}\p{Script=Adlam}\p{Script=Hanifi_Rohingya}]/u;
/** A control character (C0, DEL, C1): drawn as nothing, or as a box, unless it is a tab or a break. */
const CONTROL = /^\p{Cc}$/u;
const LAID_OUT_CONTROLS = new Set(['\t', '\n', '\r']);
/**
 * Zero-width characters that make one name look like another (`cre{U+200B}w_bob`). Thai, Khmer
 * and Lao use `U+200B` between words, so it is shown only where it can spoof something: see
 * {@link zeroWidthSpoofs}. `U+200C` and `U+200D`, the joiners Persian and every emoji sequence
 * depend on, are never touched.
 */
const ZERO_WIDTH = /^[\u{200B}\u{2060}\u{FEFF}]$/u;
/** The Unicode tag block, invisible outside a subdivision flag (England, Scotland, Wales). */
const TAG = /^[\u{E0000}-\u{E007F}]$/u;
const BLACK_FLAG = '\u{1F3F4}';
const CANCEL_TAG = '\u{E007F}';
/** Whitespace that separates tokens. Not JavaScript's `\s`, which counts `U+FEFF` as a space. */
const TOKEN_BREAK =
  /^[\t\n\v\f\r \u{00A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}]$/u;
/** A character a handle, address or file name is written in. */
const ASCII_WORD = /^[A-Za-z0-9_.@:/-]$/;

function escapeOf(character: string): string {
  return `\\u{${(character.codePointAt(0) ?? 0).toString(16)}}`;
}

function codePointOf(character: string): string {
  return `U+${(character.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, '0')}`;
}

/** A token that names somebody or somewhere: `@crew_bob`, `bob@lab.org`, a URL, a domain. */
function namesSomething(token: string): boolean {
  return (
    token.includes('@') ||
    token.includes('://') ||
    /(?:^|[^\p{L}\p{N}])www\./iu.test(token) ||
    /[\p{L}\p{N}-]\.\p{L}{2,}/u.test(token)
  );
}

/**
 * Whether a zero-width character at `index` of `characters` can make one name look like another:
 * inside a token that names somebody or somewhere, or beside a letter of a handle, an address or
 * a file name.
 */
function zeroWidthSpoofs(characters: readonly string[], index: number, token: string): boolean {
  if (namesSomething(token)) return true;
  const before = characters[index - 1] ?? '';
  const after = characters[index + 1] ?? '';
  return ASCII_WORD.test(before) || ASCII_WORD.test(after);
}

/** The whitespace-separated token around `index`, with its zero-width characters left out. */
function tokenAround(characters: readonly string[], index: number): string {
  let start = index;
  while (start > 0 && !TOKEN_BREAK.test(characters[start - 1])) start -= 1;
  let end = index;
  while (end < characters.length - 1 && !TOKEN_BREAK.test(characters[end + 1])) end += 1;
  return characters
    .slice(start, end + 1)
    .filter((character) => !ZERO_WIDTH.test(character))
    .join('');
}

/** Whether the tag character at `index` belongs to a subdivision flag: the black flag, tags, a cancel tag. */
function inFlagSequence(characters: readonly string[], index: number): boolean {
  let start = index;
  while (start > 0 && TAG.test(characters[start - 1])) start -= 1;
  if (characters[start - 1] !== BLACK_FLAG) return false;
  let end = index;
  while (end < characters.length - 1 && TAG.test(characters[end + 1])) end += 1;
  return characters[end] === CANCEL_TAG;
}

/**
 * Another person's text, split so that what could make it read as something else is drawn
 * visibly (QA M3, SEC-9): the direction controls, a direction mark in text with no right-to-left
 * letter, the invisible control characters, the tag block outside a flag, and a zero-width
 * character where it can spoof a name. Each becomes a
 * `hidden` segment carrying its escape (`\u{202e}`, as `biorouter crew` prints it) and the raw
 * character, so a surface that draws the escape can still copy the bytes that were sent.
 *
 * Unlike {@link stripHiddenCharacters}, nothing is removed and most format characters stay: the
 * zero-width joiners (every emoji sequence), the direction marks of right-to-left text, the soft
 * hyphen, and a zero-width space between Thai words. For display only; it never changes what is
 * stored or sent.
 */
export function revealHiddenCharacters(value: string): RevealedSegment[] {
  const characters = Array.from(value);
  const segments: RevealedSegment[] = [];
  const rightToLeft = RIGHT_TO_LEFT_LETTER.test(value);
  let text = '';
  characters.forEach((character, index) => {
    const hidden =
      DIRECTION_CONTROL.test(character) ||
      (DIRECTION_MARK.test(character) && !rightToLeft) ||
      (CONTROL.test(character) && !LAID_OUT_CONTROLS.has(character)) ||
      (TAG.test(character) && !inFlagSequence(characters, index)) ||
      (ZERO_WIDTH.test(character) &&
        zeroWidthSpoofs(characters, index, tokenAround(characters, index)));
    if (!hidden) {
      text += character;
      return;
    }
    if (text) segments.push({ kind: 'text', text });
    text = '';
    segments.push({
      kind: 'hidden',
      raw: character,
      escape: escapeOf(character),
      codePoint: codePointOf(character),
    });
  });
  if (text) segments.push({ kind: 'text', text });
  return segments;
}
