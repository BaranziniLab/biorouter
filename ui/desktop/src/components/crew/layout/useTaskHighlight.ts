import { useCallback, useEffect, useRef, useState } from 'react';
import type { ObservedRun } from '../crewApi';
import { useCrew, useCrewSurfaceReset } from '../state/CrewControllerContext';

/** How long a started task may take to appear in the observer's runs and still be highlighted. */
export const NEW_TASK_HIGHLIGHT_WINDOW_MS = 30_000;

interface AwaitedTask {
  channelId: string;
  known: ReadonlySet<string>;
  until: number;
}

export interface TaskHighlight {
  /** The task row the timeline brings into view and washes once. */
  runId: string | null;
  /** "Show task in channel" (the agent pane) and an Agents row: highlight this task. */
  show(run: ObservedRun): void;
  /** The timeline finished the wash. */
  done(): void;
}

/**
 * The one highlight the timeline draws (ui-redesign-spec, "The timeline": a new task, Show task in
 * channel, an Agents row). The agent pane and the sidebar's Agents section only ask; the layout
 * owns the state because the timeline is the one that scrolls.
 *
 * A task the person just started has no id yet when the pane closes (the start request answers
 * without one the pane can see), so the layout notes which of the viewer's runs it already knew
 * when the controller reports `run-started`, and highlights the first new run in that channel
 * once the observer reports it.
 */
export function useTaskHighlight(): TaskHighlight {
  const { runs, channelId } = useCrew();
  const [runId, setRunId] = useState<string | null>(null);
  const awaited = useRef<AwaitedTask | null>(null);
  const latest = useRef({ runs, channelId });
  latest.current = { runs, channelId };

  useCrewSurfaceReset((reason) => {
    if (reason === 'run-started') {
      awaited.current = {
        channelId: latest.current.channelId,
        known: new Set(latest.current.runs.map((run) => run.run_id)),
        until: Date.now() + NEW_TASK_HIGHLIGHT_WINDOW_MS,
      };
    } else if (reason === 'channel-changed' || reason === 'connection-changed') {
      awaited.current = null;
    }
  });

  useEffect(() => {
    const waiting = awaited.current;
    if (!waiting) return;
    if (Date.now() > waiting.until) {
      awaited.current = null;
      return;
    }
    const started = runs.find(
      (run) => run.channel_id === waiting.channelId && !waiting.known.has(run.run_id)
    );
    if (!started) return;
    awaited.current = null;
    setRunId(started.run_id);
  }, [runs]);

  const show = useCallback((run: ObservedRun) => {
    awaited.current = null;
    setRunId(run.run_id);
  }, []);
  const done = useCallback(() => setRunId(null), []);

  return { runId, show, done };
}
