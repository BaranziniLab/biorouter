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

/**
 * THE avatar fallback rule (L16 — the old view had two that disagreed): the
 * first letters of up to two words of the display name, else the first two
 * letters of the username. Upper-cased; '' when neither yields a letter.
 *
 * "Alice Chen" → "AC" · "alice" → "A" · display name "🧬" with username
 * "bob" → "BO".
 */
export function avatarInitials(name?: string | null, username?: string | null): string {
  const words = (name ?? '').normalize('NFC').trim().split(/\s+/u);
  const fromName = words
    .map(firstLetter)
    .filter((letter) => letter.length > 0)
    .slice(0, 2)
    .join('');
  if (fromName) return fromName.toLocaleUpperCase();

  const fromUsername = (username ?? '')
    .normalize('NFC')
    .match(/[\p{L}\p{N}]\p{M}*/gu)
    ?.slice(0, 2)
    .join('');
  return (fromUsername ?? '').toLocaleUpperCase();
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
