import { isMacPlatform } from '../ui/copy';

/**
 * The words find-in-chat says (spec 3.16), in one place so the bar and its
 * tests read the same text. The noun is "chat" and the ellipsis is "…".
 */
export const findInChatCopy = {
  placeholder: 'Find in chat…',
  inputLabel: 'Find in chat',
  /** The InfoTip beside the field: what ⌘F does to a long chat. */
  helpLabel: 'find in chat',
  help: (mac: boolean = isMacPlatform()) =>
    `${mac ? '⌘F' : 'Ctrl+F'} also loads every message of a long chat, so nothing is missed.`,
  matchCase: 'Match case',
  previous: 'Previous match',
  next: 'Next match',
  close: 'Close find',
  /** Tooltips carry the shortcut after the name. */
  previousTip: 'Previous match · ↑',
  nextTip: 'Next match · Enter',
  closeTip: 'Close · Esc',
  /** "3 of 12": the current match and the count. */
  position: (current: number, count: number) => `${current} of ${count}`,
  /** Below the surface's floor nothing is searched, and the bar says so. */
  belowMinimum: (minimum: number) => `Type at least ${minimum} characters to search.`,
} as const;
