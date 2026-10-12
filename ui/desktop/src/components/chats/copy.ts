/**
 * The words on a chat row's menu and its inline editor, in one place so the
 * sidebar, History and the tab strip say the same thing and tests import them.
 */
export const chatRowCopy = {
  menu: {
    rename: 'Rename',
    renameShortcut: 'F2',
    openInNewTab: 'Open in new tab',
    openInNewWindow: 'Open in new window',
    diverge: 'Diverge',
    export: 'Export…',
    copyId: 'Copy chat ID',
    /** The tab strip's own pair: a tab closes, the chat it holds is kept. */
    closeTab: 'Close tab',
    closeOtherTabs: 'Close other tabs',
    delete: 'Delete chat…',
  },
  copyId: {
    copied: 'Chat ID copied',
    failed: "Couldn't copy the chat ID",
    failedDetail: (sessionId: string) => `Copy it by hand: ${sessionId}`,
  },
  rename: {
    inputLabel: (title: string) => `Rename chat ${title}`,
  },
  /** Export… (`utils/exportConversation.ts`), from the sidebar and History alike. */
  export: {
    /** The Save window's title. */
    dialogTitle: 'Export chat',
    fileType: 'JSON',
    /** The file name when the chat has none. */
    untitled: 'Untitled chat',
    /** Shown once the file is written, naming the file and its folder. */
    saved: 'Chat exported',
    savedTo: (file: string, folder: string) => `Saved "${file}" to ${folder}.`,
    failed: "Couldn't export this chat",
    readFailed: 'The chat could not be read for export. Nothing was downloaded.',
    writeFailed: (folder: string) =>
      `The file could not be written to ${folder}. Nothing was saved.`,
  },
  deleteDialog: {
    title: 'Delete chat?',
    message: (title: string) => `Delete "${title}"? This can't be undone.`,
    confirm: 'Delete',
    cancel: 'Cancel',
    /** The toast after a delete lands: the row leaving the list says the rest. */
    deleted: 'Chat deleted',
    failed: "Couldn't delete chat",
    /** Only when a failure carries no text of its own; a daemon sentence is shown as is. */
    failedFallback: 'Try again.',
  },
} as const;
