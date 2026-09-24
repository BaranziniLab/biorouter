import * as React from 'react';
import * as AvatarPrimitive from '@radix-ui/react-avatar';

import { cn } from '../../utils';

export type AvatarSize = 20 | 24 | 32;
export type AvatarShape = 'circle' | 'square';

export interface AvatarProps {
  /**
   * What the person chose to be shown as — an emoji or their own initials
   * (`profile.update {avatar}`). Wins over derived initials when non-blank, and
   * is clamped to two characters so a long choice cannot overflow the tile.
   */
  fallback?: string | null;
  /** The display name the initials are taken from. */
  name?: string | null;
  /** The username, used when the display name yields no initials. */
  username?: string | null;
  /** An image, when one exists. The fallback shows until it loads, and if it fails. */
  src?: string;
  size?: AvatarSize;
  /** `circle` for people; `square` for agents and objects. */
  shape?: AvatarShape;
  /** A glyph in place of letters — an agent's `Bot`. Sized by the avatar. */
  icon?: React.ReactNode;
  /**
   * A 2px ring in the surface colour, for avatars that overlap in a stack. The
   * ring paints `--biorouter-avatar-ring`, which a stack sets to its own ground
   * (it defaults to the canvas).
   */
  ring?: boolean;
  /**
   * The avatar's accessible name when it stands alone. Without one it is
   * decorative (`aria-hidden`): beside a person's name it would only repeat it.
   */
  label?: string;
  /** Layout only. */
  className?: string;
}

/** The first letter (with its combining marks) of a word, or ''. */
function firstLetter(word: string): string {
  return word.match(/[\p{L}\p{N}]\p{M}*/u)?.[0] ?? '';
}

// `Intl.Segmenter` is ES2022 and this project's `lib` is ES2020, so the one
// method used is typed here rather than widening `lib` for every file.
type GraphemeSegmenter = { segment(text: string): Iterable<{ segment: string }> };
type GraphemeSegmenterConstructor = new (
  locales: undefined,
  options: { granularity: 'grapheme' }
) => GraphemeSegmenter;

/** Split into user-perceived characters, so an emoji with a joiner stays whole. */
function graphemes(text: string): string[] {
  const Segmenter = (Intl as unknown as { Segmenter?: GraphemeSegmenterConstructor }).Segmenter;
  if (Segmenter) {
    return Array.from(new Segmenter(undefined, { granularity: 'grapheme' }).segment(text)).map(
      (part) => part.segment
    );
  }
  return Array.from(text);
}

/** What a handle joins its parts with: `crew_alice`, `j.smith`, `lab-bob`. */
const HANDLE_SEPARATOR = /[_.-]+/u;
const WORD_SEPARATOR = /\s+/u;
const LEADING_LETTER = /^\p{L}/u;
const LETTER_OR_DIGIT = /[\p{L}\p{N}]\p{M}*/gu;
const HANDLE_MARK = /[@#]/u;
/** `@` and its compatibility forms, fullwidth `＠` and small `﹫`. */
const AT_SIGN = /[@\uFE6B\uFF20]/u;
const LEADING_AT_SIGNS = /^[@\uFE6B\uFF20]+/u;

const folded = (text: string) => text.normalize('NFC').toLowerCase();

/** `text` without `@` and `#` in any compatibility form (`＠`, `﹫`, …). */
function withoutHandleMarks(text: string): string {
  return Array.from(text)
    .filter((char) => !HANDLE_MARK.test(char.normalize('NFKC')))
    .join('');
}

/**
 * Whether a display name only repeats the username — that is, the person has
 * not chosen one (T-31). A new member's nickname IS their username, and the
 * daemon and the renderer both remove `@` and `#` from a nickname, so an SSSD
 * account `bob@ad.ucsf.edu` that never set a name is shown as `bobad.ucsf.edu`.
 * Either shape counts, case aside. `personLabel` asks the same question of the
 * same two shapes (`displayNameRepeatsUsername`), and a test runs both off one
 * projection so they cannot drift.
 */
function repeatsUsername(name: string, username: string): boolean {
  if (!username) return false;
  const shown = folded(name);
  return shown === folded(username) || shown === folded(withoutHandleMarks(username));
}

/**
 * The account part of a username: what comes before an SSSD realm
 * (`bob@ad.ucsf.edu` → `bob`). A realm is never read — every member of one
 * shares it, and `ad.ucsf.edu` would give them all an "E".
 */
function accountName(username: string): string {
  return username.replace(LEADING_AT_SIGNS, '').split(AT_SIGN)[0];
}

/**
 * The letter a separated handle is known by, or `null` when the word is not one
 * (fewer than two parts carry a letter or digit).
 *
 * It is read from the LAST part, because that is where a shared prefix is not:
 * server accounts are often issued as `crew_alice`, `crew_bob`, `crew_carol`,
 * and their first letters are all "C" — which is how every avatar in a
 * workspace came to read "C". A part that starts with a letter wins over a
 * trailing number, so `alice_2` is "a", not "2".
 */
function separatedHandleLetter(word: string): string | null {
  const letters = word
    .split(HANDLE_SEPARATOR)
    .map(firstLetter)
    .filter((letter) => letter.length > 0);
  if (letters.length < 2) return null;
  const alphabetic = letters.filter((letter) => LEADING_LETTER.test(letter));
  const from = alphabetic.length > 0 ? alphabetic : letters;
  return from[from.length - 1];
}

/**
 * THE avatar fallback rule (L16 — the old view had two that disagreed).
 *
 * - A display name the person chose is read as written, whatever joins its
 *   parts. Two or more words give the first letters of the first two ("Alice
 *   Chen" → "AC", "Mary-Jane Watson" → "MW"); one word gives its first letter
 *   ("Jean-Luc" → "J", "A.J." → "A", "st.john" → "S").
 * - A display name that only repeats the username is not a choice, so the
 *   username is read instead — its account part, never an SSSD realm
 *   (`bob@ad.ucsf.edu` → "bob"). A handle joined by `_`, `.` or `-` gives the
 *   first letter of its last part ("crew_alice" → "A", "crew_bob@ad.ucsf.edu"
 *   → "B"); anything else its first letter ("bob@ad.ucsf.edu" → "B").
 * - With no letter in the display name at all, the username is read the same
 *   way, except that an unseparated one gives its first two letters ("bob" →
 *   "BO").
 *
 * Upper-cased; '' when neither yields a letter.
 */
export function avatarInitials(name?: string | null, username?: string | null): string {
  const shown = (name ?? '').normalize('NFC').trim();
  const handle = (username ?? '').normalize('NFC').trim();
  const words = shown.split(WORD_SEPARATOR).filter((word) => firstLetter(word).length > 0);

  if (words.length > 0 && !repeatsUsername(shown, handle)) {
    const initials =
      words.length >= 2 ? firstLetter(words[0]) + firstLetter(words[1]) : firstLetter(words[0]);
    return initials.toLocaleUpperCase();
  }

  const account = accountName(handle);
  const separated = separatedHandleLetter(account);
  if (separated) return separated.toLocaleUpperCase();
  // A name that repeats the username reads as one letter, as any one-word name
  // does; no name at all reads as the username's first two.
  const fromUsername = (account.match(LETTER_OR_DIGIT) ?? [])
    .slice(0, words.length > 0 ? 1 : 2)
    .join('');
  return (fromUsername || (words.length > 0 ? firstLetter(words[0]) : '')).toLocaleUpperCase();
}

/**
 * The one identity tile, on `@radix-ui/react-avatar`: 20, 24 or 32px, a circle
 * for people and a square for agents and objects, `--background-medium` ground
 * with `--text-muted` ink. Geometry, ground, ring and type are authored CSS
 * (`.biorouter-avatar` in `main.css`) keyed on `data-size` / `data-shape` /
 * `data-ring`, because a newly written utility can silently fail to generate
 * under `BIOROUTER_NO_HMR`.
 */
export function Avatar({
  fallback,
  name,
  username,
  src,
  size = 32,
  shape = 'circle',
  icon,
  ring = false,
  label,
  className,
}: AvatarProps) {
  const chosen = (fallback ?? '').trim();
  const text = chosen ? graphemes(chosen).slice(0, 2).join('') : avatarInitials(name, username);
  const named = typeof label === 'string' && label.trim().length > 0;

  return (
    <AvatarPrimitive.Root
      data-slot="avatar"
      data-size={size}
      data-shape={shape}
      data-ring={ring ? 'true' : undefined}
      className={cn('biorouter-avatar', className)}
      {...(named ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true })}
    >
      {src ? <AvatarPrimitive.Image className="biorouter-avatar-image" src={src} alt="" /> : null}
      <AvatarPrimitive.Fallback className="biorouter-avatar-fallback">
        {icon ?? text}
      </AvatarPrimitive.Fallback>
    </AvatarPrimitive.Root>
  );
}
