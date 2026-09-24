import { grantDestinationLabel, sessionGrantState, type CrewSessionGrant } from '../api/grants';
import type { ObservedRun } from '../crewApi';
import {
  channelNamesAcrossTeams,
  isMachineIdShaped,
  sanitizeDisplayText,
  type ChannelNameInput,
  type TeamNameInput,
} from '../identity';
import { CANCELLABLE_RUN_STATUSES } from '../state/crewStatus';
import { accessCopy } from './copy';

/**
 * How a grant reads in a list (ui-redesign-spec, "Revoke", "Access rows"). Pure, so the Access tab,
 * the Agent access tab, the Agents section and the header chip share one set of decisions and a
 * test can read them without rendering anything.
 *
 * Nothing here decides what a person may do. A row's `canRevoke` and `canStop` only say which
 * control describes the row's state; the daemon decides whether the request succeeds.
 */

/** A row's state at rest. `unconfirmed` = stopped on this device, not yet confirmed remotely. */
export type AccessStatus = 'active' | 'expired' | 'revoked' | 'unconfirmed';

export interface AccessRow {
  /** Unique across connections. */
  key: string;
  connectionId: string;
  sessionId: string;
  runId: string;
  kind: 'chat' | 'task';
  /** What the row is called: the chat's title, "Untitled chat", or "Your task". */
  title: string;
  /** The chat's own title for sentences, or `null` when the daemon does not know it. */
  chatTitle: string | null;
  /** The channel it posts in. */
  channelId: string;
  /**
   * `#slug`, or `{team} / #slug` when two teams share the slug. When the snapshot does not show the
   * channel, the name the person saw when granting access; otherwise "a channel you can't see".
   */
  destination: string;
  /** Further channels it may read, beyond the destination. */
  extraSources: number;
  status: AccessStatus;
  statusLabel: string;
  /** Unix seconds, when the daemon recorded when the workspace ends the grant. */
  expiresAt: number | null;
  /** The owned run for a task, when the observer reported it. */
  run: ObservedRun | null;
  /** An active chat row: offer a visible Revoke. */
  canRevoke: boolean;
  /** A task row whose run can still be stopped: offer a visible Stop. */
  canStop: boolean;
  /** Stopped on this device only: offer Retry. */
  canRetry: boolean;
}

/** The fields of a snapshot a row reads. */
export interface AccessRowSnapshot {
  teams: readonly TeamNameInput[];
  channels: readonly ChannelNameInput[];
}

export interface AccessRowInput {
  snapshot?: AccessRowSnapshot | null;
  /** The observed owned runs, for task rows and for classifying rows the daemon did not type. */
  runs?: readonly ObservedRun[];
  /** Milliseconds; defaults to `Date.now()`. */
  now?: number;
  /** Whether this window saw the grant stopped only on this device. */
  isUnconfirmed?: (connectionId: string, sessionId: string) => boolean;
}

/** Run statuses that are over: no Stop, and hidden at rest in the Agents section. */
const FINISHED_RUN_STATUSES: readonly string[] = ['completed', 'failed', 'cancelled'];

export function isFinishedRun(run: Pick<ObservedRun, 'status'>): boolean {
  return FINISHED_RUN_STATUSES.includes(run.status);
}

/** A task when the daemon says so; before RV-D2, when an owned run carries the same session. */
export function grantKind(
  grant: CrewSessionGrant,
  runs: readonly ObservedRun[] = []
): 'chat' | 'task' {
  if (grant.kind === 'chat' || grant.kind === 'task') return grant.kind;
  return runs.some((run) => run.session_id === grant.session_id) ? 'task' : 'chat';
}

/** A conversation title fit to display, or `null` (never an ID-shaped string). */
export function chatTitleOf(grant: Pick<CrewSessionGrant, 'session_name'>): string | null {
  const title = sanitizeDisplayText(grant.session_name);
  return title && !isMachineIdShaped(title) ? title : null;
}

/** Channel labels for a snapshot, keyed by channel ID. */
export function channelLabels(snapshot: AccessRowSnapshot | null | undefined): Map<string, string> {
  return snapshot ? channelNamesAcrossTeams(snapshot.channels, snapshot.teams) : new Map();
}

/** "4:40 PM" today, "Sep 24, 4:40 PM" on another day, in the viewer's locale. */
export function formatExpiry(expiresAtSeconds: number, now = Date.now()): string {
  const at = new Date(expiresAtSeconds * 1000);
  const today = new Date(now);
  const sameDay =
    at.getFullYear() === today.getFullYear() &&
    at.getMonth() === today.getMonth() &&
    at.getDate() === today.getDate();
  return new Intl.DateTimeFormat(
    undefined,
    sameDay
      ? { hour: 'numeric', minute: '2-digit' }
      : { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }
  ).format(at);
}

/** The status a grant shows, and its label. */
export function accessStatusOf(
  grant: CrewSessionGrant,
  now = Date.now(),
  unconfirmed = false
): { status: AccessStatus; label: string } {
  const state = sessionGrantState(grant, now);
  if (state === 'active')
    return {
      status: 'active',
      label:
        typeof grant.expires_at === 'number'
          ? accessCopy.status.expires(formatExpiry(grant.expires_at, now))
          : accessCopy.status.active,
    };
  if (state === 'expired') return { status: 'expired', label: accessCopy.status.expired };
  return unconfirmed
    ? { status: 'unconfirmed', label: accessCopy.status.unconfirmed }
    : { status: 'revoked', label: accessCopy.status.revoked };
}

/** The badge tone of a status. */
export function accessStatusTone(status: AccessStatus): 'success' | 'warning' | 'neutral' {
  if (status === 'active') return 'success';
  if (status === 'unconfirmed') return 'warning';
  return 'neutral';
}

/** One row per grant. `channelLabelsById` defaults to the snapshot's labels. */
export function accessRow(
  grant: CrewSessionGrant,
  input: AccessRowInput,
  channelLabelsById: Map<string, string> = channelLabels(input.snapshot)
): AccessRow {
  const runs = input.runs ?? [];
  const now = input.now ?? Date.now();
  const kind = grantKind(grant, runs);
  const chatTitle = kind === 'chat' ? chatTitleOf(grant) : null;
  const run =
    runs.find((candidate) => candidate.session_id === grant.session_id) ??
    runs.find((candidate) => candidate.run_id === grant.run_id) ??
    null;
  const { status, label } = accessStatusOf(
    grant,
    now,
    input.isUnconfirmed?.(grant.connection_id, grant.session_id) ?? false
  );
  const extraSources = new Set(grant.source_channels.filter((id) => id !== grant.channel_id)).size;
  return {
    key: `${grant.connection_id}\n${grant.session_id}`,
    connectionId: grant.connection_id,
    sessionId: grant.session_id,
    runId: grant.run_id,
    kind,
    title: kind === 'task' ? accessCopy.yourTask : (chatTitle ?? accessCopy.untitled),
    chatTitle,
    channelId: grant.channel_id,
    destination:
      channelLabelsById.get(grant.channel_id) ??
      (sanitizeDisplayText(grantDestinationLabel(grant)) || accessCopy.unknownChannel),
    extraSources,
    status,
    statusLabel: label,
    expiresAt: typeof grant.expires_at === 'number' ? grant.expires_at : null,
    run,
    canRevoke: kind === 'chat' && status === 'active',
    canStop:
      kind === 'task' &&
      status === 'active' &&
      (run ? CANCELLABLE_RUN_STATUSES.includes(run.status) : true),
    canRetry: status === 'unconfirmed',
  };
}

function rank(row: AccessRow): number {
  if (row.status === 'unconfirmed') return 0;
  if (row.status === 'active') return 1;
  return 2;
}

/**
 * Rows for a list of grants, attention first: stopped-but-unconfirmed, then active (the most
 * recently granted first), then expired and revoked. `channelId` keeps only the grants that may
 * post in or read that channel.
 */
export function accessRows(
  grants: readonly CrewSessionGrant[],
  input: AccessRowInput & { channelId?: string } = {}
): AccessRow[] {
  const labels = channelLabels(input.snapshot);
  return grants
    .filter(
      (grant) =>
        !input.channelId ||
        grant.channel_id === input.channelId ||
        grant.source_channels.includes(input.channelId)
    )
    .map((grant) => accessRow(grant, input, labels))
    .sort(
      (a, b) =>
        rank(a) - rank(b) ||
        (b.expiresAt ?? 0) - (a.expiresAt ?? 0) ||
        a.title.localeCompare(b.title) ||
        a.sessionId.localeCompare(b.sessionId)
    );
}

/** Rows shown at rest, and the revoked and expired rows behind "Show revoked and expired (n)". */
export function splitAccessRows(rows: readonly AccessRow[]): {
  current: AccessRow[];
  old: AccessRow[];
} {
  return {
    current: rows.filter((row) => row.status === 'active' || row.status === 'unconfirmed'),
    old: rows.filter((row) => row.status === 'expired' || row.status === 'revoked'),
  };
}

export interface AgentAccessCount {
  chats: number;
  tasks: number;
  total: number;
  /** "{n} chats" / "{n} tasks" / "{n} agents", or "" when nothing can post. */
  label: string;
  /** "{n} chats or agents can post here". */
  accessibleName: string;
}

/**
 * How many chats and tasks can post in a channel right now, for the channel header's chip: active
 * chat grants whose destination is the channel, and tasks posting there — an owned run still in
 * progress, or an active task grant whose run the observer has not reported.
 */
export function agentAccessCount(input: {
  grants: readonly CrewSessionGrant[];
  runs?: readonly ObservedRun[];
  channelId: string;
  now?: number;
}): AgentAccessCount {
  const runs = input.runs ?? [];
  const now = input.now ?? Date.now();
  const chats = new Set<string>();
  const tasks = new Set<string>();
  for (const run of runs) {
    if (run.channel_id === input.channelId && CANCELLABLE_RUN_STATUSES.includes(run.status))
      tasks.add(run.session_id || run.run_id);
  }
  for (const grant of input.grants) {
    if (grant.channel_id !== input.channelId || sessionGrantState(grant, now) !== 'active')
      continue;
    if (grantKind(grant, runs) === 'task') {
      const known = runs.some((run) => run.session_id === grant.session_id);
      if (!known) tasks.add(grant.session_id);
    } else chats.add(grant.session_id);
  }
  const total = chats.size + tasks.size;
  const label =
    total === 0
      ? ''
      : tasks.size === 0
        ? accessCopy.chipChats(chats.size)
        : chats.size === 0
          ? accessCopy.chipTasks(tasks.size)
          : accessCopy.chipAgents(total);
  return {
    chats: chats.size,
    tasks: tasks.size,
    total,
    label,
    accessibleName: total === 0 ? '' : accessCopy.chipName(total),
  };
}
