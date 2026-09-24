import { createContext, useContext, type ReactNode } from 'react';
import type { CrewMessage } from '../crewApi';
import type { PeopleDirectory } from '../identity';

/** What every row of one timeline shares. Provided by `Timeline`; never read outside it. */
export interface TimelineContextValue {
  /** The workspace's people, for names and avatars. */
  dir: PeopleDirectory;
  /** The viewer's principal ID: their agent is "Your agent". A key, never rendered. */
  viewerId: string | null;
  /** Presentation only (the last verified view during re-verification): nothing acts. */
  readOnly: boolean;
  /** Attachments and server paths under a body; the files area renders them. */
  renderAttachments?: (message: CrewMessage) => ReactNode;
  /**
   * The row whose actions are in the tab order. Rows are reached with the arrow
   * keys (roving focus), so a long log is not hundreds of Tab stops.
   */
  activeRow: string | null;
  setActiveRow(key: string): void;
  /** Message IDs that arrived live while the reader followed the bottom: they rise in once. */
  arriving: ReadonlySet<string>;
  /** Task rows register their element so the timeline can scroll one into view. */
  registerTaskRow(runId: string, element: HTMLElement | null): void;
  /** The run whose row carries the highlight wash right now. */
  highlightedRunId: string | null;
  onHighlightEnd(runId: string): void;
}

const TimelineContext = createContext<TimelineContextValue | null>(null);

export const TimelineContextProvider = TimelineContext.Provider;

export function useTimeline(): TimelineContextValue {
  const value = useContext(TimelineContext);
  if (!value) throw new Error('Timeline rows must render inside a Timeline.');
  return value;
}
