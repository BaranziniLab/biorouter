import { useSyncExternalStore } from 'react';

/**
 * What this computer remembers about a join or a host setup it started, per saved connection.
 *
 * A per-viewer convenience in `localStorage`, never authority and never shared: the labels are the
 * invitation's display metadata (who invited you, what the workspace is called) so the join screens
 * can name the host before the workspace will say anything to a non-member, and the two flags keep
 * a half-finished flow findable after the dialog that started it closed. Every read and write is
 * guarded — a private window, blocked site data or a preview can make storage throw or come back
 * empty, and the screens then fall back to plainer words ("your host").
 */
export interface JoinContext {
  workspaceName?: string | null;
  hostUsername?: string | null;
  hostDisplayName?: string | null;
  /** The joiner's own username on the server, when the SSH target does not carry it. */
  username?: string | null;
  /** Set when Join saved this connection; cleared once the workspace verifies this computer. */
  joining?: boolean;
  /** Set when Host saved this connection; cleared once `auth.bootstrap` succeeded. */
  hostSetup?: boolean;
  /** Offer the server-account name once, after this computer joins. */
  suggestName?: boolean;
}

const STORAGE_KEY = 'biorouter.crew.onboarding.v1';
const EMPTY: Readonly<JoinContext> = Object.freeze({});

let memory: Record<string, JoinContext> | null = null;
const listeners = new Set<() => void>();

function load(): Record<string, JoinContext> {
  if (memory) return memory;
  let stored: Record<string, JoinContext> = {};
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed))
      stored = parsed as Record<string, JoinContext>;
  } catch {
    stored = {};
  }
  memory = stored;
  return memory;
}

function save(next: Record<string, JoinContext>) {
  memory = next;
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Storage is a convenience; the in-memory copy still serves this session.
  }
  listeners.forEach((listener) => listener());
}

/** The remembered context of one connection (an empty object when there is none). */
export function readJoinContext(connectionId: string | null | undefined): Readonly<JoinContext> {
  if (!connectionId) return EMPTY;
  const entry = load()[connectionId];
  return entry && typeof entry === 'object' ? entry : EMPTY;
}

/** Merge `patch` into a connection's context. A `null`/`false` value is kept as written. */
export function updateJoinContext(connectionId: string, patch: JoinContext): void {
  if (!connectionId) return;
  const all = load();
  save({ ...all, [connectionId]: { ...readJoinContext(connectionId), ...patch } });
}

/** Forget a connection's context entirely. */
export function forgetJoinContext(connectionId: string): void {
  const all = load();
  if (!(connectionId in all)) return;
  const next = { ...all };
  delete next[connectionId];
  save(next);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** A connection's context, re-rendering the caller when any context changes. */
export function useJoinContext(connectionId: string | null | undefined): Readonly<JoinContext> {
  return useSyncExternalStore(
    subscribe,
    () => readJoinContext(connectionId),
    () => readJoinContext(connectionId)
  );
}

/** Tests only: drop the in-memory copy so the next read goes back to storage. */
export function resetJoinContextForTests(): void {
  try {
    globalThis.localStorage?.removeItem(STORAGE_KEY);
  } catch {
    // Nothing to reset.
  }
  memory = null;
  listeners.forEach((listener) => listener());
}
