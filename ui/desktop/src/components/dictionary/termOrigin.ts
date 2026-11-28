/**
 * Which chat a word picked from the context menu came from ("Context menu
 * door" in `docs/dictation/dictionary.md`, O-6, O-36), and which dictation a
 * word taken from the **History** tab came from (`history-design.md` 10.6).
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
 *
 * A History row (`[data-dictation-history-entry]`) is checked first: the
 * dictation is the origin, read from the row's own attributes
 * ({@link historyEntryOrigin}), and the term dialog starts **Only in private
 * chats** from the dictation's tier, not the chat's.
 */
import type { DictationHistorySurface, DictionaryTier } from '../../api';
import { getWindowId, loadChatGroups } from '../chatGroups/chatGroupsStorage';
import type { ChatGroupsState } from '../chatGroups/chatGroupsTypes';

/** A word taken from a dictation in the history: the dictation's tier decides the private box. */
export interface HistoryOrigin {
  tier: DictionaryTier;
}

/**
 * Where an add started. `history`, when set, says it started in a History row, whose tier
 * overrides whatever the chat's own tier would start the box at.
 */
export type TermOrigin =
  | { kind: 'chat'; sessionId: string; history?: HistoryOrigin }
  | { kind: 'unresolved'; history?: HistoryOrigin }
  | { kind: 'none'; history?: HistoryOrigin };

/** The attributes a History row carries for the doors that start an add inside it. */
export const HISTORY_ENTRY_ATTRIBUTE = 'data-dictation-history-entry';
export const HISTORY_TIER_ATTRIBUTE = 'data-history-tier';
export const HISTORY_SESSION_ATTRIBUTE = 'data-history-session';
export const HISTORY_SURFACE_ATTRIBUTE = 'data-history-surface';

/** The surfaces a dictation may have been recorded into a chat from: a composer, or a terminal session. */
const CHAT_SURFACES: ReadonlySet<DictationHistorySurface> = new Set(['composer', 'terminal']);

/** What a History row says of its dictation; `sessionId` only a chat the daemon verified for this caller. */
export interface HistoryEntryFacts {
  tier: DictionaryTier;
  sessionId?: string | null;
  surface: DictationHistorySurface;
}

/**
 * The origin an add from a dictation sends (`history-design.md` 10.6):
 *
 * | Dictation | Box starts | Origin |
 * |---|---|---|
 * | Private, with a verified chat | ticked | that chat (`origin_session_id`) |
 * | Private, into a composer or a terminal, no verified chat | ticked | unresolved (`origin_unresolved`) |
 * | Private, outside any chat (Crew, clipboard, another app, app preview) | ticked | none |
 * | Public | unticked | the verified chat, else none |
 *
 * The daemon names a dictation's chat only while the chat holding that id is the one it
 * recorded and this caller may list it, so a dictation into a chat that is gone, reissued or
 * unlisted reads exactly as one into a new chat that was never saved. Both fail toward the
 * narrower stamp, unresolved (local models only, the `[]` of P-16).
 */
export function historyEntryOrigin(facts: HistoryEntryFacts): TermOrigin {
  const history: HistoryOrigin = { tier: facts.tier };
  const sessionId = facts.sessionId?.trim() ?? '';
  if (sessionId) return { kind: 'chat', sessionId, history };
  if (facts.tier === 'private' && CHAT_SURFACES.has(facts.surface)) {
    return { kind: 'unresolved', history };
  }
  return { kind: 'none', history };
}

/** The tier the add's private box starts at, or `null` for an origin that is no dictation. */
export function historyStartsPrivate(origin: TermOrigin | undefined): boolean | null {
  return origin?.history ? origin.history.tier === 'private' : null;
}

const TIERS: ReadonlySet<string> = new Set(['public', 'private']);
const SURFACES: ReadonlySet<string> = new Set([
  'composer',
  'crew',
  'clipboard',
  'other_app',
  'app_preview',
  'terminal',
]);

/**
 * The dictation a History row names, read from its attributes; `null` for any other element.
 * A tier that is not one of the two words reads as private (fail toward private).
 */
export function historyFactsOf(row: Element): HistoryEntryFacts | null {
  if (!row.hasAttribute(HISTORY_ENTRY_ATTRIBUTE)) return null;
  const tier = row.getAttribute(HISTORY_TIER_ATTRIBUTE) ?? '';
  const surface = row.getAttribute(HISTORY_SURFACE_ATTRIBUTE) ?? '';
  return {
    tier: TIERS.has(tier) ? (tier as DictionaryTier) : 'private',
    sessionId: row.getAttribute(HISTORY_SESSION_ATTRIBUTE),
    surface: SURFACES.has(surface) ? (surface as DictationHistorySurface) : 'composer',
  };
}

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
  const historyRow = element.closest(`[${HISTORY_ENTRY_ATTRIBUTE}]`);
  const facts = historyRow ? historyFactsOf(historyRow) : null;
  if (facts) return historyEntryOrigin(facts);
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
