import { crewHttp } from '../crewApi';
import { outdatedDaemonResponse, unexpectedCrewResponse } from './errors';
import { isRecord, optionalText, stringArray } from './parse';

// The daemon's name resolver (S1b, D7): `POST /crew/resolve`. It resolves against the person's own
// workspace snapshot, so it can never name something the person cannot see. The resolver is a
// lookup, not a permission: the broker still authorizes every mutation that uses its answer.

/**
 * What a selector names. Authority-bearing inputs (invite, remove, transfer ownership, admit) must
 * send people as `@username` or an ID; the resolver never matches a display name for them.
 *
 * Attachments are not resolvable: they are chosen from a channel's files, and the daemon refuses a
 * selector of that kind with 400 `crew_invalid_selector`. The kind is therefore neither sent nor
 * accepted in an answer.
 */
export type CrewSelectorKind = 'person' | 'former_person' | 'team' | 'channel' | 'connection';

const SELECTOR_KINDS: readonly string[] = [
  'person',
  'former_person',
  'team',
  'channel',
  'connection',
];

export interface CrewSelector {
  /** Omit to let the grammar decide: `@bob` is a person, `team/channel` a channel. */
  kind?: CrewSelectorKind;
  text: string;
}

export interface CrewResolvedName {
  status: 'resolved';
  kind: CrewSelectorKind;
  text: string;
  /** The object's ID. Send it to the broker; never show it. */
  id: string;
  /** A label to confirm the choice with, for example `Bob Lee (@bob)` or `#methods`. */
  label?: string;
  /** For a person: the canonical username to send as `expected_username`. */
  username?: string;
}

/** Nothing the person can see has this name. Deliberately lists no candidates. */
export interface CrewUnknownName {
  status: 'unknown_name';
  kind: CrewSelectorKind;
  text: string;
  /**
   * The one member whose username differs from the text only in letter case, as `@bob`. It is
   * never resolved silently: show "Did you mean @bob?" and let the person type it as shown.
   */
  did_you_mean?: string;
}

/** More than one visible object matches. `candidates` are labels to choose from, never IDs. */
export interface CrewAmbiguousName {
  status: 'ambiguous_name';
  kind: CrewSelectorKind;
  text: string;
  candidates: string[];
}

export type CrewResolution = CrewResolvedName | CrewUnknownName | CrewAmbiguousName;

export interface CrewResolveResult {
  /** The saved connection, when one was named. */
  connection: CrewResolution | null;
  /** One resolution per selector, in the order they were sent. */
  results: CrewResolution[];
}

function resolutionFrom(value: unknown): CrewResolution | null {
  if (!isRecord(value)) return null;
  const kind = value.kind;
  const text = value.text;
  if (typeof kind !== 'string' || !SELECTOR_KINDS.includes(kind) || typeof text !== 'string')
    return null;
  const selector = { kind: kind as CrewSelectorKind, text };
  switch (value.status) {
    case 'resolved': {
      const id = optionalText(value.id);
      if (!id) return null;
      const resolved: CrewResolvedName = { status: 'resolved', ...selector, id };
      const label = optionalText(value.label);
      if (label) resolved.label = label;
      const username = optionalText(value.username);
      if (username) resolved.username = username;
      return resolved;
    }
    case 'unknown_name': {
      const unknown: CrewUnknownName = { status: 'unknown_name', ...selector };
      const suggestion = optionalText(value.did_you_mean);
      if (suggestion) unknown.did_you_mean = suggestion;
      return unknown;
    }
    case 'ambiguous_name': {
      const candidates = stringArray(value.candidates);
      return candidates ? { status: 'ambiguous_name', ...selector, candidates } : null;
    }
    default:
      return null;
  }
}

/**
 * Resolve typed names in one workspace. `connection` selects the saved connection whose workspace
 * the selectors are resolved in: its ID (UUID-shaped text is always an ID), saved name or
 * `ssh_target`. Omit it only to leave the choice of connection to the daemon.
 *
 * A daemon without the route rejects with a `CrewHttpError` that `isStaleDaemon` recognizes.
 */
export async function resolve(
  selectors: CrewSelector[],
  connection?: string,
  signal?: AbortSignal
): Promise<CrewResolveResult> {
  const result = await crewHttp<unknown>(
    '/resolve',
    'POST',
    connection === undefined ? { selectors } : { connection, selectors },
    signal
  );
  if (!isRecord(result)) throw outdatedDaemonResponse();
  if (!Array.isArray(result.results) || result.results.length !== selectors.length)
    throw unexpectedCrewResponse('a name lookup');
  const results = result.results.map(resolutionFrom);
  if (results.some((resolution) => resolution === null))
    throw unexpectedCrewResponse('a name lookup');
  let resolvedConnection: CrewResolution | null = null;
  if (connection !== undefined) {
    resolvedConnection = resolutionFrom(result.connection);
    if (!resolvedConnection || resolvedConnection.kind !== 'connection')
      throw unexpectedCrewResponse('a name lookup');
  }
  return { connection: resolvedConnection, results: results as CrewResolution[] };
}
