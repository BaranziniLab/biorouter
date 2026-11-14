import { useSyncExternalStore } from 'react';
import { getProviderSetupAccess } from '../api';
import { HOST_MANAGED_MODEL_REASON } from '../components/privacy/hostManagedModelCopy';
import { isBrowserSurface } from './surface';

/**
 * SD-18: may the person in this browser tab set up providers and the host's default model?
 *
 * SD-1 used to answer "never" for every browser served by `biorouter serve`, so every provider
 * and model control there was disabled with a note sending the person to `biorouter configure`
 * on the host. The daemon now admits the served page itself (its HttpOnly cookie plus the Fetch
 * Metadata every same-origin `fetch` carries) to the provider routes, and says whether it will
 * through `GET /config/provider_setup`. This module holds that answer for the life of the page.
 *
 * What stays the host's even when the answer is yes: moving an OPEN chat to another model
 * (`/agent/update_provider`). In a browser an open chat keeps the model it started on, and the
 * person changes the default new chats start on instead. That is not this module's question;
 * the model controls ask {@link isBrowserSurface} for it.
 *
 * ⚠ **Fail closed.** Until the daemon has answered, and whenever it could not, a browser surface
 * is host-managed: the controls show today's host-managed note rather than offering a write the
 * daemon might refuse. Only a successful answer is cached; a failed read is asked again on the
 * next explicit {@link loadProviderSetupAccess} (the provider check on launch does), so one
 * dropped request cannot pin the page to "managed on the host" for as long as it stays open.
 *
 * The desktop application never asks: its daemon proves the person with the user-action key,
 * and every provider control there works as it always has.
 */

/** The daemon's answer, as this page holds it. */
export interface ProviderSetupAccessState {
  allowed: boolean;
  /** One person-facing sentence, present only when not allowed and the daemon gave one. */
  reason?: string;
}

type Phase = 'unknown' | 'loading' | 'answered' | 'failed';

let phase: Phase = 'unknown';
let answer: ProviderSetupAccessState = { allowed: false };
let inFlight: Promise<ProviderSetupAccessState> | null = null;
/** Bumped by a reset, so a read that was out when it happened cannot write over it. */
let generation = 0;
/** Bumped on every change, so `useSyncExternalStore` sees a new snapshot. */
let version = 0;
const listeners = new Set<() => void>();

function notify() {
  version += 1;
  for (const listener of [...listeners]) listener();
}

function readVersion(): number {
  return version;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The desktop's answer: nothing is managed on a host. */
const DESKTOP_ANSWER: ProviderSetupAccessState = { allowed: true };

/**
 * Ask the daemon once per page. On the desktop this resolves at once and never fetches.
 *
 * Resolves with the answer it settled on; a failure resolves `{ allowed: false }` (it never
 * rejects), and leaves the next call free to ask again.
 */
export function loadProviderSetupAccess(): Promise<ProviderSetupAccessState> {
  if (!isBrowserSurface()) return Promise.resolve(DESKTOP_ANSWER);
  if (phase === 'answered') return Promise.resolve(answer);
  if (inFlight) return inFlight;

  phase = 'loading';
  const asked = generation;
  const request = (async (): Promise<ProviderSetupAccessState> => {
    let settled: ProviderSetupAccessState;
    let settledPhase: Phase;
    try {
      const { data } = await getProviderSetupAccess({ throwOnError: true });
      const reason =
        typeof data?.reason === 'string' && data.reason.trim() ? data.reason.trim() : undefined;
      settled = data?.allowed === true ? { allowed: true } : { allowed: false, reason };
      settledPhase = 'answered';
    } catch (error) {
      // An older daemon without the route answers 404, which is the same answer as a refusal:
      // the host owns the choice, as it always did.
      console.error('Could not read whether this browser may set up providers:', error);
      settled = { allowed: false };
      settledPhase = 'failed';
    }
    // A reset while this was out (tests only) leaves the answer to the next read.
    if (asked !== generation) return settled;
    answer = settled;
    phase = settledPhase;
    inFlight = null;
    notify();
    return answer;
  })();
  inFlight = request;
  return request;
}

/**
 * Are provider and default-model choices made on the host rather than in this renderer?
 *
 * `false` on the desktop. On a browser surface `true` until the daemon has said yes, so a
 * control rendered before the answer arrived is disabled with its reason, never live and then
 * refused.
 */
export function providersAreHostManaged(): boolean {
  if (!isBrowserSurface()) return false;
  return !(phase === 'answered' && answer.allowed);
}

/**
 * The daemon's sentence for why this browser may not set up providers, or `undefined` when it
 * gave none (or may, or has not answered, or this is the desktop).
 */
export function providerSetupRefusal(): string | undefined {
  if (!isBrowserSurface() || phase !== 'answered' || answer.allowed) return undefined;
  return answer.reason;
}

/**
 * The reason a browser-served provider control is disabled, or `null` where it is not (the
 * desktop, and a browser the daemon lets set up providers).
 *
 * `null` rather than `''` is deliberate: a caller spreading this into a `title`
 * wants the attribute absent on the desktop, not present and empty.
 */
export function hostManagedModelReason(): string | null {
  return providersAreHostManaged() ? HOST_MANAGED_MODEL_REASON : null;
}

/**
 * {@link providersAreHostManaged} for a component: re-renders when the daemon's answer arrives.
 *
 * ⚠ **It never asks the daemon itself.** `ProviderGuard` wraps the whole app and asks on launch,
 * on every route, a resume link and Crew included, so the answer is in (or on its way) before
 * any of these controls mounts. A read per mount would turn one dropped request into one per
 * control, and would make every component that draws a model control a network caller.
 */
export function useProvidersAreHostManaged(): boolean {
  // The same snapshot on the server, for a component rendered to a string in a test.
  useSyncExternalStore(subscribe, readVersion, readVersion);
  return providersAreHostManaged();
}

/** For tests: forget the answer, as a fresh page would. */
export function resetProviderSetupAccessForTests(): void {
  generation += 1;
  phase = 'unknown';
  answer = { allowed: false };
  inFlight = null;
  notify();
}
