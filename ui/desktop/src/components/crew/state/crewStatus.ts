import { crewStatusCopy } from './copy';
import { isNotSetUpFailure, isTrustFailure, type ConnectFailureKind } from './connectFailure';

// ---------------------------------------------------------------------------------------------
// Connection status (the status row's dot and word)
// ---------------------------------------------------------------------------------------------

export type ConnectionStatusKey =
  | 'cant-verify'
  | 'connecting'
  | 'connected'
  | 'not-joined'
  | 'updates-unavailable'
  | 'updating'
  | 'reconnecting'
  | 'checking'
  | 'sign-in-needed'
  | 'not-set-up'
  | 'offline'
  | 'cant-connect';

export type StatusTone = 'success' | 'warning' | 'danger' | 'neutral' | 'idle';

export interface ConnectionStatusPresentation {
  tone: StatusTone;
  word: string;
  /** A connect or sign-in is running: show the spinner beside the word. */
  spinner: boolean;
  /** Screen-reader text that replaces the word (the pinned verified sentence). */
  srText?: string;
}

export const CONNECTION_STATUS: Readonly<
  Record<ConnectionStatusKey, ConnectionStatusPresentation>
> = {
  'cant-verify': { tone: 'danger', word: crewStatusCopy.cantVerify, spinner: false },
  connecting: { tone: 'neutral', word: crewStatusCopy.connecting, spinner: true },
  connected: {
    tone: 'success',
    word: crewStatusCopy.connected,
    spinner: false,
    srText: crewStatusCopy.verified,
  },
  'not-joined': { tone: 'neutral', word: crewStatusCopy.notJoined, spinner: false },
  'updates-unavailable': {
    tone: 'warning',
    word: crewStatusCopy.updatesUnavailable,
    spinner: false,
  },
  updating: { tone: 'neutral', word: crewStatusCopy.updating, spinner: false },
  reconnecting: { tone: 'neutral', word: crewStatusCopy.reconnecting, spinner: true },
  checking: { tone: 'neutral', word: crewStatusCopy.checking, spinner: false },
  'sign-in-needed': { tone: 'warning', word: crewStatusCopy.signInNeeded, spinner: false },
  'not-set-up': { tone: 'danger', word: crewStatusCopy.notSetUp, spinner: false },
  offline: { tone: 'idle', word: crewStatusCopy.offline, spinner: false },
  'cant-connect': { tone: 'danger', word: crewStatusCopy.cantConnect, spinner: false },
};

export interface ConnectionStatusInput {
  /** The selected saved connection; null when none is selected. */
  connection: { status: string } | null;
  /** The classified failure of the most recent connect or sign-in for this connection. */
  lastConnectFailure: { kind: ConnectFailureKind } | null;
  /** A connect or sign-in is in flight. */
  inFlight: boolean;
  /** A verified snapshot and observed privacy exist for this connection. */
  verified: boolean;
  /** The observer reported an error (the connection bar shows it). */
  observationError: boolean;
  /** Connected but not a member yet (join status other than joined). */
  notJoined: boolean;
  /**
   * A verified view ended for a recoverable reason and is being observed again by itself: a
   * neutral "Updating…", never "Updates unavailable". Absent: false.
   */
  reverifying?: boolean;
  /**
   * Live updates ended as a dropped connection would, and Crew is reading the saved record again
   * or observing it again quietly over the daemon's bridge (Q2-01): "Reconnecting…", above
   * "Offline" and "Updates unavailable". Absent: false.
   */
  reconnecting?: boolean;
}

/**
 * The status row's state, checked in the spec's order. "Not joined yet" is checked before the
 * connected-but-unverified rows, because a person who is not a member can only ever be connected
 * and unverified, and would otherwise read "Checking connection" forever. The daemon writes only
 * `connected` and `disconnected`, so sign-in and setup problems come from the typed code of the
 * most recent connect, not from the saved status.
 *
 * "Updates unavailable" is said only for a connection the daemon calls connected, and never while
 * a reconnect runs: the precedence is Reconnecting… > Offline > Updates unavailable (Q2-17).
 */
export function deriveConnectionStatus(input: ConnectionStatusInput): ConnectionStatusKey | null {
  const { connection, lastConnectFailure, inFlight, verified, observationError, notJoined } = input;
  const reverifying = input.reverifying === true;
  if (!connection) return null;
  const failure = lastConnectFailure?.kind;
  if (isTrustFailure(failure)) return 'cant-verify';
  if (verified) return inFlight ? 'connecting' : 'connected';
  if (input.reconnecting === true) return 'reconnecting';
  if (inFlight) return 'connecting';
  if (notJoined) return 'not-joined';
  if (connection.status === 'connected') {
    if (observationError) return 'updates-unavailable';
    return reverifying ? 'updating' : 'checking';
  }
  if (failure === 'auth_required' || connection.status === 'authentication_required')
    return 'sign-in-needed';
  if (isNotSetUpFailure(failure)) return 'not-set-up';
  if (connection.status === 'disconnected' && !failure) return 'offline';
  return 'cant-connect';
}

// ---------------------------------------------------------------------------------------------
// Main-area screen outside (and inside) a channel
// ---------------------------------------------------------------------------------------------

/**
 * `checking` is a saved-connected connection whose first verified snapshot has not arrived and
 * nothing has failed: a neutral placeholder. It exists so that opening Crew, or a dropped bridge
 * reconnecting, never shows the first-run or join screens to a person who is already a member.
 */
export type CrewScreen =
  | 'loading'
  | 'welcome'
  | 'connecting'
  | 'checking'
  | 'offline'
  | 'sign-in'
  | 'trust'
  | 'not-set-up'
  | 'join'
  | 'updates-paused'
  | 'no-team'
  | 'no-channel'
  | 'channel';

export interface CrewScreenInput {
  connectionsState: 'loading' | 'loaded' | 'failed';
  connectionCount: number;
  connection: { status: string } | null;
  lastConnectFailure: { kind: ConnectFailureKind } | null;
  /** A connect is in flight or the sign-in dialog is open. */
  inFlight: boolean;
  signInOpen: boolean;
  /**
   * The snapshot to draw a workspace from: the verified snapshot, else (during re-verification)
   * the last verified one for this connection. Null when neither exists.
   */
  view: {
    teams: readonly { id: string }[];
    channels: readonly { id: string; team_id: string }[];
  } | null;
  channelId: string;
  observationError: boolean;
  notJoined: boolean;
  /**
   * Crew is picking a dropped view up again (Q2-01): the connecting screen, never the "updates
   * stopped" one. Absent: false.
   */
  reconnecting?: boolean;
}

/** Exactly one main-area screen for the controller's state. Pure and table-tested. */
export function deriveCrewScreen(input: CrewScreenInput): CrewScreen {
  if (input.connectionCount === 0) {
    if (input.connectionsState === 'loading') return 'loading';
    return input.connectionsState === 'failed' ? 'updates-paused' : 'welcome';
  }
  const { connection, view } = input;
  // A saved connection exists but is not selected yet (the selection lands with the list).
  if (!connection) return 'loading';
  const failure = input.lastConnectFailure?.kind;
  if (isTrustFailure(failure)) return 'trust';
  // A workspace that was verified is shown until it is replaced or a failure clears it; the
  // re-verification view never falls back to a setup or join screen.
  if (view) {
    if (view.teams.length === 0) return 'no-team';
    return view.channels.some((channel) => channel.id === input.channelId)
      ? 'channel'
      : 'no-channel';
  }
  if (input.inFlight || input.reconnecting === true) return 'connecting';
  if (failure === 'auth_required' && !input.signInOpen) return 'sign-in';
  if (isNotSetUpFailure(failure)) return 'not-set-up';
  if (input.notJoined) return 'join';
  // Mirrors the status table: an observation error pauses updates only on a connection the
  // daemon calls connected. The observer also runs for a saved-disconnected connection (after an
  // app restart or a dropped SSH bridge), and the daemon answers it with an error frame; showing
  // Retry there would re-run the same refusal forever, when the one action that helps is Connect.
  if (connection.status === 'connected') {
    return input.observationError ? 'updates-paused' : 'checking';
  }
  if (connection.status === 'authentication_required') return 'sign-in';
  return 'offline';
}

// ---------------------------------------------------------------------------------------------
// Owned-run status words
// ---------------------------------------------------------------------------------------------

/** Run statuses in which a Stop (the existing cancel route) is offered. */
export const CANCELLABLE_RUN_STATUSES: readonly string[] = [
  'starting',
  'running',
  'waiting_for_approval',
  'cancellation_pending',
  'cancellation_unconfirmed',
  'interrupted',
  'outcome_not_durable',
];

export type RunTone = 'running' | 'warning' | 'muted' | 'danger';
export type RunInlineAction = 'open' | 'review' | 'stop-again' | null;

export interface RunStatusPresentation {
  word: string;
  tone: RunTone;
  action: RunInlineAction;
  /** Show the visible Stop control. */
  stoppable: boolean;
}

const RUN_STATUS: Readonly<Record<string, Omit<RunStatusPresentation, 'stoppable'>>> = {
  starting: { word: 'Starting…', tone: 'running', action: null },
  running: { word: 'Working…', tone: 'running', action: 'open' },
  waiting_for_approval: { word: 'Waiting for your approval', tone: 'warning', action: 'review' },
  cancellation_pending: { word: 'Stopping…', tone: 'running', action: null },
  cancellation_unconfirmed: { word: 'Stop not confirmed', tone: 'warning', action: 'stop-again' },
  interrupted: { word: 'Interrupted', tone: 'muted', action: 'open' },
  outcome_not_durable: { word: 'Outcome unknown', tone: 'muted', action: 'open' },
  completed: { word: 'Done', tone: 'muted', action: 'open' },
  failed: { word: 'Couldn’t finish', tone: 'danger', action: 'open' },
  cancelled: { word: 'Stopped', tone: 'muted', action: 'open' },
};

/** `snake_case_value` → "Snake case value", for a status this renderer does not know yet. */
export function sentenceCaseStatus(value: string): string {
  const words = value.replace(/_/g, ' ').trim().toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function runStatusPresentation(status: string): RunStatusPresentation {
  const known = Object.prototype.hasOwnProperty.call(RUN_STATUS, status)
    ? RUN_STATUS[status]
    : undefined;
  return {
    ...(known ?? { word: sentenceCaseStatus(status), tone: 'muted', action: 'open' }),
    stoppable: CANCELLABLE_RUN_STATUSES.includes(status),
  };
}

// ---------------------------------------------------------------------------------------------
// Transfer state words
// ---------------------------------------------------------------------------------------------

export type TransferStateKey =
  | 'starting'
  | 'uploading'
  | 'downloading'
  | 'finishing'
  | 'pausing'
  | 'paused'
  | 'ready'
  | 'saved'
  | 'failed'
  | 'not-confirmed'
  | 'unknown';

export interface TransferStatePresentation {
  key: TransferStateKey;
  word: string;
  /** The transfer is moving bytes or finishing: offer Pause and keep polling. */
  active: boolean;
  /** Whole percent for the progress bar, when the state has one. */
  percent?: number;
}

export interface TransferStateInput {
  state: string;
  direction: 'upload' | 'download';
  offset: number;
  size: number;
  error?: string | null;
}

function percentOf(offset: number, size: number): number {
  if (!(size > 0) || !Number.isFinite(offset)) return 0;
  return Math.min(100, Math.max(0, Math.floor((offset / size) * 100)));
}

/**
 * The word a transfer row shows. The daemon's receipt states are `starting`, `uploading`,
 * `downloading`, `publishing`, `pause_requested`, `needs_file_selection` (paused, or stopped by a
 * failure when `error` is set), `completed` and `publication_unconfirmed`.
 */
export function transferStatePresentation(transfer: TransferStateInput): TransferStatePresentation {
  const percent = percentOf(transfer.offset, transfer.size);
  switch (transfer.state) {
    case 'starting':
      return { key: 'starting', word: 'Starting…', active: true };
    case 'uploading':
      return { key: 'uploading', word: `Uploading ${percent}%`, active: true, percent };
    case 'downloading':
      return { key: 'downloading', word: `Downloading ${percent}%`, active: true, percent };
    case 'publishing':
      return { key: 'finishing', word: 'Finishing…', active: true };
    case 'pause_requested':
      return { key: 'pausing', word: 'Pausing…', active: true, percent };
    case 'needs_file_selection':
      return transfer.error
        ? { key: 'failed', word: 'Failed', active: false }
        : { key: 'paused', word: 'Paused', active: false, percent };
    case 'completed':
      return transfer.direction === 'upload'
        ? { key: 'ready', word: 'Ready', active: false }
        : { key: 'saved', word: 'Saved', active: false };
    case 'failed':
      return { key: 'failed', word: 'Failed', active: false };
    case 'publication_unconfirmed':
      return { key: 'not-confirmed', word: 'Not confirmed', active: false };
    default:
      return { key: 'unknown', word: sentenceCaseStatus(transfer.state), active: false };
  }
}
