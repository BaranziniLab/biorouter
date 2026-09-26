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
import { pastAccessGrants, type PastAccessEntry } from './pastAccess';

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
  /**
   * What tells one task row from another after its title: when it started and the task's first
   * words, `1:16 PM · Please work out…` (Q2-74). `null` for a chat, whose title already does, and
   * for a task when neither is known.
   */
  detail: string | null;
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
  /** The badge. A task's grant that is over reads "Ended": it ended with the task (Q2-09). */
  statusLabel: string;
  /** Unix seconds, when the daemon recorded when the workspace ends the grant. */
  expiresAt: number | null;
  /**
   * When this device saw the revoke confirmed, in milliseconds, for a revoked row it remembers
   * (`pastAccess.ts`); `null` otherwise. It is in the badge — "Revoked · 7:32 AM" — so two revokes
   * of the same chat are two different rows to the eye (F5).
   */
  revokedAt: number | null;
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
  /**
   * Revokes this device remembers for one connection (`pastAccess.ts`, Q4-12). Each becomes a
   * "Revoked" row unless the daemon's list still holds the same run: the list keeps one grant per
   * chat, so a chat granted again had lost its revoked row.
   */
  pastAccess?: { connectionId: string; entries: readonly PastAccessEntry[] };
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

/**
 * How long a grant lasts: the daemon asks the broker for an hour (`expires_in: 3600` in
 * `crates/biorouter/src/crew/mod.rs`), which is the consent's "or after an hour". A grant's start
 * is its end less this, for a task whose run the observer did not report.
 */
const GRANT_LIFETIME_SECONDS = 3600;

/** When a task started, in Unix seconds: its run's own time, else its grant's end less an hour. */
export function taskStartedAt(
  grant: Pick<CrewSessionGrant, 'expires_at'>,
  run: Pick<ObservedRun, 'started_at'> | null
): number | null {
  if (run && typeof run.started_at === 'number' && Number.isFinite(run.started_at))
    return run.started_at / 1000;
  if (typeof grant.expires_at === 'number' && Number.isFinite(grant.expires_at))
    return grant.expires_at - GRANT_LIFETIME_SECONDS;
  return null;
}

const TASK_TITLE_PREFIX = 'Crew';
const TASK_TITLE_SEPARATOR = ' · ';
const TASK_WORDS = 3;

/**
 * The first words of a task, from its conversation's title. The daemon names a task conversation
 * `Crew · #methods · Please work out the sum…` (`task_title` in the server's Crew routes); anything
 * else — "Crew task" before admission, a title with no prompt, one renamed since — has none.
 */
export function taskFirstWords(sessionName: string | null | undefined): string | null {
  const name = sanitizeDisplayText(sessionName);
  const parts = name.split(TASK_TITLE_SEPARATOR);
  if (parts.length < 3 || parts[0] !== TASK_TITLE_PREFIX) return null;
  const excerpt = parts.slice(2).join(TASK_TITLE_SEPARATOR).trim();
  const cut = excerpt.endsWith('…');
  const words = excerpt.replace(/…$/, '').trim().split(/\s+/).filter(Boolean);
  if (words.length === 0 || isMachineIdShaped(words.join(' '))) return null;
  const shown = words.slice(0, TASK_WORDS).join(' ');
  return cut || words.length > TASK_WORDS ? `${shown}…` : shown;
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

/** The workspace itself ended the grant: Crew's settings or its policy moved since it (D-1). */
function endedByWorkspace(grant: CrewSessionGrant): boolean {
  return grant.expired && grant.revocation === 'ended_by_workspace';
}

/**
 * The status a grant shows, and its label. `unconfirmed` is this window's memory of a revoke that
 * stopped only on this device; the daemon's own word on the grant (`revocation`, F3) wins over it
 * whenever the daemon gives one. `revokedAt` (milliseconds) dates a revoked row (F5).
 */
export function accessStatusOf(
  grant: CrewSessionGrant,
  now = Date.now(),
  unconfirmed = false,
  revokedAt: number | null = null
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
  if (state === 'expired')
    return {
      status: 'expired',
      label: endedByWorkspace(grant)
        ? accessCopy.status.endedSettingsChanged
        : accessCopy.status.expired,
    };
  const waiting = grant.revocation !== undefined ? grant.revocation === 'unconfirmed' : unconfirmed;
  if (waiting) return { status: 'unconfirmed', label: accessCopy.status.unconfirmed };
  return {
    status: 'revoked',
    label:
      revokedAt !== null
        ? accessCopy.status.revokedAt(formatExpiry(revokedAt / 1000, now))
        : accessCopy.status.revoked,
  };
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
  // When the revoke was confirmed: a remembered row carries it; a listed row finds it in what this
  // device remembers of the same run (F5).
  const remembered =
    typeof grant.revoked_at === 'number'
      ? grant.revoked_at
      : (input.pastAccess?.entries.find(
          (entry) =>
            input.pastAccess?.connectionId === grant.connection_id &&
            entry.session_id === grant.session_id &&
            entry.run_id === grant.run_id
        )?.revoked_at ?? null);
  const { status, label } = accessStatusOf(
    grant,
    now,
    input.isUnconfirmed?.(grant.connection_id, grant.session_id) ?? false,
    remembered
  );
  // A task's grant ends when the task does, which is how a task that did its work ends: not
  // "Revoked" (nobody revoked it) and not a failure. One the workspace ended because Crew's
  // settings changed says so, task or chat, as the CLI does.
  const ended =
    kind === 'task' && (status === 'revoked' || status === 'expired') && !endedByWorkspace(grant);
  const startedAt = kind === 'task' ? taskStartedAt(grant, run) : null;
  const detail =
    kind === 'task'
      ? accessCopy.taskDetail(
          startedAt === null ? null : formatExpiry(startedAt, now),
          taskFirstWords(grant.session_name)
        ) || null
      : null;
  const extraSources = new Set(grant.source_channels.filter((id) => id !== grant.channel_id)).size;
  return {
    key: `${grant.connection_id}\n${grant.session_id}`,
    connectionId: grant.connection_id,
    sessionId: grant.session_id,
    runId: grant.run_id,
    kind,
    title: kind === 'task' ? accessCopy.yourTask : (chatTitle ?? accessCopy.untitled),
    detail,
    chatTitle,
    channelId: grant.channel_id,
    destination:
      channelLabelsById.get(grant.channel_id) ??
      (sanitizeDisplayText(grantDestinationLabel(grant)) || accessCopy.unknownChannel),
    extraSources,
    status,
    statusLabel: ended ? accessCopy.status.ended : label,
    expiresAt: typeof grant.expires_at === 'number' ? grant.expires_at : null,
    revokedAt: status === 'revoked' ? remembered : null,
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
 * recently granted first), then expired and revoked — the remembered revokes of
 * `input.pastAccess` among them. `channelId` keeps only the grants that may post in or read that
 * channel.
 */
export function accessRows(
  grants: readonly CrewSessionGrant[],
  input: AccessRowInput & { channelId?: string } = {}
): AccessRow[] {
  const labels = channelLabels(input.snapshot);
  const inChannel = (grant: CrewSessionGrant) =>
    !input.channelId ||
    grant.channel_id === input.channelId ||
    grant.source_channels.includes(input.channelId);
  const listed = grants.filter(inChannel).map((grant) => accessRow(grant, input, labels));
  // A remembered revoke is a fact about a run this device saw stopped: never "Stopped on this
  // device" (that mark is per chat, and may be the newer grant's), and keyed by its run, since the
  // same chat may also be listed.
  const remembered = input.pastAccess
    ? pastAccessGrants(input.pastAccess.connectionId, input.pastAccess.entries, grants)
        .filter(inChannel)
        .map((grant) => ({
          ...accessRow(grant, { ...input, isUnconfirmed: undefined }, labels),
          key: `${grant.connection_id}\n${grant.session_id}\n${grant.run_id}`,
        }))
    : [];
  return [...listed, ...remembered].sort(
    (a, b) =>
      rank(a) - rank(b) ||
      (b.expiresAt ?? 0) - (a.expiresAt ?? 0) ||
      (b.revokedAt ?? 0) - (a.revokedAt ?? 0) ||
      a.title.localeCompare(b.title) ||
      a.sessionId.localeCompare(b.sessionId)
  );
}

/** Rows shown at rest, and the revoked, expired and ended rows behind "Show past access (n)". */
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
