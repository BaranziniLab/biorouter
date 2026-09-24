import { CrewHttpError } from '../crewApi';
import { crewObservationCopy } from './copy';

/**
 * Observation error codes that concern access, privacy or the acting person. A history page refused
 * with one of these clears the view (`isLocalHistoryFailure`), and without the options below
 * `observationFailureOutcome` clears the draft for each of them.
 */
export const DRAFT_CLEARING_OBSERVATION_CODES: readonly string[] = [
  'channel_access_changed',
  'policy_changed',
  'scope_changed',
  'access_denied',
  'principal_revoked',
  'forbidden',
  'privacy_denied',
  'human_authority_required',
];

/**
 * Terminal codes a fresh observation answers by itself: the workspace policy epoch moved (any
 * invitation accepted, any member removed), the selected channel's access changed, the resume
 * cursor went stale, or the observed scope moved. On a connection the daemon calls connected, the
 * observer observes again on its own instead of stopping, and the next verified `state` frame
 * decides — by `draftScopeChanged` — whether the unsent draft may stay.
 */
export const RECOVERABLE_OBSERVATION_CODES: readonly string[] = [
  'policy_changed',
  'channel_access_changed',
  'stale_cursor',
  'scope_changed',
];

export function isRecoverableObservationCode(code: string | null | undefined): boolean {
  return typeof code === 'string' && RECOVERABLE_OBSERVATION_CODES.includes(code);
}

/** The broker's codes for a computer the workspace does not know (`unauthorized: unknown device`). */
export const UNKNOWN_DEVICE_CODES: readonly string[] = ['unauthorized', 'unknown_device'];

export interface ObservationFailureOutcome {
  /** Clear the draft, its attachments, references and context channels. */
  clearDraft: boolean;
  /** The text for the connection bar: what stopped, then what happened to the draft, if anything. */
  text: string;
}

export interface ObservationFailureOptions {
  /**
   * Whether the composer holds anything. The draft sentence is added only when it does: telling a
   * person their empty draft was cleared is a false alarm (T-07). Default: true.
   */
  draftHasContent?: boolean;
  /**
   * Keep the draft for a recoverable code: the next verified view compares what the draft was
   * written under and clears it only if that changed. The draft cannot be sent meanwhile — the
   * composer has no text box while nothing is verified. Default: false.
   */
  deferRecoverableToReverification?: boolean;
}

/** What an observation failure does to the draft, and the sentence that says so. */
export function observationFailureOutcome(
  message: string,
  code?: string,
  options: ObservationFailureOptions = {}
): ObservationFailureOutcome {
  const { draftHasContent = true, deferRecoverableToReverification = false } = options;
  const clearDraft =
    Boolean(code && DRAFT_CLEARING_OBSERVATION_CODES.includes(code)) &&
    !(deferRecoverableToReverification && isRecoverableObservationCode(code));
  if (!draftHasContent) return { clearDraft, text: message };
  const sentence = clearDraft
    ? crewObservationCopy.draftCleared
    : crewObservationCopy.draftRetained;
  return { clearDraft, text: `${message} ${sentence}` };
}

/** Names the plain sentences use: the workspace's display name, and the channel as `#name`. */
export interface ObservationNames {
  workspace: string;
  channel: string | null;
}

/**
 * Plain words for a terminal observation frame, by its code. The daemon writes one fixed sentence
 * for every observer error ("Room observation ended. Clear cached room content…"), so the frame's
 * own text is never shown.
 */
export function observationFrameText(code: string | undefined, names: ObservationNames): string {
  const { workspace, channel } = names;
  switch (code) {
    case 'channel_access_changed':
      return channel
        ? crewObservationCopy.channelAccessChanged(channel)
        : crewObservationCopy.channelAccessLost;
    case 'access_denied':
    case 'forbidden':
    case 'privacy_denied':
    case 'principal_revoked':
      return crewObservationCopy.accessChanged(workspace);
    case 'unauthorized':
    case 'unknown_device':
      return crewObservationCopy.unknownComputer(workspace);
    case 'human_authority_required':
      return crewObservationCopy.notConfirmed(workspace);
    case 'observer_capacity_reached':
      return crewObservationCopy.tooManyViews(workspace);
    case 'response_too_large':
      return crewObservationCopy.updateTooLarge(workspace);
    default:
      return crewObservationCopy.updatesStopped(workspace);
  }
}

/** The message of a thrown failure, or `fallback` for anything that is not an `Error`. */
export function failureMessage(failure: unknown, fallback: string): string {
  return failure instanceof Error ? failure.message : fallback;
}

/** The daemon's typed code of a thrown failure, when it has one. */
export function failureCode(failure: unknown): string | undefined {
  return failure instanceof CrewHttpError ? failure.code : undefined;
}

/**
 * The code an observation failure is classified by: the broker's own refusal code when the daemon
 * passed one on (`unauthorized`, `stale_cursor`…), else the daemon's.
 */
export function observationFailureCode(failure: unknown): string | undefined {
  return failure instanceof CrewHttpError ? (failure.brokerCode ?? failure.code) : undefined;
}

// ---------------------------------------------------------------------------------------------
// When an unsent draft must go: the material scope it was written under
// ---------------------------------------------------------------------------------------------

/** The parts of a verified `state` frame the draft rule reads. */
export interface ScopeFrame {
  connection_id: string;
  connection_mode: string;
  connection_policy_epoch: number;
  connection_institution_id?: string | null;
  snapshot: {
    workspace: { mode: string; institution_id?: string | null };
    channels: readonly { id: string; classification?: string }[];
  };
}

/**
 * What an unsent draft was written under, recorded from each verified view: the workspace's and
 * this connection's privacy, and the selected and source channels with their classification.
 *
 * ⚠ The workspace **policy epoch is deliberately absent**. The broker moves it for every accepted
 * invitation, removed member or archived channel anywhere in the workspace; comparing it cleared
 * every member's draft whenever anyone accepted an invitation (live QA round 1, P0-1/T-07).
 * Membership of the selected channel is absent for the same reason. The connection's own policy
 * epoch stays: it moves only when this connection's privacy binding does.
 */
export interface DraftScope {
  connectionId: string;
  workspaceMode: string;
  workspaceInstitution: string | null;
  connectionMode: string;
  connectionEpoch: number;
  connectionInstitution: string | null;
  /** The selected channel and its classification, or null outside a channel. */
  channel: { id: string; classification: string | null } | null;
  /** Each selected source channel's classification when recorded, by channel ID. */
  sources: ReadonlyMap<string, string | null>;
}

function classificationIn(frame: ScopeFrame, id: string): string | null | undefined {
  const channel = frame.snapshot.channels.find((item) => item.id === id);
  return channel ? (channel.classification ?? null) : undefined;
}

/** The scope a verified frame establishes for the selected channel and sources. */
export function draftScope(
  frame: ScopeFrame,
  channelId: string,
  sources: readonly string[]
): DraftScope {
  const selected = channelId ? classificationIn(frame, channelId) : undefined;
  const recorded = new Map<string, string | null>();
  for (const id of sources) {
    const classification = classificationIn(frame, id);
    if (classification !== undefined) recorded.set(id, classification);
  }
  return {
    connectionId: frame.connection_id,
    workspaceMode: frame.snapshot.workspace.mode,
    workspaceInstitution: frame.snapshot.workspace.institution_id ?? null,
    connectionMode: frame.connection_mode,
    connectionEpoch: frame.connection_policy_epoch,
    connectionInstitution: frame.connection_institution_id ?? null,
    channel: selected === undefined ? null : { id: channelId, classification: selected },
    sources: recorded,
  };
}

/**
 * Whether the draft must be cleared before this verified view is shown (SECURITY-SENSITIVE: a
 * draft written under one privacy scope must never become sendable under another). True when,
 * since the previous verified view of the same connection:
 * - the workspace's mode or institution changed;
 * - this connection's mode, policy epoch or institution changed;
 * - the selected channel's classification changed (its disappearance is the channel-revoked path);
 * - a selected source channel disappeared, or its classification changed.
 *
 * Nothing else clears it: a workspace policy epoch moving on its own is not a reason.
 */
export function draftScopeChanged(
  previous: DraftScope | null,
  frame: ScopeFrame,
  channelId: string,
  sources: readonly string[]
): boolean {
  if (!previous || previous.connectionId !== frame.connection_id) return false;
  if (
    previous.workspaceMode !== frame.snapshot.workspace.mode ||
    previous.workspaceInstitution !== (frame.snapshot.workspace.institution_id ?? null) ||
    previous.connectionMode !== frame.connection_mode ||
    previous.connectionEpoch !== frame.connection_policy_epoch ||
    previous.connectionInstitution !== (frame.connection_institution_id ?? null)
  )
    return true;
  if (channelId && previous.channel?.id === channelId) {
    const now = classificationIn(frame, channelId);
    if (now !== undefined && now !== previous.channel.classification) return true;
  }
  return sources.some((id) => {
    const now = classificationIn(frame, id);
    if (now === undefined) return true;
    return previous.sources.has(id) && previous.sources.get(id) !== now;
  });
}
