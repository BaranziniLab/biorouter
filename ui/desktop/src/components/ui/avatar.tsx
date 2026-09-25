import * as React from 'react';
import * as AvatarPrimitive from '@radix-ui/react-avatar';

import { cn } from '../../utils';

export type AvatarSize = 20 | 24 | 32;
export type AvatarShape = 'circle' | 'square';

export interface AvatarProps {
  /**
   * What the person chose to be shown as — an emoji or their own initials
   * (`profile.update {avatar}`). Wins over derived initials when non-blank, and
   * is clamped to what the tile holds ({@link avatarTextLimit}: two characters
   * at 28px and above, the first one below) so a long choice cannot overflow it.
   */
  fallback?: string | null;
  /** The display name the initials are taken from. */
  name?: string | null;
  /**
   * The canonical username. It picks a person's hue ({@link avatarHue}) and is
   * read for initials when the display name yields none.
   */
  username?: string | null;
  /** An image, when one exists. The fallback shows until it loads, and if it fails. */
  src?: string;
  /**
   * 20, 24 or 32px. The size never changes a derived initial: it is one letter
   * everywhere, so a person reads the same in a header stack, a member row and a
   * message (Q3-62). It only clamps a chosen avatar ({@link avatarTextLimit}).
   */
  size?: AvatarSize;
  /**
   * `circle` for people; `square` for agents and objects. Only a circle with a
   * username takes a person's hue; a square keeps the neutral tile.
   */
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
 * THE avatar fallback rule (L16 — the old view had two that disagreed): ONE
 * letter, at every size (Q3-62).
 *
 * It used to give a two-word name two letters, which a 20px tile could not hold
 * (Q2-70), so the header stack showed "H" and the member rows and messages
 * "HI" — one person, two identities side by side. A person now reads the same
 * everywhere, as they do in Slack.
 *
 * - A display name the person chose is read as written: the first letter of
 *   its first word, whatever joins that word's parts ("Alice Chen" → "A",
 *   "Mary-Jane Watson" → "M", "Jean-Luc" → "J", "A.J." → "A", "st.john" → "S").
 * - A display name that only repeats the username is not a choice, so the
 *   username is read instead — its account part, never an SSSD realm
 *   (`bob@ad.ucsf.edu` → "bob"). A handle joined by `_`, `.` or `-` gives the
 *   first letter of its last part ("crew_alice" → "A", "crew_bob@ad.ucsf.edu"
 *   → "B"); anything else its first letter ("bob@ad.ucsf.edu" → "B").
 * - With no letter in the display name at all, the username is read the same
 *   way ("bob" → "B", "crew_bob" → "B").
 *
 * Upper-cased, and still one character when upper-casing spells a letter as
 * two ("ﬁ" → "FI" → "F"); '' when neither yields a letter.
 */
export function avatarInitials(name?: string | null, username?: string | null): string {
  const shown = (name ?? '').normalize('NFC').trim();
  const handle = (username ?? '').normalize('NFC').trim();
  const words = shown.split(WORD_SEPARATOR).filter((word) => firstLetter(word).length > 0);
  const chosen = words.length > 0 && !repeatsUsername(shown, handle);

  const account = accountName(handle);
  const letter = chosen
    ? firstLetter(words[0])
    : separatedHandleLetter(account) || firstLetter(account) || firstLetter(words[0] ?? '');
  return graphemes(letter.toLocaleUpperCase())[0] ?? '';
}

/**
 * How many person hues there are (D-AVATAR). Every theme family declares the
 * pairs `--avatar-hue-{1..8}-bg` / `-fg`, and `main.css` paints them on
 * `.biorouter-avatar[data-hue='N']`; `scripts/lib/theme-contract.mjs` carries
 * the same count (`AVATAR_HUE_COUNT`), and a test holds the two equal.
 */
export const AVATAR_HUE_COUNT = 8;

/**
 * The form of a username the hue is computed from: NFC, trimmed, without a
 * leading `@` in any compatibility form, and case-folded. Folding makes an
 * account a case-insensitive directory (SSSD) reports as `Bob@AD.UCSF.EDU` in
 * one place and `bob@ad.ucsf.edu` in another keep one colour; `toLowerCase` is
 * the locale-independent Unicode mapping, so every device folds alike. The
 * realm is kept: it is part of who the account is.
 */
function canonicalUsername(username: string): string {
  return username.normalize('NFC').trim().replace(LEADING_AT_SIGNS, '').toLowerCase();
}

/**
 * A person's hue, 1–8, or `null` with no usable username (D-AVATAR, carol F4:
 * a row of identical grey tiles made every "who is this" a read).
 *
 * - **From the canonical `@username`, never the display name.** A username is
 *   the one name nobody chooses, so a self-chosen display name cannot borrow
 *   another person's colour. The hue is a recognition aid, not a proof: the
 *   `@username` stays on screen beside it.
 * - **Stable across sessions, devices and builds.** A pure function of the
 *   username's UTF-8 bytes — no directory, no order, no seed — so a person has
 *   one colour for every viewer. Two people can share a hue (eight hues); the
 *   initials and the `@username` still tell them apart.
 * - **Every byte reaches the result.** 32-bit FNV-1a, XOR-folded down to three
 *   bits. Taking `hash % 8` instead would read only the low three bits of each
 *   byte (FNV's multiplier is odd), so `crew_alice` and `crew_ilice` — `a` and
 *   `i` share them — would always match.
 */
export function avatarHue(username?: string | null): number | null {
  const canonical = canonicalUsername(username ?? '');
  if (!canonical) return null;
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(canonical)) {
    hash = Math.imul(hash ^ byte, 0x01000193) >>> 0;
  }
  hash = (hash >>> 16) ^ (hash & 0xffff);
  hash = (hash >>> 8) ^ (hash & 0xff);
  hash = (hash >>> 4) ^ (hash & 0xf);
  hash = (hash >>> 3) ^ (hash & 0x7);
  return (hash & 0x7) + 1;
}

/** The smallest tile that shows a chosen pair in full (Q3-62). */
export const AVATAR_PAIR_MIN_SIZE = 28;

/**
 * How many characters of a CHOSEN avatar a tile of this size shows: both at
 * {@link AVATAR_PAIR_MIN_SIZE} (28px) and above, the first below. Two 11px
 * capitals are ~15px wide, which a 20px circle cuts at its edges and a member
 * stack overlaps by 4px (Q2-70), and a 24px member row showed "HI" beside a
 * 20px header's "H" (Q3-62). A derived initial is one letter at every size
 * ({@link avatarInitials}) and is not clamped here.
 */
export function avatarTextLimit(size: number): 1 | 2 {
  return size >= AVATAR_PAIR_MIN_SIZE ? 2 : 1;
}

/**
 * The one identity tile, on `@radix-ui/react-avatar`: 20, 24 or 32px, a circle
 * for people and a square for agents and objects. A person with a username
 * wears their hue pair (`data-hue`, {@link avatarHue}); anything else keeps the
 * neutral `--background-medium` ground with `--text-muted` ink. Geometry,
 * ground, hue, ring and type are authored CSS (`.biorouter-avatar` in
 * `main.css`) keyed on `data-size` / `data-shape` / `data-hue` / `data-ring`,
 * because a newly written utility can silently fail to generate under
 * `BIOROUTER_NO_HMR`.
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
  const text = chosen
    ? graphemes(chosen).slice(0, avatarTextLimit(size)).join('')
    : avatarInitials(name, username);
  const hue = shape === 'circle' && !icon ? avatarHue(username) : null;
  const named = typeof label === 'string' && label.trim().length > 0;

  return (
    <AvatarPrimitive.Root
      data-slot="avatar"
      data-size={size}
      data-shape={shape}
      data-hue={hue ?? undefined}
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
