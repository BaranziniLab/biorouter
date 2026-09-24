import { isMachineIdShaped } from '../identity';
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
 * `identity/institution.ts`'s `INSTITUTION_ID_PATTERN` (`[a-z0-9][a-z0-9_-]{0,63}`) has exactly this
 * defect, which is why the institution fields here use their own copy of the same rule.
 */

/** The HTML `pattern` for an institution ID: `is_canonical_institution_id`, `v`-flag safe. */
export const INSTITUTION_FIELD_PATTERN = '[a-z0-9][a-z0-9_\\-]{0,63}';
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
