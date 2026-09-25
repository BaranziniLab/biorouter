import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  listSessionGrants,
  revokeSessionGrant,
  type CrewRevokeResult,
  type CrewSessionGrant,
} from '../api/grants';
import { isRevocationUnconfirmed, isStaleDaemon, STALE_DAEMON_MESSAGE } from '../api/errors';
import { accessCopy } from './copy';
import { rememberConfirmedRevoke, type RevokedGrantInfo } from './pastAccess';

/**
 * Chat and task grants, as the daemon lists them (ui-redesign-spec, "Revoke", RV-R1..R3).
 *
 * The daemon decides whether a grant exists and whether it may be revoked. Nothing here gates a
 * control: a hook only reads the daemon's list, and `revoke` reports what the daemon answered.
 *
 * **No polling.** A list is fetched when the surface that shows it opens, after every action
 * taken through this module, and when another surface announces a change
 * ({@link CREW_GRANTS_CHANGED_EVENT}) — so the pane, the Access tab, the Agents section and the
 * ordinary chat stay in step without any of them asking on a timer.
 */

/** Fired on `window` after a grant was made, revoked or stopped from any surface. */
export const CREW_GRANTS_CHANGED_EVENT = 'biorouter:crew-grants-changed';

export type RevokeOutcome =
  /** 200: the daemon revoked the grant and the workspace confirmed it. */
  | { kind: 'revoked' }
  /** 503 `crew_revocation_unconfirmed`: stopped on this device, not yet confirmed remotely. */
  | { kind: 'unconfirmed'; message: string }
  /** Anything else: the grant is still active. `message` is the daemon's own text. */
  | { kind: 'not-revoked'; message: string };

export interface CrewGrantsChangedDetail {
  connectionId: string;
  sessionId: string;
  change: RevokeOutcome['kind'] | 'granted' | 'stopped';
}

// Sessions this renderer saw stopped on this device without the workspace's confirmation. It is
// what lets a row read "Stopped on this device" rather than "Revoked": the daemon's list says only
// that the grant is stopped locally. Process memory on purpose — it is a fact about requests this
// window made, and a reload honestly forgets it.
const unconfirmedRevocations = new Set<string>();
const grantKey = (connectionId: string, sessionId: string) => `${connectionId}\n${sessionId}`;

/** For tests: forget every unconfirmed revocation this window recorded. */
export function forgetUnconfirmedRevocations(): void {
  unconfirmedRevocations.clear();
}

/** Whether this window saw a revoke of this grant stop only on this device. */
export function isUnconfirmedRevocation(connectionId: string, sessionId: string): boolean {
  return unconfirmedRevocations.has(grantKey(connectionId, sessionId));
}

function record(detail: CrewGrantsChangedDetail) {
  const key = grantKey(detail.connectionId, detail.sessionId);
  if (detail.change === 'unconfirmed') unconfirmedRevocations.add(key);
  else if (detail.change === 'revoked' || detail.change === 'granted')
    unconfirmedRevocations.delete(key);
}

/** Tell every grant list and the ordinary chat that a grant changed. */
export function announceGrantsChanged(detail: CrewGrantsChangedDetail): void {
  record(detail);
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent(CREW_GRANTS_CHANGED_EVENT, { detail }));
}

/** Subscribe to {@link CREW_GRANTS_CHANGED_EVENT}. Returns the unsubscribe. */
export function onGrantsChanged(listener: (detail: CrewGrantsChangedDetail) => void): () => void {
  const handle = (event: Event) => {
    const detail = (event as CustomEvent<CrewGrantsChangedDetail>).detail;
    if (detail && typeof detail.connectionId === 'string') listener(detail);
  };
  window.addEventListener(CREW_GRANTS_CHANGED_EVENT, handle);
  return () => window.removeEventListener(CREW_GRANTS_CHANGED_EVENT, handle);
}

/**
 * What a failed revoke means. Only a 503 `crew_revocation_unconfirmed` is "stopped on this
 * device"; every other failure — a 400, 403, 404, 409, a transport error, a daemon without the
 * route — leaves the grant active, and says so with the daemon's own words.
 */
export function revokeOutcomeFrom(failure: unknown): RevokeOutcome {
  const message =
    failure instanceof Error && failure.message.trim()
      ? failure.message
      : accessCopy.revokeFallback;
  if (isRevocationUnconfirmed(failure)) return { kind: 'unconfirmed', message };
  if (isStaleDaemon(failure)) return { kind: 'not-revoked', message: STALE_DAEMON_MESSAGE };
  return { kind: 'not-revoked', message };
}

/**
 * Revoke one grant and announce the change. Resolves `revoked` only when the daemon confirmed it
 * (a 200 whose body says so); never throws. Every outcome is announced, because even a refused
 * revoke is a reason to re-read the list: a 503 means the local stop landed.
 *
 * `revoking` is the grant the person pressed Revoke on, when the surface has it: a confirmed revoke
 * of it is remembered for "Show past access" (`pastAccess.ts`, Q4-12), which the daemon's list
 * forgets once the chat is granted again. Display only; it changes nothing about the request.
 */
export async function revokeGrant(
  connectionId: string,
  sessionId: string,
  revoking?: RevokedGrantInfo | null
): Promise<RevokeOutcome> {
  let outcome: RevokeOutcome;
  let answer: CrewRevokeResult | null = null;
  try {
    answer = await revokeSessionGrant(connectionId, sessionId);
    outcome = { kind: 'revoked' };
  } catch (failure) {
    outcome = revokeOutcomeFrom(failure);
  }
  if (outcome.kind === 'revoked')
    rememberConfirmedRevoke(connectionId, sessionId, revoking, answer);
  announceGrantsChanged({ connectionId, sessionId, change: outcome.kind });
  return outcome;
}

export type GrantsStatus = 'idle' | 'loading' | 'loaded' | 'failed';

const NO_GRANTS: CrewSessionGrant[] = [];

export interface CrewGrantsView {
  /** Every listed grant across the requested connections, in the daemon's order. */
  grants: CrewSessionGrant[];
  /**
   * `loading` until the first answer; `failed` when every list failed; `loaded` otherwise (a
   * refetch keeps the previous rows on screen and stays `loaded`).
   */
  status: GrantsStatus;
  /** Set when any list failed, even if others answered. */
  error: string | null;
  /** Whether any list failed on the latest fetch. */
  anyFailed: boolean;
  /** Re-read the lists now. */
  refetch(): void;
  /**
   * Revoke through the daemon, announce it and re-read. Never throws. `revoking` describes the
   * grant for the past-access record; it defaults to the listed grant of that chat.
   */
  revoke(
    connectionId: string,
    sessionId: string,
    revoking?: RevokedGrantInfo | null
  ): Promise<RevokeOutcome>;
  /** Whether this window saw that grant stopped only on this device. */
  isUnconfirmed(connectionId: string, sessionId: string): boolean;
}

function listFailureMessage(failure: unknown): string {
  return isStaleDaemon(failure) ? STALE_DAEMON_MESSAGE : accessCopy.listFailed;
}

interface GrantsState {
  key: string;
  grants: CrewSessionGrant[];
  status: GrantsStatus;
  error: string | null;
  anyFailed: boolean;
}

const IDLE: GrantsState = { key: '', grants: [], status: 'idle', error: null, anyFailed: false };

// The last answer per connection set, per cache scope (one scope per Crew view). A surface that
// opens after another already read the list shows that answer while it reads its own, so opening
// the pane from the note, or the Access tab after the sidebar, does not flash "loading". Scoped,
// not global, so a new Crew view — or the next test — never starts from another's answer.
const lastAnswers = new WeakMap<object, Map<string, GrantsState>>();

function remembered(scope: object | undefined, key: string): GrantsState | undefined {
  return scope ? lastAnswers.get(scope)?.get(key) : undefined;
}

function remember(scope: object | undefined, state: GrantsState) {
  if (!scope || state.status !== 'loaded') return;
  const answers = lastAnswers.get(scope) ?? new Map<string, GrantsState>();
  answers.set(state.key, state);
  lastAnswers.set(scope, answers);
}

export interface UseCrewGrantsOptions {
  /** Read nothing while false. */
  enabled?: boolean;
  /**
   * Share the last answer with other lists of the same scope (any object that lives exactly as
   * long as the view, such as the controller's `subscribeSurfaceReset`). Each list still reads the
   * daemon when it opens; the shared answer only fills the moment before that read returns.
   */
  cacheScope?: object;
}

/**
 * The grants of one or more saved connections, fetched when the caller mounts (or its connections
 * change), after every revoke or grant announced by any surface, and on `refetch()`.
 */
export function useCrewGrants(
  connectionIds: readonly string[],
  options: UseCrewGrantsOptions = {}
): CrewGrantsView {
  const enabled = options.enabled ?? true;
  // Read through a ref: a scope that is a new object every render must not re-read the list.
  const scopeRef = useRef(options.cacheScope);
  scopeRef.current = options.cacheScope;
  const ids = useMemo(
    () => [...new Set(connectionIds.filter((id) => typeof id === 'string' && id))].sort(),
    // The joined key is the identity: a new array with the same ids must not refetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [connectionIds.join('\n')]
  );
  const key = ids.join('\n');
  const [nonce, setNonce] = useState(0);
  const [state, setState] = useState<GrantsState>(
    () => remembered(options.cacheScope, key) ?? IDLE
  );
  // Bumped by an announcement so the unconfirmed marks re-render even when the list is unchanged.
  const [, setMarks] = useState(0);
  const request = useRef(0);

  const refetch = useCallback(() => setNonce((value) => value + 1), []);

  useEffect(() => {
    if (!enabled) return;
    return onGrantsChanged((detail) => {
      if (!ids.includes(detail.connectionId)) return;
      setMarks((value) => value + 1);
      setNonce((value) => value + 1);
    });
  }, [enabled, ids]);

  useEffect(() => {
    if (!enabled) return;
    const current = ++request.current;
    if (ids.length === 0) {
      setState({ key, grants: [], status: 'loaded', error: null, anyFailed: false });
      return;
    }
    const controller = new AbortController();
    const scope = scopeRef.current;
    // A refetch of the same set keeps its rows; a new set starts from the scope's last answer for
    // it, else from nothing.
    setState((previous) =>
      previous.key === key && previous.status !== 'idle' && previous.status !== 'loading'
        ? previous
        : (remembered(scope, key) ?? {
            key,
            grants: [],
            status: 'loading',
            error: null,
            anyFailed: false,
          })
    );
    void Promise.allSettled(ids.map((id) => listSessionGrants(id, controller.signal))).then(
      (results) => {
        if (controller.signal.aborted || current !== request.current) return;
        const grants = results.flatMap((result) =>
          result.status === 'fulfilled' ? result.value : []
        );
        const failures = results.filter(
          (result): result is PromiseRejectedResult => result.status === 'rejected'
        );
        const next: GrantsState = {
          key,
          grants,
          status: failures.length === results.length ? 'failed' : 'loaded',
          error: failures.length ? listFailureMessage(failures[0].reason) : null,
          anyFailed: failures.length > 0,
        };
        if (failures.length === 0) remember(scope, next);
        setState(next);
      }
    );
    return () => controller.abort();
  }, [enabled, key, ids, nonce]);

  const fresh = state.key === key;
  // Read through a ref, so `revoke` stays one function while the list changes under it.
  const listed = useRef<CrewSessionGrant[]>(NO_GRANTS);
  listed.current = fresh ? state.grants : NO_GRANTS;
  const revoke = useCallback(
    (connectionId: string, sessionId: string, revoking?: RevokedGrantInfo | null) =>
      revokeGrant(
        connectionId,
        sessionId,
        revoking ??
          listed.current.find(
            (grant) => grant.connection_id === connectionId && grant.session_id === sessionId
          ) ??
          null
      ),
    []
  );

  return {
    grants: fresh ? state.grants : NO_GRANTS,
    status: !enabled ? 'idle' : fresh ? state.status : 'loading',
    error: fresh ? state.error : null,
    anyFailed: fresh ? state.anyFailed : false,
    refetch,
    revoke,
    isUnconfirmed: isUnconfirmedRevocation,
  };
}
