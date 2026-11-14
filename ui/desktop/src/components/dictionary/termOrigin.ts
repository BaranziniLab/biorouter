/**
 * Which chat a word picked from the context menu came from ("Context menu
 * door" in `docs/dictation/dictionary.md`, O-6, O-36).
 *
 * Main knows the window, not the pane, so it sends the menu's point and the
 * renderer looks there: from the element at that point it walks up to the
 * nearest container that names a chat.
 *
 * - A chat pane (`[data-chat-group-id]`, the box `ChatGroupsShell` draws for
 *   every pane of `/pair`, including its artifact panel) names its group, and
 *   the window's saved tab layout (`chatGroupsStorage.ts`) names the chat the
 *   group's active tab holds. That is a chat.
 * - A transcript that is not in such a pane (`[data-preview-transcript]`: a
 *   shared transcript, a schedule run's replay) names no chat this renderer can
 *   read, so it is **unresolved**, and the term dialog fails toward private.
 * - Anything else (Home, Settings, a field outside any transcript) is **none**.
 *
 * A pane whose tab has no chat yet (a new tab before its first message) holds
 * nothing but what the person typed, so it is none too.
 */
import { getWindowId, loadChatGroups } from '../chatGroups/chatGroupsStorage';
import type { ChatGroupsState } from '../chatGroups/chatGroupsTypes';

export type TermOrigin =
  | { kind: 'chat'; sessionId: string }
  | { kind: 'unresolved' }
  | { kind: 'none' };

export interface OriginDeps {
  /** The element at a point; `document.elementFromPoint` by default. */
  elementAt?: (x: number, y: number) => Element | null;
  /** This window's tab layout; the saved one by default. */
  groups?: () => ChatGroupsState | null;
}

function savedGroups(): ChatGroupsState | null {
  try {
    return loadChatGroups(getWindowId(), { keepSessionlessTab: () => true });
  } catch {
    return null;
  }
}

/** The chat the active tab of `groupId` holds, or `null`. */
function chatOfGroup(state: ChatGroupsState | null, groupId: string): string | null {
  const group = state?.groups[groupId];
  if (!group) return null;
  const tab = group.tabs.find((candidate) => candidate.tabId === group.activeTabId);
  return tab?.sessionId ? tab.sessionId : null;
}

/** Where the point `(x, y)` sits: a chat, a transcript naming no chat, or neither. */
export function originAt(x: number, y: number, deps: OriginDeps = {}): TermOrigin {
  const elementAt =
    deps.elementAt ??
    ((px: number, py: number) =>
      typeof document === 'undefined' ? null : document.elementFromPoint(px, py));
  const element = elementAt(x, y);
  if (!element) return { kind: 'none' };
  const inTranscript = element.closest('[data-preview-transcript]') !== null;
  const pane = element.closest<HTMLElement>('[data-chat-group-id]');
  if (pane) {
    const groupId = pane.getAttribute('data-chat-group-id') ?? '';
    const sessionId = chatOfGroup((deps.groups ?? savedGroups)(), groupId);
    if (sessionId) return { kind: 'chat', sessionId };
    // A pane whose chat this renderer cannot read: the transcript in it is not ours to name.
    return inTranscript ? { kind: 'unresolved' } : { kind: 'none' };
  }
  return inTranscript ? { kind: 'unresolved' } : { kind: 'none' };
}
