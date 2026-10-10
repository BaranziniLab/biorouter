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
  deleteDialog: {
    title: 'Delete chat?',
    message: (title: string) => `Delete "${title}"? This can't be undone.`,
    confirm: 'Delete',
    cancel: 'Cancel',
    failed: "Couldn't delete chat",
  },
} as const;
