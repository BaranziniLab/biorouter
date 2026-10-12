import { useEffect, useState, useSyncExternalStore } from 'react';
import { listSessions, type Session } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { computeNoticeCounts, shouldShowFirstRunNotice, type NoticeCounts } from './noticeCounts';

/**
 * Where "this machine has already been told" is recorded.
 *
 * ⚠ **localStorage, unlike the sibling non-private-model disclosure, which
 * records its acknowledgement in the daemon.** That one gates an action and its
 * acknowledgement is a fact about the user: DR-16 makes it carry a proof-of-user
 * header and the daemon refuses to record it without one. This one gates
 * nothing. It reports something that has already happened to data at rest, it is
 * dismissible, and the worst outcome of losing the record is that a user sees an
 * accurate one-screen summary twice. Paying for a route, a store, an OpenAPI
 * regeneration and a proof-of-user round trip to prevent that would be spending
 * the security machinery on a receipt.
 *
 * The cost is stated rather than hidden: clearing browser storage, or a second
 * machine sharing the same database, shows the notice again. Both are correct
 * behaviours for a notice whose subject is "your history changed".
 */
export const FIRST_RUN_NOTICE_ACK_KEY = 'biorouter-privacy-first-run-notice-ack';

function alreadyAcknowledged(): boolean {
  try {
    return localStorage.getItem(FIRST_RUN_NOTICE_ACK_KEY) === '1';
  } catch {
    // Storage disabled or full. Treat as "not acknowledged" — showing an
    // accurate notice one extra time is the harmless direction, and the write
    // below fails the same way, so this cannot loop silently on a launch that
    // would otherwise have been quiet.
    return false;
  }
}

function recordAcknowledged(): void {
  try {
    localStorage.setItem(FIRST_RUN_NOTICE_ACK_KEY, '1');
  } catch {
    // See above. Nothing the user can do about it and nothing to say.
  }
}

/**
 * The answer to "does the day-one notice open this launch?", as the whole window can read it.
 *
 * - `idle`: nobody has asked yet. Nothing waits on an answer nobody is working out.
 * - `pending`: the count is in flight.
 * - `show`: the migration marked chats the person can see; the counts are the notice's.
 * - `none`: no notice this launch. Acknowledged already, not due (see
 *   {@link shouldShowFirstRunNotice}), or the daemon did not answer.
 */
export type FirstRunNoticeDecision =
  | { status: 'idle' }
  | { status: 'pending' }
  | { status: 'show'; counts: NoticeCounts }
  | { status: 'none' };

const IDLE: FirstRunNoticeDecision = { status: 'idle' };
const PENDING: FirstRunNoticeDecision = { status: 'pending' };
const NONE: FirstRunNoticeDecision = { status: 'none' };

/**
 * One count per launch, owned here and not by the gate that draws the notice (FR-R4-4).
 *
 * The gate used to start its own count when it mounted, so the first-run page settled in about a
 * second and the notice rose over it a second later, on a database of five thousand chats, while a
 * person was reading the list or moving to a row. The count now starts at boot
 * ({@link startFirstRunNoticeCount}, called by `renderer.tsx` once the daemon is reachable), and the
 * first-run page waits for its answer ({@link useFirstRunNoticeHold}), so the notice and the page
 * arrive together.
 *
 * Module state, like `llamaServerStore`: it is the launch's state, and a launch is a page load.
 */
let current: FirstRunNoticeDecision = IDLE;
let run = 0;
const listeners = new Set<() => void>();

function publish(next: FirstRunNoticeDecision): void {
  current = next;
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot(): FirstRunNoticeDecision {
  return current;
}

/**
 * Start the count, once. Safe to call from anywhere and any number of times: the boot calls it, and
 * the gate calls it again on mount for a surface the boot did not reach.
 *
 * With the user's proof: without it the daemon omits every private chat, and the notice would
 * count none (issue #56, QA 2026-09-10 M1). The History population, not the one with subagents.
 *
 * ⚠ **One fetch, and it settles the question for good.** Read once per launch and only while
 * unacknowledged: a machine that has dismissed the notice does no work at all here.
 *
 * ⚠ **A daemon that does not answer says nothing and records nothing**, so the next launch asks
 * again. The alternative is burning the one-time notice on a launch where it could not have been
 * shown.
 */
export function startFirstRunNoticeCount(): void {
  if (current.status !== 'idle') return;
  if (alreadyAcknowledged()) {
    publish(NONE);
    return;
  }
  const mine = ++run;
  publish(PENDING);
  userActionHeaders()
    .then((headers) =>
      listSessions<true>({ throwOnError: true, headers, query: { include_subagents: false } })
    )
    .then((response) => {
      if (mine !== run) return;
      const counts = computeNoticeCounts(response.data.sessions as Session[]);
      publish(shouldShowFirstRunNotice(counts) ? { status: 'show', counts } : NONE);
    })
    .catch(() => {
      if (mine === run) publish(NONE);
    });
}

/** The person acknowledged the notice: remember it on this machine and stop showing it. */
export function acknowledgeFirstRunNotice(): void {
  recordAcknowledged();
  publish(NONE);
}

/** For tests: a new launch. Forgets the answer and drops any count still in flight. */
export function resetFirstRunNoticeDecisionForTests(): void {
  run += 1;
  publish(IDLE);
}

export function useFirstRunNoticeDecision(): FirstRunNoticeDecision {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/**
 * The longest the first-run page waits for the count. The same budget as the catalog's wait for the
 * coding-agent probe (`AGENT_PROBE_WAIT_MS`, 2.5 s), so no answer the page is held for can keep the
 * first screen a skeleton longer than that. A daemon that is slow to count lets the page open
 * without it and the notice opens when the count lands, which is the behaviour before the hold.
 */
export const FIRST_RUN_NOTICE_WAIT_MS = 2500;

/**
 * Should the first-run page still be holding its skeleton for the notice's answer?
 *
 * `enabled` is true while the page is drawing its catalog. The hold can only DELAY the first draw:
 * once it ends (the answer came, the budget ran out, or nobody was counting when the page
 * appeared) it is spent for good. A count that starts later, or a reset, never takes a drawn page
 * back to a skeleton under the pointer.
 */
export function useFirstRunNoticeHold(
  enabled: boolean,
  waitMs: number = FIRST_RUN_NOTICE_WAIT_MS
): boolean {
  const decision = useFirstRunNoticeDecision();
  const [spent, setSpent] = useState(false);
  const holding = enabled && !spent && decision.status === 'pending';
  useEffect(() => {
    if (!enabled || spent) return;
    if (!holding) {
      setSpent(true);
      return;
    }
    const timer = window.setTimeout(() => setSpent(true), waitMs);
    return () => window.clearTimeout(timer);
  }, [enabled, holding, spent, waitMs]);
  return holding;
}
