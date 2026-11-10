import { useEffect, useMemo, useRef, useState } from 'react';
import { getSessionSubagents, type Message } from '../api';
import { userActionHeaders } from '../utils/userAction';
import {
  hasActiveSubagent,
  parseSessionSubagents,
  rosterRevisionHasCalls,
  subagentRosterRevision,
  type SessionSubagents,
} from '../utils/sessionSubagents';

/** How often the roster is re-read while a child is active or the chat's turn runs. */
export const SUBAGENT_POLL_MS = 2000;

/** Parallel spawns land as separate tool responses within a few milliseconds; one read covers them. */
const COALESCE_MS = 80;

/**
 * `GET /sessions/{session_id}/subagents`, through the generated client.
 *
 * The user-action proof goes with it: the daemon filters private children
 * through `lists_session`, and without the proof they would silently vanish
 * from a private chat's list. The answer is parsed defensively rather than
 * trusted as the generated type, so a status a newer daemon adds reads as
 * unknown instead of breaking the panel.
 */
async function fetchSessionSubagents(
  sessionId: string,
  signal: AbortSignal
): Promise<{ data: SessionSubagents | null; missing: boolean }> {
  const headers = await userActionHeaders();
  if (signal.aborted) return { data: null, missing: false };
  const response = await getSessionSubagents({
    path: { session_id: sessionId },
    headers,
    signal,
  });
  // A daemon that predates the route answers 404: no roster to show, and
  // nothing a person can fix by pressing Retry.
  if (response.response?.status === 404) return { data: null, missing: true };
  if (response.error !== undefined || response.data === undefined)
    throw new Error('Subagents unavailable');
  const data = parseSessionSubagents(response.data as unknown);
  if (!data) throw new Error('Subagents unavailable');
  return { data, missing: false };
}

type Snapshot = {
  sessionId: string;
  data: SessionSubagents | null;
  /** True until this session's first read has answered. */
  loading: boolean;
  /** The last read failed; `data` is the last good answer. */
  error: boolean;
};

const EMPTY: Snapshot = { sessionId: '', data: null, loading: false, error: false };

export interface SessionSubagentsState {
  data: SessionSubagents | null;
  loading: boolean;
  error: boolean;
  refresh: () => void;
}

function hasRoster(data: SessionSubagents | null | undefined): boolean {
  return !!data && (data.subagents.length > 0 || data.plan !== null);
}

/**
 * What the chat reads of the roster while the summary is hidden: which children
 * there are and their states (the header button's running count, and whether
 * to keep polling), and whether a plan exists. Nothing else is drawn then.
 */
function hiddenRosterKey(data: SessionSubagents | null): string {
  if (!data) return 'null';
  return JSON.stringify([
    data.plan !== null,
    data.subagents.map((row) => [row.sessionId, row.status]),
  ]);
}

/**
 * Two answers that would draw the same thing. A 30-row roster is a few KB of
 * JSON. While the summary is hidden only what the chat still draws counts: an
 * active child's run time and quiet time are computed fresh on every read, and
 * its token and message counts move as it works, so a full comparison would
 * never match and every poll would re-render the chat for nothing.
 */
function sameRoster(
  a: SessionSubagents | null,
  b: SessionSubagents | null,
  visible: boolean
): boolean {
  if (a === b) return true;
  return visible
    ? JSON.stringify(a) === JSON.stringify(b)
    : hiddenRosterKey(a) === hiddenRosterKey(b);
}

/**
 * A chat's subagents and phase plan for the Chat summary, kept live while it
 * matters and quiet otherwise.
 *
 * It reads while the summary is shown (`visible`: the rail or its popover) or
 * the chat's turn runs (`turnRunning`), the to-do list's gate
 * (`useSessionTodos`). It re-reads when a roster tool call answers (a spawn, the
 * plan tool, a watch, a close; `subagentRosterRevision`), and it polls every
 * 2s while any child is queued, running or being stopped, or while the turn runs
 * in a chat that has subagents. A background child that outlives its parent's
 * turn therefore stays current until it ends, then the polling stops.
 *
 * One read at a time: the next poll is scheduled only after a read settles, and
 * an answer that would draw the same as the last one sets no state, so a poll
 * that changed nothing visible does not re-render the chat. While the summary is
 * hidden that is only the children and their states (`sameRoster`); showing it
 * again puts the newest answer up at once. A stale read is aborted; a late
 * answer for a superseded read or another session is ignored. A failed read
 * keeps the last good data.
 */
export function useSessionSubagents(
  sessionId: string,
  messages: Message[],
  { visible, turnRunning }: { visible: boolean; turnRunning: boolean }
): SessionSubagentsState {
  const [snapshot, setSnapshot] = useState<Snapshot>(EMPTY);
  const [retry, setRetry] = useState(0);

  const current = snapshot.sessionId === sessionId ? snapshot : null;
  const enabled = !!sessionId && (visible || turnRunning || hasActiveSubagent(current?.data));
  const revision = useMemo(
    () => (enabled ? subagentRosterRevision(messages) : ''),
    [messages, enabled]
  );

  // Read by the poll loop when a read settles, so a new transcript or a turn
  // ending decides the NEXT poll without restarting the loop.
  const pollInputs = useRef({ turnRunning, revision, visible });
  pollInputs.current = { turnRunning, revision, visible };
  // This session's last good answer, where a restarted loop starts from.
  const lastData = useRef<SessionSubagents | null>(null);
  lastData.current = current?.data ?? null;
  // The newest answer read, which a hidden summary may not have stored.
  const newest = useRef<{ sessionId: string; data: SessionSubagents | null } | null>(null);

  // Shown again: what was read while hidden goes up now, not at the next poll.
  useEffect(() => {
    if (!visible) return;
    const read = newest.current;
    if (!read || read.sessionId !== sessionId) return;
    setSnapshot((previous) =>
      previous.sessionId === sessionId &&
      !previous.loading &&
      !previous.error &&
      !sameRoster(previous.data, read.data, true)
        ? { ...previous, data: read.data }
        : previous
    );
  }, [visible, sessionId]);

  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let last = lastData.current;
    setSnapshot((previous) =>
      previous.sessionId === sessionId
        ? previous
        : { sessionId, data: null, loading: true, error: false }
    );

    const keepPolling = () => {
      const inputs = pollInputs.current;
      return (
        hasActiveSubagent(last) ||
        (inputs.turnRunning && (hasRoster(last) || rosterRevisionHasCalls(inputs.revision)))
      );
    };

    const read = async () => {
      try {
        const { data, missing } = await fetchSessionSubagents(sessionId, controller.signal);
        if (controller.signal.aborted) return;
        const next = missing ? null : data;
        last = next;
        newest.current = { sessionId, data: next };
        const { visible: shown } = pollInputs.current;
        setSnapshot((previous) =>
          previous.sessionId === sessionId &&
          !previous.loading &&
          !previous.error &&
          sameRoster(previous.data, next, shown)
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
      if (!controller.signal.aborted && keepPolling()) timer = setTimeout(read, SUBAGENT_POLL_MS);
    };

    timer = setTimeout(read, COALESCE_MS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
    // `retry` is a trigger, read only to re-run the read. `turnRunning` restarts
    // the loop, so a turn that starts or ends gets one read of its own.
  }, [sessionId, enabled, revision, retry, turnRunning]);

  return {
    data: current?.data ?? null,
    loading: current ? current.loading : enabled,
    error: current?.error ?? false,
    refresh: () => setRetry((value) => value + 1),
  };
}
