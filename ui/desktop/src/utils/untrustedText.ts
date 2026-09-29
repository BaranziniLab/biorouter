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
 * right-to-left mark inside `invoice.exe` lays the pieces out right to left. Nor does one between
 * two Latin letters or digits, in any text: a left-to-right mark there changes nothing, and a
 * right-to-left one splits the word, so `@cre{U+200F}w_bob` passed for `@crew_bob` in a message
 * that also held one Hebrew word.
 */
const DIRECTION_MARK = /^[\u{200E}\u{200F}\u{061C}]$/u;
/** A Latin letter, a digit or an underscore: what a direction mark has no business between. */
const ASCII_LETTER_OR_DIGIT = /^[A-Za-z0-9_]$/;
/** A letter of a script written right to left (a letter: the Arabic letter mark is Arabic too). */
const RIGHT_TO_LEFT_LETTER =
  /(?=\p{L})[\p{Script=Hebrew}\p{Script=Arabic}\p{Script=Syriac}\p{Script=Thaana}\p{Script=Nko}\p{Script=Samaritan}\p{Script=Mandaic}\p{Script=Adlam}\p{Script=Hanifi_Rohingya}]/u;
/** A control character (C0, DEL, C1): drawn as nothing, or as a box, unless it is a tab or a break. */
const CONTROL = /^\p{Cc}$/u;
const LAID_OUT_CONTROLS = new Set(['\t', '\n', '\r']);
/** The Unicode tag block, invisible outside a subdivision flag (England, Scotland, Wales). */
const TAG = /^[\u{E0000}-\u{E007F}]$/u;
const BLACK_FLAG = '\u{1F3F4}';
const CANCEL_TAG = '\u{E007F}';
/**
 * The tags a subdivision flag spells its region and subdivision in (`gbsct`): lower-case letters
 * and digits, three to seven of them (UTS #51, `unicode_subdivision_id`). The tag block mirrors
 * ASCII, so a longer run is text nobody can see and an agent reads.
 */
const FLAG_TAG = /^[\u{E0030}-\u{E0039}\u{E0061}-\u{E007A}]$/u;
const FLAG_TAGS_MIN = 3;
const FLAG_TAGS_MAX = 7;
/**
 * Every other character that draws nothing: the default-ignorable code points and the format
 * characters, by category rather than by list. That is the zero-width space, joiners and
 * non-joiner, the word joiner and the invisible operators (`U+2060..U+2064`), the deprecated
 * format characters (`U+206A..U+206F`), the byte-order mark, the soft hyphen, the combining
 * grapheme joiner, the variation selectors, the Mongolian vowel separator and free variation
 * selectors, the Hangul fillers and the unassigned default-ignorables. Any of them between two
 * letters of `@crew_bob` leaves it looking like `@crew_bob`.
 */
const INVISIBLE = /^[\p{Default_Ignorable_Code_Point}\p{Cf}]$/u;
/**
 * The format characters that do draw a glyph: Unicode's prepended concatenation marks (the Arabic
 * number and year signs, the Syriac abbreviation mark, the Kaithi number signs). They are
 * written before digits, which are often ASCII, and are no way to hide anything.
 */
const VISIBLE_FORMAT =
  /^[\u{0600}-\u{0605}\u{06DD}\u{070F}\u{0890}\u{0891}\u{08E2}\u{110BD}\u{110CD}]$/u;
/** Whether the text holds anything this step might draw differently, so plain text costs one scan. */
const MAYBE_HIDDEN = /[^\P{Cc}\t\n\r]|[\p{Default_Ignorable_Code_Point}\p{Cf}]/u;
/** Whitespace that separates tokens. Not JavaScript's `\s`, which counts `U+FEFF` as a space. */
const TOKEN_BREAK =
  /^[\t\n\v\f\r \u{00A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}]$/u;
/** A character a handle, address or file name is written in. */
const ASCII_WORD = /^[A-Za-z0-9_.@:/-]$/;
const ZERO_WIDTH_JOINER = '\u{200D}';
/** The variation selectors: `U+FE00..U+FE0F` and the ideographic supplement. */
const VARIATION_SELECTOR = /^[\u{FE00}-\u{FE0F}\u{E0100}-\u{E01EF}]$/u;
/** What an emoji zero-width-joiner sequence joins: a pictograph, after it a skin tone too. */
const PICTOGRAPH = /^\p{Extended_Pictographic}$/u;
const EMOJI_MODIFIER = /^\p{Emoji_Modifier}$/u;
/** What a variation selector legitimately follows: an emoji or a Han ideograph (not ASCII). */
const VARIATION_BASE = /^[\p{Emoji}\p{Ideographic}]$/u;
/** A keycap: `1`, `#` or `*`, the emoji presentation selector, then the combining keycap. */
const KEYCAP_BASE = /^[0-9#*]$/;
const COMBINING_KEYCAP = '\u{20E3}';

/**
 * How one character of a text is drawn, decided once: as it is (`plain`); always as its escape (an
 * embedding, override or isolate, `direction`; a control other than a tab or a break, `control`);
 * as its escape in text with no right-to-left letter (a direction `mark`); outside a subdivision
 * flag (a `tag`); or where it can spoof a name or hide text (another `invisible`).
 */
type Kind = 'plain' | 'direction' | 'mark' | 'control' | 'tag' | 'invisible';

function kindOf(character: string): Kind {
  const code = character.codePointAt(0) ?? 0;
  // Printable ASCII, most of any message, needs no test.
  if (code >= 0x20 && code < 0x7f) return 'plain';
  if (DIRECTION_CONTROL.test(character)) return 'direction';
  if (DIRECTION_MARK.test(character)) return 'mark';
  if (CONTROL.test(character)) return LAID_OUT_CONTROLS.has(character) ? 'plain' : 'control';
  if (TAG.test(character)) return 'tag';
  if (INVISIBLE.test(character) && !VISIBLE_FORMAT.test(character)) return 'invisible';
  return 'plain';
}

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
 * Whether the invisible character at `index` is part of an emoji or a Han ideograph, where it
 * belongs whatever surrounds it: a joiner between two pictographs (`U+1F469 U+200D U+1F52C`), a
 * variation selector right after an emoji or an ideograph (`U+2764 U+FE0F`), or the selector of a
 * keycap (`1 U+FE0F U+20E3`). `before` and `after` are the nearest drawn characters on each side.
 */
function partOfEmoji(
  characters: readonly string[],
  index: number,
  before: string,
  after: string
): boolean {
  const character = characters[index];
  if (character === ZERO_WIDTH_JOINER)
    return (PICTOGRAPH.test(before) || EMOJI_MODIFIER.test(before)) && PICTOGRAPH.test(after);
  if (!VARIATION_SELECTOR.test(character)) return false;
  const base = characters[index - 1] ?? '';
  if (KEYCAP_BASE.test(base)) return characters[index + 1] === COMBINING_KEYCAP;
  return (base.codePointAt(0) ?? 0) > 0x7f && VARIATION_BASE.test(base);
}

/**
 * Another person's text, split so that what could make it read as something else is drawn
 * visibly (QA M3, SEC-9): the direction controls, a direction mark in text with no right-to-left
 * letter or between two Latin letters or digits, the control characters, the tag block outside a
 * flag, and any other invisible character (a default-ignorable code point or a format character)
 * where it can spoof a name or hide text: beside a letter of a handle, an address or a file name
 * (skipping other hidden characters to find it), anywhere in a token that names somebody or
 * somewhere, or next to another invisible character outside an emoji. Each becomes a `hidden`
 * segment carrying its escape (`\u{202e}`, as `biorouter crew` prints it) and the raw character, so
 * a surface that draws the escape can still copy the bytes that were sent.
 *
 * Unlike {@link stripHiddenCharacters}, nothing is removed and what legitimate text needs stays:
 * the joiners and variation selectors of emoji and keycaps, the variation selectors of Han
 * ideographs, the direction marks of right-to-left text, and, between letters of other scripts,
 * Persian's non-joiner, the Indic joiners, Thai's zero-width space or a soft hyphen. For display
 * only; it never changes what is stored or sent.
 *
 * One pass over the text, whatever it holds: each character is classified once, the nearest drawn
 * character on each side, each tag run and each run of invisible characters are found in one sweep
 * apiece, and a token's name test runs once per token. A message is up to 64 KB of somebody else's
 * choosing, and this runs on the renderer's main thread as every row mounts.
 */
export function revealHiddenCharacters(value: string): RevealedSegment[] {
  return revealHiddenCharactersAcross([value])[0];
}

/**
 * Several texts drawn one after another on a line, split as {@link revealHiddenCharacters} splits
 * one text, and judged as that one text: a character's nearest drawn neighbours, its token and its
 * run are read across the edges between them. Returns one list of segments per text.
 *
 * This is for text a renderer splits into pieces that are drawn with nothing between them: a
 * markdown paragraph whose words are cut by emphasis or a link. Judged a piece at a time, a
 * zero-width space in a piece of its own had no neighbour and no token to name anything, so
 * `@crew_b*{U+200B}*ob` drew as `@crew_bob` with nothing shown. What the caller draws between two
 * pieces it passes as a piece of its own (a line break as `\n`), and ignores that piece's segments.
 *
 * Each text is split into characters on its own, so a lone surrogate at the end of one is never
 * joined to one at the start of the next into a character neither piece draws.
 */
export function revealHiddenCharactersAcross(texts: readonly string[]): RevealedSegment[][] {
  if (!texts.some((text) => MAYBE_HIDDEN.test(text)))
    return texts.map((text) => (text ? [{ kind: 'text', text }] : []));
  const characters: string[] = [];
  const starts: number[] = [];
  for (const text of texts) {
    starts.push(characters.length);
    for (const character of text) characters.push(character);
  }
  starts.push(characters.length);
  const rightToLeft = texts.some((text) => RIGHT_TO_LEFT_LETTER.test(text));
  const shown = hiddenCharacterFlags(characters, rightToLeft);
  return texts.map((_text, piece) =>
    segmentsOf(characters, shown, starts[piece], starts[piece + 1])
  );
}

/** The characters from `start` to `end`, as segments: runs drawn as they are, and hidden ones. */
function segmentsOf(
  characters: readonly string[],
  shown: Uint8Array,
  start: number,
  end: number
): RevealedSegment[] {
  const segments: RevealedSegment[] = [];
  let text = '';
  for (let index = start; index < end; index += 1) {
    const character = characters[index];
    if (shown[index] === 0) {
      text += character;
      continue;
    }
    if (text) segments.push({ kind: 'text', text });
    text = '';
    segments.push({
      kind: 'hidden',
      raw: character,
      escape: escapeOf(character),
      codePoint: codePointOf(character),
    });
  }
  if (text) segments.push({ kind: 'text', text });
  return segments;
}

/**
 * For each character, 1 when it is drawn as its escape: the rule {@link revealHiddenCharacters}
 * states. `rightToLeft`: whether the text holds a letter of a right-to-left script.
 */
function hiddenCharacterFlags(characters: readonly string[], rightToLeft: boolean): Uint8Array {
  const count = characters.length;
  const kinds = characters.map(kindOf);

  // The nearest drawn character before and after each position (hidden ones are looked through).
  const before = new Int32Array(count);
  let last = -1;
  for (let index = 0; index < count; index += 1) {
    before[index] = last;
    if (kinds[index] === 'plain') last = index;
  }
  const after = new Int32Array(count);
  last = count;
  for (let index = count - 1; index >= 0; index -= 1) {
    after[index] = last;
    if (kinds[index] === 'plain') last = index;
  }

  // Tag characters that belong to a subdivision flag: the black flag, the three to seven tags of
  // its code, a cancel tag.
  const inFlag = new Uint8Array(count);
  for (let index = 0; index < count; ) {
    if (kinds[index] !== 'tag') {
      index += 1;
      continue;
    }
    let end = index;
    while (end + 1 < count && kinds[end + 1] === 'tag') end += 1;
    const code = end - index;
    if (
      characters[index - 1] === BLACK_FLAG &&
      characters[end] === CANCEL_TAG &&
      code >= FLAG_TAGS_MIN &&
      code <= FLAG_TAGS_MAX &&
      characters.slice(index, end).every((tag) => FLAG_TAG.test(tag))
    )
      inFlag.fill(1, index, end + 1);
    index = end + 1;
  }

  // Invisible characters that stand next to another: no script's writing puts two together outside
  // an emoji, and a run of them is how text is hidden in text (one bit per character, or a byte per
  // variation selector after an emoji), for an agent to read where no person can.
  const inRun = new Uint8Array(count);
  for (let index = 0; index < count; ) {
    if (kinds[index] !== 'invisible') {
      index += 1;
      continue;
    }
    let end = index;
    while (end + 1 < count && kinds[end + 1] === 'invisible') end += 1;
    if (end > index) inRun.fill(1, index, end + 1);
    index = end + 1;
  }

  // Tokens: runs between whitespace. Whether one names something is asked of its drawn
  // characters, once, and only for a token that holds an invisible character.
  const token = new Int32Array(count);
  const tokenStart: number[] = [];
  for (let index = 0, current = -1; index < count; index += 1) {
    if (TOKEN_BREAK.test(characters[index])) {
      token[index] = -1;
      current = -1;
      continue;
    }
    if (current < 0) {
      current = tokenStart.length;
      tokenStart.push(index);
    }
    token[index] = current;
  }
  const tokenNames: (boolean | undefined)[] = [];
  const tokenNamesSomething = (id: number): boolean => {
    const known = tokenNames[id];
    if (known !== undefined) return known;
    let drawn = '';
    for (let index = tokenStart[id]; index < count && token[index] === id; index += 1)
      if (kinds[index] === 'plain') drawn += characters[index];
    const answer = namesSomething(drawn);
    tokenNames[id] = answer;
    return answer;
  };

  const shows = (index: number): boolean => {
    switch (kinds[index]) {
      case 'plain':
        return false;
      case 'direction':
      case 'control':
        return true;
      case 'mark':
        return (
          !rightToLeft ||
          (ASCII_LETTER_OR_DIGIT.test(characters[before[index]] ?? '') &&
            ASCII_LETTER_OR_DIGIT.test(characters[after[index]] ?? ''))
        );
      case 'tag':
        return inFlag[index] === 0;
      case 'invisible': {
        const previous = characters[before[index]] ?? '';
        const next = characters[after[index]] ?? '';
        if (partOfEmoji(characters, index, previous, next)) return false;
        if (inRun[index] === 1) return true;
        if (ASCII_WORD.test(previous) || ASCII_WORD.test(next)) return true;
        return token[index] >= 0 && tokenNamesSomething(token[index]);
      }
    }
  };

  const shown = new Uint8Array(count);
  for (let index = 0; index < count; index += 1) if (shows(index)) shown[index] = 1;
  return shown;
}
