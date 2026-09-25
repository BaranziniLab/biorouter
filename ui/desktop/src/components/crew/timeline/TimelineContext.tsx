import { createContext, useContext, type ReactNode } from 'react';
import type { CrewMessage } from '../crewApi';
import type { PeopleDirectory } from '../identity';

/** What a message's attachments slot is told about the row it renders in. */
export interface AttachmentSlotState {
  /**
   * The row is the log's active row (the one arrowed or clicked to). Only then are the slot's
   * controls Tab stops, exactly like the row's own actions: a file card's Save and ⋯ were always
   * in the Tab order, so a channel with files was 8–10 stops from the log to the composer (Q3-05).
   */
  active: boolean;
}

/** Renders a message's attachments and server paths under its body. */
export type RenderAttachments = (message: CrewMessage, slot: AttachmentSlotState) => ReactNode;

/**
 * One of the VIEWER's own chats that holds Crew access, by the run it posts as (Q3-22). Built by
 * the layout from this device's own grants, so it can only ever name the viewer's chats.
 */
export interface OwnAgentChat {
  /** The chat's title, as the daemon lists it. Display text. */
  title: string;
  /** The chat to open. A key, never rendered. */
  sessionId: string;
}

/** What every row of one timeline shares. Provided by `Timeline`; never read outside it. */
export interface TimelineContextValue {
  /** The workspace's people, for names and avatars. */
  dir: PeopleDirectory;
  /** The viewer's principal ID: their agent is "Your agent". A key, never rendered. */
  viewerId: string | null;
  /** Presentation only (the last verified view during re-verification): nothing acts. */
  readOnly: boolean;
  /** Attachments and server paths under a body; the files area renders them. */
  renderAttachments?: RenderAttachments;
  /** The viewer's own chats with access, by run ID: their posts read "Your agent · {title}". */
  ownAgentChats: ReadonlyMap<string, OwnAgentChat>;
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
