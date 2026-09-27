import { isStaleDaemon, STALE_DAEMON_MESSAGE } from '../api/errors';
import { dialogErrorCopy, inviteCopy, letInCopy, nameRuleCopy, refusalCopy } from './copy';

/**
 * Turning a refusal into the words a dialog shows.
 *
 * The daemon forwards a broker refusal as its text, which the broker writes `code: sentence`
 * (`bail!("forbidden: team owner required")`, `NameError::wire`). A daemon from before the
 * `broker_code` field wrapped that text as `Crew broker refused request: {"code":…,"message":…}`;
 * the envelope is removed first, so both daemons read the same.
 *
 * Most refusals are shown exactly as written, in their own text node, because the daemon's words
 * are the ones support can search for. A refusal is reworded only when its code is recognised:
 * - a code whose broker text is written for a person (`SENTENCE_CODES`) shows that sentence without
 *   its code, provided it reads as one (capitalised, ending in a full stop);
 * - a code whose broker text is written for a program (`REWORDED`) shows the copy deck's sentence.
 *
 * Anything else — including a code this renderer has never seen — is shown verbatim, never guessed
 * at. The CLI makes the same choices for the same codes (`commands/crew/output.rs`).
 */

export interface BrokerRefusal {
  /** The broker's code, e.g. `name_invalid`; null when the text carries none. */
  code: string | null;
  /** The text after the code, or the whole text when there is none. */
  sentence: string;
  /** The broker's own text (`code: sentence`), with an older daemon's envelope removed. */
  text: string;
  /** The text as the daemon sent it. */
  raw: string;
}

const CODED = /^([a-z][a-z0-9_]*): ([\s\S]+)$/;
const CODE_SHAPE = /^[a-z][a-z0-9_]*$/;
/** What an older daemon put before the broker's JSON error object. */
const LEGACY_ENVELOPE = 'Crew broker refused request: ';

/** An older daemon's `Crew broker refused request: {json}`: the code, and the text it carries. */
function unwrapLegacyEnvelope(text: string): { code: string; text: string } | null {
  if (!text.startsWith(LEGACY_ENVELOPE)) return null;
  let body: unknown;
  try {
    body = JSON.parse(text.slice(LEGACY_ENVELOPE.length));
  } catch {
    return null;
  }
  if (typeof body !== 'object' || body === null || Array.isArray(body)) return null;
  const { code, message } = body as Record<string, unknown>;
  if (typeof code !== 'string' || !CODE_SHAPE.test(code)) return null;
  const words = typeof message === 'string' ? message.trim() : '';
  return { code, text: words || code };
}

export function parseRefusal(message: string): BrokerRefusal {
  const trimmed = message.trim();
  const legacy = unwrapLegacyEnvelope(trimmed);
  const text = legacy?.text ?? trimmed;
  const match = CODED.exec(text);
  if (match) return { code: match[1], sentence: match[2].trim(), text, raw: message };
  // An envelope names its code even when its message does not repeat it.
  return { code: legacy?.code ?? null, sentence: text, text, raw: message };
}

/** Whether the broker wrote `sentence` for a person: capitalised, and ending in a full stop. */
function readsAsSentence(sentence: string): boolean {
  return /^[\p{Lu}\p{N}@]/u.test(sentence) && /[.!?]$/.test(sentence);
}

/**
 * Codes whose sentence part the broker writes for a person, so the code prefix can go
 * (`broker/join.rs`, `broker.rs`'s naming constants, `DeviceCodeError`). Some of these codes also
 * have a technical text elsewhere in the broker; `REWORDED` catches those first, and any other text
 * that does not read as a sentence is shown verbatim.
 */
const SENTENCE_CODES = new Set([
  'name_invalid',
  'name_taken',
  'device_code_invalid',
  'rate_limited',
  'target_mismatch',
  'not_invited',
  'identity_unavailable',
  'identity_ambiguous',
  'identity_conflict',
  'identity_mismatch',
  'unknown_account',
  'already_member',
  'already_approved',
  'quota_exceeded',
  'join_expired',
  'join_changed',
  'account_changed',
  'code_mismatch',
]);

/** `@username` in a broker sentence; a trailing full stop is never part of it. */
const MENTION = /@([A-Za-z0-9._-]*[A-Za-z0-9_-])/;

/**
 * The `quota_exceeded` texts that mean "this workspace is full, and no further change can be made
 * to it" (`refusalCopy.storageFull`). A request can meet three of them, all in `broker.rs`:
 * - the audit journal limit in `commit`: `retained audit journal exceeds 1 GiB; …`;
 * - the state-size limit in `commit`: `workspace logical state exceeds 16 MiB; …`;
 * - the operation quota in `apply_mutation`: `workspace operation quota requires maintenance`
 *   (the table of remembered request IDs is full).
 *
 * A fourth, `journal exceeds supported replay size of 1 GiB`, is raised by `open_inner` and so only
 * stops the broker starting; no request is ever refused with it, but it means the same and is
 * matched in case a startup failure is ever forwarded.
 */
const STORAGE_FULL_TEXT =
  /^(?:(?:retained audit )?journal exceeds|workspace logical state exceeds|workspace operation quota requires maintenance)\b/i;

/**
 * Broker texts written for a program, and the copy deck's words for them. Each is matched on the
 * code and on the text's own form, because a code can also carry a person-written sentence that
 * says something more specific (`identity_conflict` for an invited account, `identity_mismatch`
 * for a renamed one, the waiting-list `quota_exceeded`), and that sentence is kept.
 *
 * `quota_exceeded` is matched on its words, not its form (`STORAGE_FULL_TEXT`). The join quota
 * (`broker/join.rs`) means "too many people are waiting", and its person-written sentence is kept,
 * so one sentence for every `quota_exceeded` would be wrong for it. `device_conflict` has only a
 * technical text, so every one is reworded.
 */
const REWORDED: readonly {
  code: string;
  matches(sentence: string): boolean;
  words(sentence: string): string;
}[] = [
  {
    code: 'identity_conflict',
    matches: (sentence) => /^another active member is @/i.test(sentence),
    words: (sentence) => refusalCopy.identityConflict(MENTION.exec(sentence)?.[1] ?? null),
  },
  { code: 'device_conflict', matches: () => true, words: () => refusalCopy.deviceConflict },
  {
    code: 'identity_mismatch',
    matches: (sentence) => !readsAsSentence(sentence),
    words: () => refusalCopy.identityMismatch,
  },
  {
    code: 'quota_exceeded',
    matches: (sentence) => STORAGE_FULL_TEXT.test(sentence),
    words: () => refusalCopy.storageFull,
  },
  {
    code: 'rate_limited',
    matches: (sentence) => !readsAsSentence(sentence),
    words: () => refusalCopy.tooManyAttempts,
  },
];

/** A name some object the viewer may not even see already holds (naming design D5). */
const NAME_TAKEN_CODES = new Set(['name_conflict', 'name_taken']);

/** The words for a refusal no dialog rewrites: the copy deck's or the broker's sentence, else verbatim. */
export function refusalText(message: string): string {
  const refusal = parseRefusal(message);
  if (!refusal.text) return dialogErrorCopy.fallback;
  if (refusal.code) {
    const reworded = REWORDED.find(
      (entry) => entry.code === refusal.code && entry.matches(refusal.sentence)
    );
    if (reworded) return reworded.words(refusal.sentence);
    if (SENTENCE_CODES.has(refusal.code) && readsAsSentence(refusal.sentence))
      return refusal.sentence;
  }
  return refusal.text;
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
/**
 * The canonical spelling a broker names (`Invite @alice.`). A username may contain dots, but its
 * last character never is one, so the sentence's own full stop is left out.
 */
const CANONICAL_HINT = /(?:invite|did you mean)\s+@([A-Za-z0-9._-]*[A-Za-z0-9_-])/i;

/** The account spelling a refusal offers instead of the typed one, if it names one. */
export function canonicalHint(sentence: string): string | null {
  return CANONICAL_HINT.exec(sentence)?.[1] ?? null;
}

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
    (code !== null && ALREADY_MEMBER_CODES.has(code)) || /\balready a member\b/i.test(refusal.text);
  if (alreadyMember)
    return { text: inviteCopy.refusal.alreadyMember(typed, workspace), alreadyMember };
  if (code !== null && NO_ACCOUNT_CODES.has(code))
    return { text: inviteCopy.refusal.noAccount(typed), alreadyMember: false };
  const canonical = canonicalHint(refusal.sentence);
  if (
    canonical &&
    canonical !== typed &&
    (code === 'identity_ambiguous' || code === 'invalid_params')
  )
    return { text: inviteCopy.refusal.canonical(canonical), alreadyMember: false };
  return { text: refusalText(message), alreadyMember: false };
}

/**
 * Codes under which the broker's direct-add refusals (`team.add_member`, `channel.add_member`,
 * `direct_add_v1`) are written for a person — "You can only add people to channels you own.
 * Uncheck #methods and try again." — while the same codes elsewhere carry technical text
 * (`forbidden: team unavailable`). So they are not `SENTENCE_CODES`, which the CLI mirrors: only a
 * direct-add dialog drops the prefix, and only from a text that reads as a sentence.
 */
const DIRECT_ADD_SENTENCE_CODES = new Set(['forbidden', 'invalid_params', 'channel_archived']);

/**
 * A direct-add refusal: the broker's sentence where it wrote one for a person, else as usual. A
 * sentence may open with the channel it is about (`#old is archived, …`).
 */
export function directAddRefusalText(message: string): string {
  const refusal = parseRefusal(message);
  const sentence = refusal.sentence;
  if (
    refusal.code !== null &&
    DIRECT_ADD_SENTENCE_CODES.has(refusal.code) &&
    (readsAsSentence(sentence) || (sentence.startsWith('#') && /[.!?]$/.test(sentence)))
  )
    return sentence;
  return refusalText(message);
}

/** Whether an `enrollment.approve` refusal says a device was already let in for this person. */
export function isAlreadyApproved(message: string): boolean {
  return parseRefusal(message).code === 'already_approved';
}

/** An `enrollment.approve` refusal. */
export function approveRefusalText(message: string, username: string): string {
  return isAlreadyApproved(message) ? letInCopy.alreadyApproved(username) : refusalText(message);
}

/** A failed call to a route that may be newer than the daemon, in words. */
export function newRouteFailureText(failure: unknown, fallback: string): string {
  if (isStaleDaemon(failure)) return STALE_DAEMON_MESSAGE;
  if (failure instanceof Error && failure.message) return failure.message;
  return fallback;
}
