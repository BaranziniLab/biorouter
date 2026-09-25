/**
 * The Crew timeline (ui-redesign-spec, "The timeline"): the message list, day
 * and New dividers, older-history paging, and the viewer's task status rows.
 * `layout/CrewLayout.tsx` mounts `Timeline`; nothing here imports another area.
 */
export { Timeline, type TimelineProps, type TimelineView } from './Timeline';
export { timelineCopy } from './copy';
export {
  GROUP_GAP_MS,
  HISTORY_PAGE_SIZE,
  groupMessages,
  isTraceMessage,
  newLineBeforeId,
  reachesChannelStart,
  taskTitle,
  type TimelineDay,
  type TimelineGroup,
  type TimelineItem,
  type TimelineTask,
} from './groupMessages';
export { CrewMarkdown, MessageBody, safeExternalHref } from './MessageBody';
export type { AttachmentSlotState, OwnAgentChat, RenderAttachments } from './TimelineContext';
export { AUTO_READ_DWELL_MS, AUTO_READ_MIN_INTERVAL_MS } from './useAutoMarkRead';
