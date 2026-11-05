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
 * notice lands (`decisionRevision`), and it polls the summary every 3s only
 * while a turn runs in a chat that has monitors switched on or has made decision
 * calls. The settings are read once per enabled stretch, on a revision and on
 * `refresh`, not on every poll: they change only when someone saves them.
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
    let readSettings = true;
    setSnapshot((previous) =>
      previous.sessionId === sessionId
        ? previous
        : { sessionId, data: null, loading: true, error: false }
    );

    const keepPolling = () => {
      const inputs = pollInputs.current;
      return (
        inputs.turnRunning && (usesDecisions(last) || decisionRevisionHasActivity(inputs.revision))
      );
    };

    const read = async () => {
      try {
        const [summary, settings] = await Promise.all([
          fetchDecisionSummary(sessionId, controller.signal),
          readSettings
            ? fetchDecisionSettings(sessionId, controller.signal)
            : Promise.resolve({ data: last?.settings ?? null, missing: false }),
        ]);
        if (controller.signal.aborted) return;
        readSettings = false;
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
