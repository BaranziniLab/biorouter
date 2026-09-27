import { deviceCodeCopy } from './copy';

/**
 * Reading a device code a person pastes or types (naming design, "The device code").
 *
 * The code is 16 Crockford base-32 characters, computed by the JOINER's own daemon and handed to
 * the host by a person. The host's screen only ever holds what the host typed: nothing here
 * produces a code, it only normalizes one — spaces, hyphens and invisible characters removed,
 * upper-cased, `I` and `L` read as `1`, `O` as `0` — exactly as
 * `biorouter_crew::normalize_device_code` does, so the host sends what the broker compares.
 * `U` is never part of a code and is refused rather than repaired.
 */

export const DEVICE_CODE_LENGTH = 16;

/** Crockford base 32: no I, L, O or U. */
const ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';

/** What a person may type between groups, and what a paste may carry invisibly. */
const SEPARATOR = /[\p{White_Space}\p{Pd}\p{Default_Ignorable_Code_Point}]/u;

function isSeparator(char: string): boolean {
  return SEPARATOR.test(char);
}

/**
 * The code's characters as the broker will compare them: separators dropped, upper-cased, the
 * Crockford lookalikes mapped. Anything else (a `U`, a stray `!`) is KEPT, upper-cased, so the
 * field shows the person what they typed and `deviceCodeProblem` can say what is wrong with it.
 */
export function normalizeDeviceCodeInput(raw: string): string {
  let code = '';
  for (const char of raw) {
    if (isSeparator(char)) continue;
    const upper = char.toUpperCase();
    code += upper === 'I' || upper === 'L' ? '1' : upper === 'O' ? '0' : upper;
  }
  return code;
}

/** Why a normalized code is not a device code yet, in the shared library's words; null when it is one. */
export function deviceCodeProblem(code: string): string | null {
  const chars = Array.from(code);
  if (chars.includes('U')) return deviceCodeCopy.containsU;
  if (chars.some((char) => !ALPHABET.includes(char))) return deviceCodeCopy.invalidCharacter;
  if (chars.length !== DEVICE_CODE_LENGTH) return deviceCodeCopy.wrongLength;
  return null;
}

/** `7QK2M9XA3JTPWZ4D` → `7QK2-M9XA-3JTP-WZ4D`. Display only; the ungrouped code is what is sent. */
export function groupDeviceCodeInput(code: string): string {
  const chars = Array.from(code);
  const groups: string[] = [];
  for (let index = 0; index < chars.length; index += 4) {
    groups.push(chars.slice(index, index + 4).join(''));
  }
  return groups.join('-');
}

/** How many code characters (not separators) come before `position` in `text`. */
export function codeCharactersBefore(text: string, position: number): number {
  let count = 0;
  let offset = 0;
  for (const char of text) {
    if (offset >= position) break;
    offset += char.length;
    if (!isSeparator(char)) count += 1;
  }
  return count;
}

/** The caret offset in a grouped code just after its `count`th code character. */
export function caretAfterCodeCharacters(grouped: string, count: number): number {
  if (count <= 0) return 0;
  let seen = 0;
  let offset = 0;
  for (const char of grouped) {
    offset += char.length;
    if (char !== '-') seen += 1;
    if (seen === count) return offset;
  }
  return grouped.length;
}
