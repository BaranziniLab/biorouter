import { isMachineIdShaped } from './nameKey';

/**
 * Sanitizing names for display, and isolating them inside strings.
 *
 * The broker validates a display name when it is set (naming-design.md,
 * "Display-name validation") and, from S1, projects a sanitized `display_name`.
 * The renderer repeats the cheap half of that here because a daemon that
 * predates S1 forwards legacy nicknames verbatim — and a nickname was only ever
 * checked for being non-empty and at most 120 bytes, so it can carry a
 * right-to-left override, a zero-width joiner, or `(@alice)`.
 */

/**
 * Categories a name may never contain: controls, format characters (every bidi
 * override and isolate, zero-width space and joiner, BOM), private use, lone
 * surrogates, unassigned, line and paragraph separators, and every
 * `Default_Ignorable_Code_Point` (variation selectors, Hangul fillers, …).
 */
const REJECTED = /[\p{Cc}\p{Cf}\p{Co}\p{Cs}\p{Cn}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}]/gu;
const WHITE_SPACE_RUN = /\p{White_Space}+/gu;
const SPACE_RUN = / {2,}/g;
const HAS_LETTER_OR_NUMBER = /[\p{L}\p{N}]/u;
const ONLY_NUMBERS = /^[\p{N} ]+$/u;

/** A display name may be at most this many Unicode scalar values (naming design). */
export const DISPLAY_NAME_MAX_CHARS = 64;

/**
 * NFC, every `White_Space` run to one space, rejected characters removed,
 * trimmed. Non-strings become `''`.
 */
export function sanitizeDisplayText(raw: unknown): string {
  if (typeof raw !== 'string') return '';
  return raw
    .normalize('NFC')
    .replace(WHITE_SPACE_RUN, ' ')
    .replace(REJECTED, '')
    .replace(SPACE_RUN, ' ')
    .trim();
}

/**
 * Removes every character whose compatibility form contains `@` or `#` —
 * `@`, fullwidth `＠`, small `﹫`, and their `#` counterparts. A display name may
 * not carry them (naming design, SR5), because a nickname such as
 * `Alice (@alice)` would otherwise put a second, forged handle on screen.
 */
function stripHandleMarks(value: string): string {
  return Array.from(value)
    .filter((char) => !/[@#]/.test(char.normalize('NFKC')))
    .join('')
    .replace(SPACE_RUN, ' ')
    .trim();
}

/** A username as displayed: sanitized, and with no whitespace at all. `''` when unusable. */
export function sanitizeUsername(raw: unknown): string {
  return sanitizeDisplayText(raw).replace(/ /g, '');
}

/**
 * A person's name, if it is still a usable name after sanitizing; otherwise
 * `''`.
 *
 * Unusable means: nothing displayable left, longer than
 * {@link DISPLAY_NAME_MAX_CHARS}, no letter or digit, or — so no rendering of a
 * person can ever read as a machine reference — shaped like a UUID or a 64-hex
 * digest, or only digits (which reads as a numeric account ID).
 */
export function usableName(raw: unknown): string {
  const name = stripHandleMarks(sanitizeDisplayText(raw));
  if (!name) return '';
  if (Array.from(name).length > DISPLAY_NAME_MAX_CHARS) return '';
  if (!HAS_LETTER_OR_NUMBER.test(name)) return '';
  if (isMachineIdShaped(name) || ONLY_NUMBERS.test(name)) return '';
  return name;
}

/**
 * The name a person is shown by: the (projected or self-set) name when it is
 * usable ({@link usableName}), otherwise the username.
 */
export function personDisplayName(raw: unknown, username: string): string {
  return usableName(raw) || username;
}

/** Whether the display name adds nothing to the username (compared case-insensitively). */
export function displayNameIsUsername(displayName: string, username: string): boolean {
  return displayName.normalize('NFC').toLowerCase() === username.normalize('NFC').toLowerCase();
}

/**
 * Right-to-left scripts: Hebrew through Arabic Extended-A, the Hebrew and Arabic
 * presentation forms, and the two supplementary blocks that hold the remaining
 * right-to-left scripts (Hanifi Rohingya, Yezidi, Adlam, Mende Kikakui, …).
 */
const RIGHT_TO_LEFT =
  /[\u0590-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFC\u{10800}-\u{10FFF}\u{1E800}-\u{1EFFF}]/u;

/** First Strong Isolate and Pop Directional Isolate. */
const FSI = '\u2068';
const PDI = '\u2069';

/**
 * Wraps text in Unicode isolates (U+2068 … U+2069) when it contains a
 * right-to-left character, so a name inside a string — an aria-label, a toast,
 * a confirmation title — cannot reorder the words around it. In a rendered tree
 * `<bdi>` does this job; this is its equivalent for a plain string.
 *
 * Left-to-right text is returned unchanged, so ordinary labels stay plain,
 * comparable strings.
 */
export function isolate(text: string): string {
  return RIGHT_TO_LEFT.test(text) ? `${FSI}${text}${PDI}` : text;
}
