import { useSyncExternalStore } from 'react';

/**
 * The join screen's reconnect and claim counters, per saved connection, held outside the card.
 *
 * `JoinStatusCard` reconnects by itself when the join route cannot reach the workspace, and that
 * connect unmounts it: while a connect is pending the controller's screen is `connecting`, so
 * `ConnectingCard` replaces the card, and the card comes back as a NEW mount once the connect
 * settles and the status still says the join is unfinished. A counter kept in the card (a ref or
 * state) is thrown away with that mount, so every attempt started from zero: the new mount polled,
 * claimed, found no connection and ran its "first" automatic reconnect again — an unattended loop
 * of SSH connects and claims signed with the device key (D15(7); measured at 40 of each in 20
 * seconds). Kept here, "once per approval" and "once per loss" survive the remount their own
 * connect causes.
 *
 * In memory only, deliberately never `localStorage` (unlike `joinContext`): these counters bound
 * unattended work in THIS process. They are not preferences, and a restart must start clean.
 */
export interface JoinClaimState {
  /** The approval a claim was sent for; no claim is sent for it again by itself. */
  claimedFor: string | null;
  /** The approval whose claim has used its one automatic reconnect. */
  claimReconnectFor: string | null;
  /** The approval whose claim still found no connection after that reconnect: offer Reconnect. */
  claimLost: string | null;
  /** The poll path's one automatic reconnect for this loss of the connection has run. */
  reconnectTried: boolean;
  /** A connect the card started is still running. */
  connecting: boolean;
}

const EMPTY: Readonly<JoinClaimState> = Object.freeze({
  claimedFor: null,
  claimReconnectFor: null,
  claimLost: null,
  reconnectTried: false,
  connecting: false,
});

const states = new Map<string, Readonly<JoinClaimState>>();
const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((listener) => listener());
}

/** One connection's counters (a frozen empty state when there are none). */
export function readJoinClaim(connectionId: string | null | undefined): Readonly<JoinClaimState> {
  if (!connectionId) return EMPTY;
  return states.get(connectionId) ?? EMPTY;
}

/** Merge `patch` into a connection's counters; subscribers hear of a real change only. */
export function updateJoinClaim(connectionId: string, patch: Partial<JoinClaimState>): void {
  if (!connectionId) return;
  const current = readJoinClaim(connectionId);
  const next: JoinClaimState = { ...current, ...patch };
  const keys = Object.keys(next) as (keyof JoinClaimState)[];
  if (keys.every((key) => next[key] === current[key])) return;
  states.set(connectionId, Object.freeze(next));
  notify();
}

/** Forget a connection's counters entirely (its join finished). */
export function forgetJoinClaim(connectionId: string): void {
  if (!states.delete(connectionId)) return;
  notify();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** A connection's counters, re-rendering the caller when they change. */
export function useJoinClaim(connectionId: string | null | undefined): Readonly<JoinClaimState> {
  return useSyncExternalStore(
    subscribe,
    () => readJoinClaim(connectionId),
    () => readJoinClaim(connectionId)
  );
}

/** Tests only: forget every connection's counters. */
export function resetJoinClaimForTests(): void {
  states.clear();
  notify();
}
