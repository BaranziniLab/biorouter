/**
 * The channel header, channel menu and connection bar strings (ui-redesign-spec, copy deck
 * "Channel header, menu and pane" and "Connection problems and sign in").
 *
 * Tests import these instead of retyping them. A string marked pinned is asserted by a regression
 * test or cited by acceptance evidence; change it only together with that test.
 */

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

export const channelCopy = {
  /** Accessible name of the `# name ▾` trigger. The regression helpers find it by this name. */
  menuName: (slug: string) => `${slug} channel menu`,
  restricted: 'Restricted',
  publicSafe: 'Public-safe',
  archived: 'Archived',
  restrictedHint: 'Public models can’t read it.',
  publicSafeHint: 'Public models may read it when the workspace allows.',
  /** The agent-access chip's visible words. Chats and tasks together are "agents". */
  accessChip: (chats: number, tasks: number) =>
    chats > 0 && tasks > 0
      ? plural(chats + tasks, 'agent', 'agents')
      : tasks > 0
        ? plural(tasks, 'task', 'tasks')
        : plural(chats, 'chat', 'chats'),
  /** The chip's accessible name: its visible words, then what they mean. */
  accessChipName: (visible: string) => `${visible} can post here`,
  /** The member stack's accessible name (and the Members tab count). */
  members: (count: number) => plural(count, 'member', 'members'),
  /** The details toggle. */
  details: 'Channel details',
  menu: {
    details: 'Channel details',
    members: 'Members',
    files: 'Files',
    access: 'Chats and agents with access',
    addPeople: 'Add people…',
    markRead: 'Mark as read',
    /** Pinned. */
    refresh: 'Refresh channel',
    copyName: 'Copy channel name',
    copyId: 'Copy channel ID',
    rename: 'Rename…',
    transfer: 'Transfer ownership…',
    archive: 'Archive channel…',
  },
} as const;

export const connectionBarCopy = {
  /** The connection bar's landmark name. */
  label: 'Connection',
  retry: 'Retry',
  /** Pinned accessible name of the observation error's Retry. */
  retryName: 'Retry Crew updates',
  dismiss: 'Dismiss',
  unreachable: (host: string) => `Can’t reach ${host}.`,
  tryAgain: 'Try again',
  connectionSettings: 'Connection settings…',
  vaultLocked: 'Your Crew vault is locked.',
  unlock: 'Unlock',
  unlockFailed: 'Crew couldn’t unlock the vault.',
  reconnecting: (workspace: string) => `Reconnecting to ${workspace}…`,
  newDevice: (date: string) => `A new device was added to your account on ${date}.`,
  newDeviceUndated: 'A new device was added to your account.',
  review: 'Review',
  reviewName: 'Review devices',
} as const;
