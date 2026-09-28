import { isMachineIdShaped, nameKey } from '../identity';
import { createTeamCopy, nameRuleCopy } from './copy';

/**
 * The client half of the naming rules a create or rename dialog needs (naming design, "Validation
 * per kind"). The broker decides; this previews and catches the obvious before a round trip.
 *
 * The channel slug is canonicalized exactly as `biorouter_crew::canonical_channel_name` does
 * (NFKC, trim, one leading `#` dropped, lower-cased, NFKC again, runs of white space, `.` and `-`
 * become one `-`, edge separators dropped), so "Will be created as #…" shows the name the broker
 * will store. The checks after it are a subset — no UTS #39 identifier status or script mixing,
 * which only the broker's tables can judge — so a name that passes here can still be refused, and
 * the refusal is shown as the broker words it.
 */

const CHANNEL_SEPARATOR = /[\p{White_Space}.-]+/u;
const EDGE_WHITE_SPACE = /^\p{White_Space}+|\p{White_Space}+$/gu;
/** A selector character, or anything whose compatibility form is one (fullwidth `＠`). */
const SELECTOR_CHARACTER = /[@#/:]/;
const CHANNEL_CHARACTER = /^[\p{L}\p{M}\p{Nd}_-]$/u;
const LETTER_OR_DIGIT = /^[\p{L}\p{Nd}]$/u;

export const CHANNEL_NAME_MAX_CHARS = 80;

/*
 * ⚠ HTML `pattern` attributes are compiled with the `v` flag (Chromium since 112, and jsdom), and
 * under `v` a bare `-` inside a character class is a SYNTAX ERROR — the browser then ignores the
 * pattern silently and the field accepts anything. So every `-` in a class below is escaped.
 */

/**
 * The HTML `pattern` for an institution ID: `is_canonical_institution_id`, `v`-flag safe. It is
 * `identity/institution.ts`'s `INSTITUTION_ID_PATTERN` under the name the dialogs import — one
 * rule, so the dialog fields, the privacy popover and onboarding cannot drift apart again.
 */
export { INSTITUTION_ID_PATTERN as INSTITUTION_FIELD_PATTERN } from '../identity';
/** The HTML `pattern` for a workspace name (naming design, "Workspace name"). */
export const WORKSPACE_NAME_PATTERN = '[a-z0-9](?:[a-z0-9\\-]{0,38}[a-z0-9])?';
/** The HTML `pattern` for an absolute path on the server. */
export const ABSOLUTE_PATH_PATTERN = '/.*';

/** The slug the broker will store for a typed channel name, or `''` when nothing is left. */
export function channelSlugPreview(raw: string): string {
  const compatible = raw.normalize('NFKC').replace(EDGE_WHITE_SPACE, '');
  const withoutHash = compatible.startsWith('#') ? compatible.slice(1) : compatible;
  const lowered = withoutHash.toLowerCase().normalize('NFKC');
  return lowered.split(CHANNEL_SEPARATOR).filter(Boolean).join('-');
}

/** Why a slug would be refused, in the shared library's words; null when this side sees no problem. */
export function channelSlugProblem(slug: string): string | null {
  const chars = Array.from(slug);
  if (chars.length === 0) return nameRuleCopy.channelEmpty;
  if (chars.length > CHANNEL_NAME_MAX_CHARS || new TextEncoder().encode(slug).length > 120)
    return nameRuleCopy.channelTooLong;
  if (chars.some((char) => SELECTOR_CHARACTER.test(char.normalize('NFKC'))))
    return nameRuleCopy.channelReserved;
  if (
    chars.some(
      (char) =>
        !CHANNEL_CHARACTER.test(char) || (/\p{L}/u.test(char) && char.toLowerCase() !== char)
    )
  )
    return nameRuleCopy.channelDisallowed;
  if (!LETTER_OR_DIGIT.test(chars[0])) return nameRuleCopy.channelStart;
  if (isMachineIdShaped(slug)) return nameRuleCopy.channelLooksLikeId;
  return null;
}

/**
 * The scripts `mixesScripts` can tell apart. A letter in none of them is not judged: this side can
 * only ever be less strict than the broker, never refuse or hide what it would accept for a
 * script it does not know.
 */
const SCRIPTS = [
  'Latin',
  'Cyrillic',
  'Greek',
  'Armenian',
  'Georgian',
  'Hebrew',
  'Arabic',
  'Syriac',
  'Thaana',
  'Devanagari',
  'Bengali',
  'Gurmukhi',
  'Gujarati',
  'Oriya',
  'Tamil',
  'Telugu',
  'Kannada',
  'Malayalam',
  'Sinhala',
  'Thai',
  'Lao',
  'Tibetan',
  'Myanmar',
  'Khmer',
  'Mongolian',
  'Ethiopic',
  'Cherokee',
  'Han',
  'Hiragana',
  'Katakana',
  'Bopomofo',
  'Hangul',
] as const;
const SCRIPT_TESTS = SCRIPTS.map(
  (script) => [script, new RegExp(`^\\p{Script_Extensions=${script}}$`, 'u')] as const
);
/** The script sets UTS #39 Highly Restrictive allows together, beside one script alone. */
const HIGHLY_RESTRICTIVE_SETS: readonly (readonly string[])[] = [
  ['Latin', 'Han', 'Hiragana', 'Katakana'],
  ['Latin', 'Han', 'Bopomofo'],
  ['Latin', 'Han', 'Hangul'],
];

/**
 * Whether a name mixes writing systems beyond UTS #39 Highly Restrictive (`mеthods` with a
 * Cyrillic `е`), as `restriction_level_ok` in `biorouter_crew::names` judges it: the letters and
 * digits (ASCII punctuation and spaces are not scored) share one script, or all fall in Latin with
 * Han and Japanese kana, Latin with Han and Bopomofo, or Latin with Han and Hangul. Common and
 * Inherited characters (digits, most marks) belong to every script.
 *
 * Display only: a name this calls mixed is not previewed, and the broker's refusal still decides.
 */
export function mixesScripts(name: string): boolean {
  const sets: Set<string>[] = [];
  for (const char of name) {
    if ((char.codePointAt(0) ?? 0) < 0x80 && !/^[A-Za-z0-9]$/.test(char)) continue;
    const scripts = new Set(
      SCRIPT_TESTS.filter(([, test]) => test.test(char)).map(([script]) => script)
    );
    // Common, Inherited or a script this list does not know: nothing to judge by.
    if (scripts.size === 0) continue;
    sets.push(scripts);
  }
  if (sets.length < 2) return false;
  const shared = sets.reduce(
    (common, scripts) => new Set([...common].filter((script) => scripts.has(script)))
  );
  if (shared.size > 0) return false;
  return !HIGHLY_RESTRICTIVE_SETS.some((allowed) =>
    sets.every((scripts) => allowed.some((script) => scripts.has(script)))
  );
}

/**
 * Whether a channel this team already has, and the viewer can see, holds a name that collides with
 * `slug` (`names_collide` in the broker, less its confusable skeleton, which only the broker's
 * tables can compute). `renaming` is the channel being renamed, which may keep its own name in
 * another case. A name taken by a channel the viewer cannot see is refused by the broker in the
 * same words, so saying it early here tells nothing the refusal would not.
 */
export function channelNameTaken(
  channels: readonly {
    id: string;
    team_id: string;
    name?: string | null;
    handle?: string | null;
  }[],
  teamId: string,
  slug: string,
  renaming?: string
): boolean {
  const key = nameKey(slug);
  if (!key) return false;
  return channels.some(
    (channel) =>
      channel.team_id === teamId &&
      channel.id !== renaming &&
      [channel.handle, channel.name].some(
        (name) => typeof name === 'string' && name !== '' && nameKey(name) === key
      )
  );
}

/** The one team-name rule worth checking before a round trip: no selector characters. */
export function teamNameProblem(raw: string): string | null {
  return Array.from(raw).some((char) => SELECTOR_CHARACTER.test(char.normalize('NFKC')))
    ? createTeamCopy.reservedCharacter
    : null;
}

const WORKSPACE_NAME = new RegExp(`^${WORKSPACE_NAME_PATTERN}$`);

export function workspaceNameProblem(name: string): string | null {
  return WORKSPACE_NAME.test(name) && !isMachineIdShaped(name)
    ? null
    : nameRuleCopy.workspacePattern;
}
