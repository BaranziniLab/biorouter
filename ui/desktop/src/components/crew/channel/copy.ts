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
  /** The member stack's name where the broker says who is online (M18). */
  membersOnline: (count: number, online: number) =>
    `${plural(count, 'member', 'members')}, ${online} online`,
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
  /**
   * A failed Connect, in words (NEW-1). The daemon's own text for an SSH failure is a transport
   * record — "Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: …" — and never reaches the
   * bar; the failure's kind picks one of these instead. The sign-in and trust screens keep
   * OpenSSH's words under their own "Copy details".
   */
  cantConnect: (host: string) => (host ? `Can’t connect to ${host}.` : 'Crew can’t connect.'),
  signInNeeded: (host: string) =>
    host ? `${host} asked you to sign in.` : 'The server asked you to sign in.',
  cantVerify: (host: string) =>
    host ? `Crew couldn’t verify ${host}.` : 'Crew couldn’t verify the server.',
  notRunning: (host: string) =>
    host ? `Crew isn’t running for you on ${host}.` : 'Crew isn’t running for you on the server.',
  /**
   * The server refused this computer's SSH key and offered nothing to type (F5, W2-DMN-5): not
   * "asked you to sign in", which opened a password window that could never help. `user` is the
   * login's account, or null.
   */
  /**
   * The app lost its background service (it restarted), so no Crew request can reach it until
   * Biorouter reconnects, which it does on its own (RES2-N7): said in place of "Live updates
   * stopped" and its Retry, which led only to "Crew couldn't load your saved workspaces".
   */
  /**
   * The workspace server has stopped saving changes (its `hello` says so, T3-BE-13, RES2-N2):
   * reading works, and every change is refused until its host frees space and restarts Crew. Said
   * before anyone tries to write, rather than "Connected" until someone did. `code` is the server's
   * reason: `storage_full` (its disk or quota) or `storage_failed` (another storage error).
   */
  serverStorage: 'The workspace server has stopped saving changes. Reading still works.',
  serverStorageHost: (code: string) =>
    code === 'storage_full'
      ? 'Free space on the server, then restart Crew there.'
      : 'Check the server’s storage, then restart Crew there.',
  serverStorageMember: (code: string, hostName: string | null) =>
    code === 'storage_full'
      ? `Ask ${hostName ?? 'your host'} to free space on the server and restart Crew.`
      : `Ask ${hostName ?? 'your host'} to check the server’s storage and restart Crew.`,
  daemonAway:
    'Biorouter lost its connection to its background service, so Crew can’t reach your workspaces until Biorouter reconnects.',
  daemonReconnect: 'Reconnect',
  daemonReconnecting: 'Reconnecting…',
  keyRefused: (host: string, user: string | null) =>
    `${host || 'The server'} refused this computer’s SSH key${user ? ` for ${user}` : ''}. Check Your server login in Connection settings.`,
  /**
   * The workspace server is not running: stopped, killed, or the server restarted (R-7). Its host
   * starts it; a member asks the host. `hostName` names the host, or is null.
   */
  brokerStoppedHost: (host: string) =>
    `Crew isn’t running on ${host || 'the server'}. Start it on the server, then connect.`,
  brokerStoppedMember: (hostName: string | null) =>
    `The workspace server isn’t running. Once ${hostName ?? 'your host'} starts Crew, this computer connects by itself within a few minutes, or you can connect now.`,
  /** The same when this computer cannot tell whether its person hosts the workspace. */
  brokerStopped: (host: string) =>
    `Crew isn’t running on ${host || 'the server'}. Its host starts it again on the server.`,
  /**
   * An action whose link dropped mid-request (the transport's own record reached the bar, R-4):
   * whether it reached the workspace is not known.
   */
  linkLost: (host: string) =>
    `The connection to ${host || 'the server'} dropped, so Crew can’t tell whether that went through. Check before you try again.`,
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
