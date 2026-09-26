import { useMemo, useSyncExternalStore } from 'react';
import type { CrewGrantKind, CrewRevokeResult, CrewSessionGrant } from '../api/grants';
import { isRecord } from '../api/parse';

/**
 * Grants this device saw revoked, remembered so "Show past access" keeps them (live QA round 4,
 * Q4-12).
 *
 * The daemon lists one grant per chat (`session_grants` in `crew/mod.rs`), so granting a revoked
 * chat again replaced its revoked row, and the Access tab forgot that the earlier grant ever
 * existed. When a revoke is confirmed, the row it stopped is recorded here, and the Access tab
 * merges these rows in as "Revoked" unless the daemon's list still holds the same run.
 *
 * Display only: nothing reads this to decide what a chat may do, and the daemon's list always wins
 * for a run it still holds. Per saved connection, in `localStorage` under
 * `crew:pastAccess:v1:<connectionId>`, newest first and at most {@link PAST_ACCESS_LIMIT} rows.
 * Every read and write is wrapped: storage that is blocked, full or corrupt reads as nothing
 * remembered, and a record that cannot be written is dropped.
 */

export interface PastAccessEntry {
  session_id: string;
  run_id: string;
  /** The chat's title when it was revoked, or `null` when the daemon did not know it. */
  session_name: string | null;
  /** The channel the grant posted in. */
  channel_id: string;
  /** When this device saw the revoke confirmed, in milliseconds. */
  revoked_at: number;
  kind?: CrewGrantKind;
  /** The channels it could read, the destination included. */
  source_channels?: string[];
}

/** The most rows kept per connection; the oldest go first. */
export const PAST_ACCESS_LIMIT = 100;
const STORAGE_PREFIX = 'crew:pastAccess:v1:';
/** Bounds on what one stored row may hold: IDs, a title and a short channel list. */
const MAX_ID_LENGTH = 256;
const MAX_NAME_LENGTH = 512;
const MAX_SOURCES = 32;

export const pastAccessStorageKey = (connectionId: string) => `${STORAGE_PREFIX}${connectionId}`;

function storage(): Storage | null {
  try {
    return typeof window !== 'undefined' ? window.localStorage : null;
  } catch {
    return null;
  }
}

const idOf = (value: unknown): string | null =>
  typeof value === 'string' && value && value.length <= MAX_ID_LENGTH ? value : null;

function entryFrom(value: unknown): PastAccessEntry | null {
  if (!isRecord(value)) return null;
  const session_id = idOf(value.session_id);
  const run_id = idOf(value.run_id);
  const channel_id = idOf(value.channel_id);
  const revoked_at = value.revoked_at;
  if (
    !session_id ||
    !run_id ||
    !channel_id ||
    typeof revoked_at !== 'number' ||
    !Number.isFinite(revoked_at)
  )
    return null;
  const name = value.session_name;
  const entry: PastAccessEntry = {
    session_id,
    run_id,
    channel_id,
    revoked_at,
    session_name: typeof name === 'string' ? name.slice(0, MAX_NAME_LENGTH) : null,
  };
  if (value.kind === 'chat' || value.kind === 'task') entry.kind = value.kind;
  if (Array.isArray(value.source_channels)) {
    const sources = value.source_channels
      .map(idOf)
      .filter((id): id is string => id !== null)
      .slice(0, MAX_SOURCES);
    if (sources.length) entry.source_channels = sources;
  }
  return entry;
}

/** What this device remembers for `connectionId`, newest first. Never throws. */
export function readPastAccess(connectionId: string | null | undefined): PastAccessEntry[] {
  if (!connectionId) return [];
  try {
    const raw = storage()?.getItem(pastAccessStorageKey(connectionId));
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const seen = new Set<string>();
    const entries: PastAccessEntry[] = [];
    for (const value of parsed) {
      const entry = entryFrom(value);
      if (!entry || seen.has(entry.run_id)) continue;
      seen.add(entry.run_id);
      entries.push(entry);
      if (entries.length >= PAST_ACCESS_LIMIT) break;
    }
    return entries;
  } catch {
    return [];
  }
}

let version = 0;
const listeners = new Set<() => void>();

function changed() {
  version += 1;
  for (const listener of [...listeners]) listener();
}

/** Record one confirmed revoke. The newest record of a run replaces an older one. Never throws. */
export function rememberPastAccess(connectionId: string, entry: PastAccessEntry): void {
  const valid = entryFrom(entry);
  if (!connectionId || !valid) return;
  const next = [
    valid,
    ...readPastAccess(connectionId).filter((item) => item.run_id !== valid.run_id),
  ].slice(0, PAST_ACCESS_LIMIT);
  try {
    storage()?.setItem(pastAccessStorageKey(connectionId), JSON.stringify(next));
  } catch {
    // Blocked or full: the record is display only, so it is dropped.
    return;
  }
  changed();
}

/** What a revoke knew about the grant it stopped: the row it was pressed on, when there was one. */
export type RevokedGrantInfo = Partial<
  Pick<CrewSessionGrant, 'run_id' | 'channel_id' | 'session_name' | 'kind' | 'source_channels'>
>;

/**
 * Record a revoke the daemon confirmed. The run is the daemon's answer's own when it names one; a
 * row that describes a different run (the chat was granted again meanwhile) is not recorded, since
 * its channel and title may not be that run's.
 */
export function rememberConfirmedRevoke(
  connectionId: string,
  sessionId: string,
  revoked: RevokedGrantInfo | null | undefined,
  answer: Pick<CrewRevokeResult, 'run_id' | 'session_id'> | null,
  now = Date.now()
): void {
  if (!revoked?.channel_id) return;
  if (answer?.session_id && answer.session_id !== sessionId) return;
  if (answer?.run_id && revoked.run_id && answer.run_id !== revoked.run_id) return;
  const runId = answer?.run_id ?? revoked.run_id;
  if (!runId) return;
  rememberPastAccess(connectionId, {
    session_id: sessionId,
    run_id: runId,
    session_name: revoked.session_name ?? null,
    channel_id: revoked.channel_id,
    revoked_at: now,
    ...(revoked.kind ? { kind: revoked.kind } : {}),
    ...(revoked.source_channels?.length ? { source_channels: revoked.source_channels } : {}),
  });
}

// ── A revoke the daemon confirmed later (final polish NEW-3) ──────────────────────────────────
//
// A revoke answered 503 stops the grant on this device, and the daemon then confirms it with the
// workspace by itself once the connection is back (F3). Only a 200 used to be recorded above, so
// that path — the one F3 created — got no time: its row read plain "Revoked", and once the chat was
// granted again the daemon's list replaced it and "Show past access" forgot it (live, runs
// f5c6dc10 and 2425afd9). Now every run this window sees waiting — its own 503, or a list that says
// `revocation: unconfirmed` — is remembered here, and the first list read that says `confirmed`
// records it, dated then: at most one list read (15 s while one is on screen) after the workspace
// answered. Process memory on purpose: it is what this window saw, and a reload honestly forgets it.

const waitingRevokes = new Map<string, RevokedGrantInfo>();
/** A run's key; `runId` '' holds a 503 whose run the surface did not know (just after Allow). */
const waitingKey = (connectionId: string, sessionId: string, runId = '') =>
  `${connectionId}\n${sessionId}\n${runId}`;

/** A revoke of this chat stopped on this device only (503): remember it until it is confirmed. */
export function noteRevokeWaiting(
  connectionId: string,
  sessionId: string,
  revoked: RevokedGrantInfo | null | undefined
): void {
  if (!connectionId || !sessionId) return;
  waitingRevokes.set(waitingKey(connectionId, sessionId, revoked?.run_id ?? ''), {
    ...(revoked ?? {}),
  });
}

/** A revoke of this chat was confirmed at once (200) and recorded then: nothing waits for it. */
export function forgetRevokeWaiting(connectionId: string, sessionId: string, runId?: string): void {
  waitingRevokes.delete(waitingKey(connectionId, sessionId));
  if (runId) waitingRevokes.delete(waitingKey(connectionId, sessionId, runId));
}

/** For tests: forget every revoke this window saw waiting. */
export function forgetWaitingRevokes(): void {
  waitingRevokes.clear();
}

const rowInfo = (row: CrewSessionGrant, seen: RevokedGrantInfo | undefined): RevokedGrantInfo => ({
  run_id: row.run_id,
  channel_id: row.channel_id,
  session_name: row.session_name ?? seen?.session_name ?? null,
  kind: row.kind ?? seen?.kind,
  source_channels: row.source_channels.length ? row.source_channels : seen?.source_channels,
});

/**
 * Read one connection's grant list as the daemon answered it: `grants`, one per chat, and
 * `replaced`, the earlier stops the daemon keeps asking about after their chat was granted again
 * (`replaced_grants`). A stopped run this window saw waiting is recorded once the list says the
 * workspace confirmed it (NEW-3). A run the workspace ended itself was not revoked by anyone, so it
 * is forgotten, never recorded as "Revoked". Display only; never throws.
 */
export function observeListedRevocations(
  connectionId: string,
  grants: readonly CrewSessionGrant[],
  replaced: readonly CrewSessionGrant[] = [],
  now = Date.now()
): void {
  try {
    const rows: [CrewSessionGrant, boolean][] = [
      ...grants.map((row): [CrewSessionGrant, boolean] => [row, true]),
      ...replaced.map((row): [CrewSessionGrant, boolean] => [row, false]),
    ];
    for (const [row, current] of rows) {
      if (!row.expired || row.connection_id !== connectionId) continue;
      const byRun = waitingKey(connectionId, row.session_id, row.run_id);
      // A 503 whose run was not known can only be the chat's current grant, never an earlier one.
      const byChat = current ? waitingKey(connectionId, row.session_id) : null;
      const seen = waitingRevokes.get(byRun) ?? (byChat ? waitingRevokes.get(byChat) : undefined);
      if (row.revocation === 'unconfirmed') {
        waitingRevokes.set(byRun, rowInfo(row, seen));
        if (byChat) waitingRevokes.delete(byChat);
        continue;
      }
      if (!seen) continue;
      waitingRevokes.delete(byRun);
      if (byChat) waitingRevokes.delete(byChat);
      if (row.revocation !== 'confirmed') continue;
      rememberConfirmedRevoke(
        connectionId,
        row.session_id,
        rowInfo(row, seen),
        { run_id: row.run_id, session_id: row.session_id },
        now
      );
    }
  } catch {
    // Display only: a record that cannot be made is dropped.
  }
}

/** Another window of this app wrote a past-access key (or cleared storage). */
function onStorage(event: StorageEvent) {
  if (event.key === null || event.key.startsWith(STORAGE_PREFIX)) changed();
}

function subscribe(listener: () => void) {
  if (listeners.size === 0 && typeof window !== 'undefined')
    window.addEventListener('storage', onStorage);
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && typeof window !== 'undefined')
      window.removeEventListener('storage', onStorage);
  };
}

/** {@link readPastAccess}, re-read whenever a revoke is recorded here or in another window. */
export function usePastAccess(connectionId: string | null | undefined): PastAccessEntry[] {
  const current = useSyncExternalStore(
    subscribe,
    () => version,
    () => version
  );
  return useMemo(
    () => readPastAccess(connectionId),
    // `current` is the store's version: a new record re-reads the rows.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [connectionId, current]
  );
}

const runKey = (sessionId: string, runId: string) => `${sessionId}\n${runId}`;

/**
 * The grants to show for what this device remembers on `connectionId`: one revoked grant per
 * remembered run the daemon's list (`listed`, any connection's) no longer holds for that chat.
 */
export function pastAccessGrants(
  connectionId: string,
  entries: readonly PastAccessEntry[],
  listed: readonly CrewSessionGrant[]
): CrewSessionGrant[] {
  const listedRuns = new Set(
    listed
      .filter((grant) => grant.connection_id === connectionId)
      .map((grant) => runKey(grant.session_id, grant.run_id))
  );
  return entries
    .filter((entry) => !listedRuns.has(runKey(entry.session_id, entry.run_id)))
    .map((entry) => ({
      session_id: entry.session_id,
      run_id: entry.run_id,
      connection_id: connectionId,
      channel_id: entry.channel_id,
      source_channels: entry.source_channels ?? [entry.channel_id],
      policy_epoch: 0,
      // Stopped: every remembered row is a revoke the daemon confirmed.
      expired: true,
      kind: entry.kind ?? 'chat',
      session_name: entry.session_name,
      expires_at: null,
      // When it was revoked, so two revokes of one chat read as two (F5).
      revoked_at: entry.revoked_at,
    }));
}
