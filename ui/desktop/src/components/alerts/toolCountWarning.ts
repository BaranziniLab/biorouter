import { openCurrentChatExtensions } from '../../utils/sessionToolEvents';

/** Above this many tools a model chooses worse and spends more context on their definitions. */
export const TOOLS_MAX_SUGGESTED = 60;

/**
 * The context card's tool-count line (Phase C F-11, 2026-11-14). It used to be a composer alert
 * whose "Review extensions" opened this chat's extensions; the October composer rework removed
 * the alerts, and the line with them, so nothing in the app opened that list any more. It now
 * sits in the context card (`ContextWindowIndicator`): the tools' definitions are part of what
 * fills the window the card reports, and the card already follows the app's one pointer rule
 * (hover peeks, click pins). One line; why it matters is its InfoTip (no walls of text).
 */
export const TOOL_COUNT_COPY = {
  line: (count: number) =>
    `This chat can call ${count} tools; ${TOOLS_MAX_SUGGESTED} or fewer works best.`,
  helpLabel: 'About the tool count',
  help: 'Too many tools can slow the model and make it pick the wrong one. Switch off what this chat does not need, built-in capabilities included.',
  review: 'Review extensions',
} as const;

/** What the context card says about the chat's tools, or `null` when there is nothing to say. */
export interface ToolCountWarning {
  count: number;
  /** Opens the composer's `+` popover on this chat's Extensions list. */
  review: () => void;
}

export function toolCountWarning(
  toolCount: number | null | undefined,
  sessionId: string | null | undefined
): ToolCountWarning | null {
  if (toolCount == null || toolCount <= TOOLS_MAX_SUGGESTED || !sessionId) return null;
  return { count: toolCount, review: () => openCurrentChatExtensions(sessionId) };
}
