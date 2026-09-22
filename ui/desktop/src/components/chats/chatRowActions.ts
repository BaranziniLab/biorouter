import type { ComponentType } from 'react';

import { Copy, MessageSquare, NewWindow } from '../icons/app-icons';
import { toastError, toastSuccess } from '../../toasts';

/**
 * The three actions every chat row offers, in one place.
 *
 * ⚠ **This is a list of ACTIONS, not a menu component.** History's `⋯` overflow
 * is a `DropdownMenu`, the right-click menus are `ContextMenu`s, and the tab
 * strip's is a third instance of the latter — four entry points onto the same
 * three operations ([#114](https://github.com/BaranziniLab/biorouter/issues/114)).
 * What must not drift between them is the *order*, the *labels* and *what each
 * one does*; the menu chrome is each menu's own. So the descriptors live here
 * and each surface maps them into its own `Item`, rather than a shared
 * component trying to be both menus at once.
 *
 * The order is fixed by the issue and is not arbitrary: the two openers first,
 * because they are what a row is usually right-clicked for, then the copy.
 */
export type ChatRowAction = {
  /** Stable key for tests and React keys — never shown. */
  key: 'open-tab' | 'open-window' | 'copy-id';
  label: string;
  icon: ComponentType<{ className?: string }>;
  run: () => void;
};

export type ChatRowActionTarget = {
  /** The conversation's stable id, e.g. `20260823_2`. */
  sessionId: string;
  /**
   * Where the conversation works, when the surface knows it. It is only the new
   * window's *starting* directory — the window resumes `sessionId`, whose own
   * record carries the real one — so a surface that does not have it (the tab
   * strip, for a tab with no `cwd`) passes nothing rather than guessing.
   */
  workingDir?: string;
  /**
   * Open the conversation in a tab. **Supplied by the surface**, because each
   * one already has a working path for it and the issue is explicit that this
   * must reuse them rather than add a second lifecycle: History passes the same
   * `onSelectSession` its row click calls, Recents the same `onOpen`, the tab
   * strip its own `onSelect`.
   */
  openInNewTab: () => void;
};

/**
 * Put a conversation id on the clipboard, undecorated.
 *
 * The raw `Session.id` and nothing else — no prefix, no URL, no display name.
 * The point of the affordance is that the string can be pasted straight into a
 * chat, where it is what Chat Recall's exact-ID load and every `workspace_*`
 * id argument already accept. Anything wrapped around it would have to be
 * stripped back off by hand.
 *
 * `navigator.clipboard` can be absent (an insecure context) as well as
 * rejecting, so the guard is a check *and* a catch: a menu item that silently
 * did nothing is the one outcome worth ruling out.
 */
export async function copyConversationId(sessionId: string): Promise<boolean> {
  try {
    if (window.electron?.copyConversationId) {
      await window.electron.copyConversationId(sessionId);
    } else {
      if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
      await navigator.clipboard.writeText(sessionId);
    }
    toastSuccess({
      title: 'Conversation ID copied',
      msg: sessionId,
      toastOptions: { autoClose: 2000 },
    });
    return true;
  } catch (error) {
    console.error('Failed to copy conversation ID:', error);
    toastError({
      title: 'Could not copy the conversation ID',
      msg: `Copy it by hand: ${sessionId}`,
    });
    return false;
  }
}

export function chatRowActions(target: ChatRowActionTarget): ChatRowAction[] {
  return [
    {
      key: 'open-tab',
      label: 'Open in new tab',
      icon: MessageSquare,
      run: target.openInNewTab,
    },
    {
      key: 'open-window',
      label: 'Open in new window',
      icon: NewWindow,
      run: () => openChatInNewWindow(target.sessionId, target.workingDir),
    },
    {
      key: 'copy-id',
      label: 'Copy conversation ID',
      icon: Copy,
      run: () => void copyConversationId(target.sessionId),
    },
  ];
}

/**
 * The ONE new-window path, so the three menus and History's overflow cannot
 * drift into four slightly different `createChatWindow` calls.
 *
 * The argument shape is History's, which was the only surface that had this
 * action before #114: no query, the session's directory as the starting hint,
 * no version, `resumeSessionId` so the window loads that conversation, and
 * `'pair'` so it opens as a tabbed chat surface rather than the launcher.
 */
export function openChatInNewWindow(sessionId: string, workingDir?: string): void {
  window.electron?.createChatWindow?.(undefined, workingDir, undefined, sessionId, 'pair');
}
