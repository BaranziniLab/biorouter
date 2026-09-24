import { isStaleDaemon, STALE_DAEMON_MESSAGE } from '../api/errors';
import { dialogErrorCopy, inviteCopy, letInCopy, nameRuleCopy } from './copy';

/**
 * Turning a refusal into the words a dialog shows.
 *
 * The daemon forwards a broker refusal as its text, which the broker writes `code: sentence`
 * (`bail!("forbidden: team owner required")`, `NameError::wire`). Most refusals are shown exactly
 * as written, in their own text node, because the daemon's words are the ones support can search
 * for. Only the refusals the copy deck gives a sentence of its own are rewritten, and only when
 * their code is recognised: anything else — including a code this renderer has never seen — is
 * shown verbatim, never guessed at.
 */

export interface BrokerRefusal {
  /** The broker's code, e.g. `name_invalid`; null when the text carries none. */
  code: string | null;
  /** The text after the code, or the whole text when there is none. */
  sentence: string;
  /** The text as the daemon sent it. */
  raw: string;
}

const CODED = /^([a-z][a-z0-9_]*): ([\s\S]+)$/;

export function parseRefusal(message: string): BrokerRefusal {
  const match = CODED.exec(message.trim());
  return match
    ? { code: match[1], sentence: match[2].trim(), raw: message }
    : { code: null, sentence: message.trim(), raw: message };
}

/** Codes whose sentence part the broker writes for a person, so the code prefix can go. */
const SENTENCE_CODES = new Set(['name_invalid', 'device_code_invalid']);
/** A name some object the viewer may not even see already holds (naming design D5). */
const NAME_TAKEN_CODES = new Set(['name_conflict', 'name_taken']);

/** The words for a refusal no dialog rewrites: the sentence alone for a person-written code, else verbatim. */
export function refusalText(message: string): string {
  const refusal = parseRefusal(message);
  if (!refusal.sentence) return dialogErrorCopy.fallback;
  return refusal.code && SENTENCE_CODES.has(refusal.code) ? refusal.sentence : refusal.raw.trim();
}

/** A create or rename refusal. A taken name reads the same whether or not its holder is visible. */
export function nameRefusalText(message: string, kind: 'team' | 'channel' | 'workspace'): string {
  const refusal = parseRefusal(message);
  if (refusal.code && NAME_TAKEN_CODES.has(refusal.code)) {
    if (kind === 'team') return nameRuleCopy.teamTaken;
    if (kind === 'channel') return nameRuleCopy.channelTaken;
  }
  return refusalText(message);
}

/** Whether a create or rename refusal is about the name itself, so it belongs on the name field. */
export function isNameRefusal(message: string): boolean {
  const code = parseRefusal(message).code;
  return code !== null && (NAME_TAKEN_CODES.has(code) || code === 'name_invalid');
}

const NO_ACCOUNT_CODES = new Set([
  'unknown_account',
  'account_not_found',
  'no_such_account',
  'unknown_user',
]);
const ALREADY_MEMBER_CODES = new Set(['already_member', 'identity_member']);
const CANONICAL_HINT = /(?:invite|did you mean)\s+@([^\s;:,?!]+)/i;

export interface InviteRefusal {
  text: string;
  /** The broker says the account is already a member: offer "Add another device". */
  alreadyMember: boolean;
}

/**
 * An `enrollment.invite {username}` refusal (naming design, "Canonicalizing a typed name"). The
 * account's canonical spelling, when the broker names one, is offered back — a case-only or alias
 * difference is never accepted silently, so the person re-types the exact name.
 */
export function inviteRefusal(message: string, typed: string, workspace: string): InviteRefusal {
  const refusal = parseRefusal(message);
  const code = refusal.code;
  const alreadyMember =
    (code !== null && ALREADY_MEMBER_CODES.has(code)) || /\balready a member\b/i.test(message);
  if (alreadyMember)
    return { text: inviteCopy.refusal.alreadyMember(typed, workspace), alreadyMember };
  if (code !== null && NO_ACCOUNT_CODES.has(code))
    return { text: inviteCopy.refusal.noAccount(typed), alreadyMember: false };
  const canonical = CANONICAL_HINT.exec(refusal.sentence)?.[1];
  if (
    canonical &&
    canonical !== typed &&
    (code === 'identity_ambiguous' || code === 'invalid_params')
  )
    return { text: inviteCopy.refusal.canonical(canonical), alreadyMember: false };
  return { text: refusalText(message), alreadyMember: false };
}

/** An `enrollment.approve` refusal. */
export function approveRefusalText(message: string, first: string): string {
  return parseRefusal(message).code === 'code_mismatch'
    ? letInCopy.mismatch(first)
    : refusalText(message);
}

/** A failed call to a route that may be newer than the daemon, in words. */
export function newRouteFailureText(failure: unknown, fallback: string): string {
  if (isStaleDaemon(failure)) return STALE_DAEMON_MESSAGE;
  if (failure instanceof Error && failure.message) return failure.message;
  return fallback;
}
