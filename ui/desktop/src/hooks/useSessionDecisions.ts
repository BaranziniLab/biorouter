import { useEffect, useMemo, useRef, useState } from 'react';
import type { Message } from '../api';
import {
  decisionRevision,
  decisionRevisionHasActivity,
  fetchDecisionSettings,
  fetchDecisionSummary,
  type DecisionSettings,
  type SessionDecisionSummary,
} from '../utils/sessionDecisions';

/** How often the summary is re-read while a turn runs in a chat that uses decision models. */
export const DECISION_POLL_MS = 3000;

/** A decide batch and its turn marker land within milliseconds; one read covers them. */
const COALESCE_MS = 80;

export interface SessionDecisions {
  /** `null` when the daemon has no decisions route (it predates them). */
  summary: SessionDecisionSummary | null;
  /** `null` when the daemon has no settings route. */
  settings: DecisionSettings | null;
}

type Snapshot = {
  sessionId: string;
  data: SessionDecisions | null;
  /** True until this session's first read has answered. */
  loading: boolean;
  /** The last read failed; `data` is the last good answer. */
  error: boolean;
};

const EMPTY: Snapshot = { sessionId: '', data: null, loading: false, error: false };

export interface SessionDecisionsState {
  data: SessionDecisions | null;
  loading: boolean;
  error: boolean;
  /** Read again now, settings included (after the Monitors dialog saves, or Retry). */
  refresh: () => void;
  /**
   * Draw `settings` as the chat's settings now: the value a save stored, so the
   * caller's copy is current even while the hook is not reading (summary hidden,
   * no turn running), when `refresh` reads nothing.
   */
  applySettings: (settings: DecisionSettings) => void;
}

/** Whether the chat uses decision models at all: calls on record, or a monitor switched on. */
export function usesDecisions(data: SessionDecisions | null | undefined): boolean {
  return (
    !!data &&
    ((data.summary?.total ?? 0) > 0 || !!data.settings?.monitors.some((monitor) => monitor.enabled))
  );
}

function same(a: SessionDecisions | null, b: SessionDecisions | null): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

/**
 * A chat's decision activity for the Chat summary: the aggregates per source
 * (`GET /sessions/{id}/decisions/summary`) and its monitors
 * (`GET /sessions/{id}/decision-settings`). Never one row per call: the summary
 * is aggregated on the daemon, and only the decision history view pages
 * through the calls themselves.
 *
 * Gated as the subagent roster is (`useSessionSubagents`): it reads only while
 * the summary is shown (`visible`) or the chat's turn runs (`turnRunning`). It
 * re-reads when a decision-making tool call answers (a `decide` batch, a
 * `workspace_orchestrate` call that may have run a phase gate) or a decision
 * notice lands (`decisionRevision`).
 *
 * While a turn runs it polls every 3s. The settings are read on every poll, in
 * every chat: writers other than this window change them and leave nothing in
 * this chat's transcript to re-read on (a parent agent's `workspace_monitor`, a
 * parent's save copying its monitors into running subagents, a workflow's seed,
 * `biorouter session monitors`, another window). The summary is polled only in
 * a chat that has a monitor switched on or has made decision calls; a monitor
 * that arrives mid-turn starts that from the next poll.
 *
 * One read at a time; an answer that draws the same as the last sets no state;
 * a stale read is aborted; a failed read keeps the last good data. The
 * person's proof rides every request (`fetchDecision*`).
 */
export function useSessionDecisions(
  sessionId: string,
  messages: Message[],
  { visible, turnRunning }: { visible: boolean; turnRunning: boolean }
): SessionDecisionsState {
  const [snapshot, setSnapshot] = useState<Snapshot>(EMPTY);
  const [retry, setRetry] = useState(0);

  const current = snapshot.sessionId === sessionId ? snapshot : null;
  const enabled = !!sessionId && (visible || turnRunning);
  const revision = useMemo(() => (enabled ? decisionRevision(messages) : ''), [messages, enabled]);

  const pollInputs = useRef({ turnRunning, revision });
  pollInputs.current = { turnRunning, revision };
  const lastData = useRef<SessionDecisions | null>(null);
  lastData.current = current?.data ?? null;

  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let last = lastData.current;
    // The first read of each run reads both; a poll reads the summary only in
    // a chat that uses decision models. A route the daemon answered 404 (it
    // predates the route) is not asked again in this run.
    let first = true;
    let summaryMissing = false;
    let settingsMissing = false;
    setSnapshot((previous) =>
      previous.sessionId === sessionId
        ? previous
        : { sessionId, data: null, loading: true, error: false }
    );

    const watched = () =>
      usesDecisions(last) || decisionRevisionHasActivity(pollInputs.current.revision);
    const keepPolling = () =>
      pollInputs.current.turnRunning && ((watched() && !summaryMissing) || !settingsMissing);

    const read = async () => {
      try {
        const readSummary = first || (watched() && !summaryMissing);
        const [summary, settings] = await Promise.all([
          readSummary
            ? fetchDecisionSummary(sessionId, controller.signal)
            : Promise.resolve({ data: last?.summary ?? null, missing: summaryMissing }),
          !settingsMissing
            ? fetchDecisionSettings(sessionId, controller.signal)
            : Promise.resolve({ data: last?.settings ?? null, missing: settingsMissing }),
        ]);
        if (controller.signal.aborted) return;
        first = false;
        summaryMissing = summary.missing;
        settingsMissing = settings.missing;
        const next: SessionDecisions | null =
          summary.missing && settings.missing
            ? null
            : { summary: summary.data, settings: settings.data };
        last = next;
        setSnapshot((previous) =>
          previous.sessionId === sessionId &&
          !previous.loading &&
          !previous.error &&
          same(previous.data, next)
            ? previous
            : { sessionId, data: next, loading: false, error: false }
        );
      } catch {
        if (controller.signal.aborted) return;
        setSnapshot((previous) =>
          previous.sessionId === sessionId
            ? previous.error && !previous.loading
              ? previous
              : { ...previous, loading: false, error: true }
            : { sessionId, data: null, loading: false, error: true }
        );
      }
      if (!controller.signal.aborted && keepPolling()) timer = setTimeout(read, DECISION_POLL_MS);
    };

    timer = setTimeout(read, COALESCE_MS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
    // `retry` is a trigger. `turnRunning` restarts the loop, so a turn that
    // starts or ends gets one read of its own.
  }, [sessionId, enabled, revision, retry, turnRunning]);

  return {
    data: current?.data ?? null,
    loading: current ? current.loading : enabled,
    error: current?.error ?? false,
    refresh: () => setRetry((value) => value + 1),
    applySettings: (settings) =>
      setSnapshot((previous) =>
        previous.sessionId === sessionId
          ? { ...previous, data: { summary: previous.data?.summary ?? null, settings } }
          : { sessionId, data: { summary: null, settings }, loading: false, error: false }
      ),
  };
}
