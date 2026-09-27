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
  /**
   * Whether to offer the server-account name (naming D2: offered, never applied silently). On a
   * read it is DERIVED, never stored: true for every connection — the host after Create and every
   * member, whenever they joined (Q3-51) — unless the person answered the offer on this computer
   * (Use, Edit… or Dismiss, kept under {@link nameOfferDismissedKey}), or the workspace had no
   * usable name to offer this session. Writing `true` makes a fresh offer (a new join or host
   * setup); writing `false` records a dismissal.
   */
  suggestName?: boolean;
}

const STORAGE_KEY = 'biorouter.crew.onboarding.v1';
const EMPTY: Readonly<JoinContext> = Object.freeze({});

/**
 * The per-connection record that the person answered the name offer. Its own key, so an answer
 * outlives the join context (which Host and Join rewrite) and a legacy stored `suggestName: false`
 * — written before the offer reached hosts and earlier members, often only because the broker had
 * nothing to offer yet — no longer silences it.
 */
export function nameOfferDismissedKey(connectionId: string): string {
  return `crew:nameOffer:dismissed:${connectionId}`;
}

let memory: Record<string, JoinContext> | null = null;
const listeners = new Set<() => void>();
/** Answers recorded while storage refused the write: they still hold for this session. */
const dismissedInMemory = new Set<string>();
/** Connections whose workspace had no usable name to offer this session (not an answer). */
const nothingToOffer = new Set<string>();
/** The name the broker offered this session, per connection, with the account it was for. */
const offeredNames = new Map<string, { username: string; name: string }>();
/** The derived read per connection, kept while its inputs stand (`useSyncExternalStore` needs it). */
const derived = new Map<
  string,
  { entry: JoinContext | undefined; suggest: boolean; value: Readonly<JoinContext> }
>();

function notify() {
  listeners.forEach((listener) => listener());
}

function nameOfferDismissed(connectionId: string): boolean {
  if (dismissedInMemory.has(connectionId)) return true;
  try {
    return globalThis.localStorage?.getItem(nameOfferDismissedKey(connectionId)) != null;
  } catch {
    return false;
  }
}

function writeNameOfferDismissed(connectionId: string, dismissed: boolean) {
  if (dismissed) dismissedInMemory.add(connectionId);
  else dismissedInMemory.delete(connectionId);
  try {
    if (dismissed) globalThis.localStorage?.setItem(nameOfferDismissedKey(connectionId), '1');
    else globalThis.localStorage?.removeItem(nameOfferDismissedKey(connectionId));
  } catch {
    // A per-viewer convenience: the in-memory answer still serves this session.
  }
}

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
  notify();
}

/** The stored entry, without the derived name offer. */
function storedEntry(connectionId: string): JoinContext | undefined {
  const entry = load()[connectionId];
  if (!entry || typeof entry !== 'object') return undefined;
  if (!('suggestName' in entry)) return entry;
  const { suggestName: _legacy, ...rest } = entry;
  return rest;
}

/**
 * The remembered context of one connection, with `suggestName` derived (see {@link JoinContext}).
 * An empty object when there is no connection. The same object is returned while nothing it
 * depends on changed.
 */
export function readJoinContext(connectionId: string | null | undefined): Readonly<JoinContext> {
  if (!connectionId) return EMPTY;
  const entry = load()[connectionId];
  const stored = entry && typeof entry === 'object' ? entry : undefined;
  const suggest = !nameOfferDismissed(connectionId) && !nothingToOffer.has(connectionId);
  const cached = derived.get(connectionId);
  if (cached && cached.entry === stored && cached.suggest === suggest) return cached.value;
  const value: Readonly<JoinContext> = Object.freeze({
    ...storedEntry(connectionId),
    suggestName: suggest,
  });
  derived.set(connectionId, { entry: stored, suggest, value });
  return value;
}

/**
 * Merge `patch` into a connection's context. A `null`/`false` value is kept as written, except
 * `suggestName`, which is not stored: `true` makes a fresh name offer, `false` records an answer.
 */
export function updateJoinContext(connectionId: string, patch: JoinContext): void {
  if (!connectionId) return;
  const { suggestName, ...rest } = patch;
  if (suggestName === true) {
    writeNameOfferDismissed(connectionId, false);
    nothingToOffer.delete(connectionId);
  } else if (suggestName === false) {
    writeNameOfferDismissed(connectionId, true);
  }
  const all = load();
  save({ ...all, [connectionId]: { ...storedEntry(connectionId), ...rest } });
}

/**
 * The person answered the name offer (Use, Edit… or Dismiss): never offer it again for this
 * connection on this computer.
 */
export function dismissNameOffer(connectionId: string): void {
  if (!connectionId) return;
  writeNameOfferDismissed(connectionId, true);
  notify();
}

/**
 * The workspace had no usable name to offer (no account name, or a broker without the
 * suggestion). Not an answer: it holds for this session only, so a later session asks again.
 */
export function noNameToOffer(connectionId: string): void {
  if (!connectionId || nothingToOffer.has(connectionId)) return;
  nothingToOffer.add(connectionId);
  notify();
}

/** The name the broker offered `username` on this connection this session, if it did. */
export function offeredName(connectionId: string, username: string): string | null {
  const offer = offeredNames.get(connectionId);
  return offer && offer.username === username ? offer.name : null;
}

/** Remember the name the broker offered, so the next surface to mount offers it at once. */
export function rememberOfferedName(connectionId: string, username: string, name: string): void {
  if (connectionId) offeredNames.set(connectionId, { username, name });
}

/** Forget a connection's context entirely, its answer to the name offer included. */
export function forgetJoinContext(connectionId: string): void {
  const all = load();
  const answered = nameOfferDismissed(connectionId);
  if (answered) writeNameOfferDismissed(connectionId, false);
  nothingToOffer.delete(connectionId);
  offeredNames.delete(connectionId);
  derived.delete(connectionId);
  if (!(connectionId in all)) {
    if (answered) notify();
    return;
  }
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
    const storage = globalThis.localStorage;
    storage?.removeItem(STORAGE_KEY);
    const answered: string[] = [];
    for (let index = 0; index < (storage?.length ?? 0); index += 1) {
      const key = storage?.key(index);
      if (key?.startsWith(nameOfferDismissedKey(''))) answered.push(key);
    }
    answered.forEach((key) => storage?.removeItem(key));
  } catch {
    // Nothing to reset.
  }
  memory = null;
  dismissedInMemory.clear();
  nothingToOffer.clear();
  offeredNames.clear();
  derived.clear();
  notify();
}
