import { toastError, toastSuccess } from '../../toasts';
import { chatRowCopy } from './copy';

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
 * because they are what a row is usually right-clicked for, then the copy. A tab
 * has no "open in new tab" (it is one), so its list starts at the window.
 *
 * The descriptors carry no icon: chat-row menus are text only (spec 2.6, Crew's
 * `ChannelRow` menus), so an icon here would only name a glyph nothing draws.
 */
export type ChatRowAction = {
  /** Stable key for tests and React keys — never shown. */
  key: 'open-tab' | 'open-window' | 'copy-id';
  label: string;
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
   * `onSelectSession` its row click calls, Recents the same `onOpen`.
   *
   * `null` is the tab strip's answer, and it is stated rather than left out so
   * a surface cannot forget the opener by accident: a tab IS the open tab, so
   * there is nothing to open, and the menu leaves the item out. It used to map
   * to the strip's own `onSelect`, which only selected the tab the person was
   * already pointing at and read as an action that did nothing (vision V2-R6-3).
   */
  openInNewTab: (() => void) | null;
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
      title: chatRowCopy.copyId.copied,
      msg: sessionId,
      toastOptions: { autoClose: 2000 },
    });
    return true;
  } catch (error) {
    console.error('Failed to copy conversation ID:', error);
    toastError({
      title: chatRowCopy.copyId.failed,
      msg: chatRowCopy.copyId.failedDetail(sessionId),
    });
    return false;
  }
}

export function chatRowActions(target: ChatRowActionTarget): ChatRowAction[] {
  const { openInNewTab } = target;
  return [
    ...(openInNewTab
      ? [{ key: 'open-tab' as const, label: chatRowCopy.menu.openInNewTab, run: openInNewTab }]
      : []),
    {
      key: 'open-window',
      label: chatRowCopy.menu.openInNewWindow,
      run: () => openChatInNewWindow(target.sessionId, target.workingDir),
    },
    {
      key: 'copy-id',
      label: chatRowCopy.menu.copyId,
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
