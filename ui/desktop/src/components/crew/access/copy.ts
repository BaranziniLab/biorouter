/**
 * Chat access and revoke strings (ui-redesign-spec, copy deck "Chat access and revoke", the
 * Agents rows of "Sidebar and menus", and "Error strings").
 *
 * Tests import these rather than retyping them. `Review access and posting permission` and
 * `Allow this conversation to read and post here` are pinned: a regression test and the acceptance
 * evidence find the controls by them, so change them only together with those tests. The second is
 * the Allow button's name only while the chat's title is unknown; a chat this window knows by name
 * is named on the button itself (`allowChat`).
 *
 * A `chat` argument is a conversation's title, already fit to display, or `null` when the daemon
 * does not know it; every sentence that names a chat has a form for that case.
 */

const quoted = (chat: string) => `“${chat}”`;
/** The chat as a sentence's subject: its quoted title, or "This chat" when it has none. */
const chatSubject = (chat: string | null) => (chat ? quoted(chat) : 'This chat');

export const accessCopy = {
  // ── The chat-connect note (above the Crew composer, only with ?sessionId=) ──────────────────
  // Every sentence names the chat when this window knows its title: inside Crew, "This chat" does
  // not say which one (live QA round 3, Q3-30). A `null` chat keeps the "This chat" form.
  noteChecking: 'Checking this chat’s access…',
  noteNone: (chat: string | null, channel: string) =>
    chat ? `Connect ${quoted(chat)} to ${channel}?` : `Connect this chat to ${channel}?`,
  noteReview: 'Review access',
  /** Pinned: the Review access button's accessible name. */
  noteReviewName: 'Review access and posting permission',
  noteActive: (chat: string | null, channel: string) =>
    `${chatSubject(chat)} can read and post in ${channel}.`,
  noteActiveElsewhere: (chat: string | null, other: string) =>
    `${chatSubject(chat)} already uses ${other}.`,
  noteActiveOtherWorkspace: (chat: string | null, workspace: string) =>
    `${chatSubject(chat)} already uses a channel in ${workspace}.`,
  noteManage: 'Manage access',
  noteRevoked: (chat: string | null) =>
    chat ? `Crew access for ${quoted(chat)} was revoked.` : 'This chat’s Crew access was revoked.',
  noteExpired: (chat: string | null) =>
    chat ? `Crew access for ${quoted(chat)} expired.` : 'This chat’s access expired.',
  noteGrantAgain: 'Grant again',
  /** A task's grant ends when the task does: nothing went wrong, and there is nothing to grant. */
  noteTaskFinished: 'This task is finished. Its access ended when it finished.',

  // ── The Chat access pane ───────────────────────────────────────────────────────────────────
  paneTitle: 'Chat access',
  willBeAble: (chat: string | null) =>
    chat ? `${quoted(chat)} will be able to` : 'This chat will be able to',
  canNow: (chat: string | null) => `${chatSubject(chat)} can`,
  chatName: (chat: string | null) => chatSubject(chat),
  read: (channel: string) => `Read ${channel}`,
  reads: (channel: string) => `Reads ${channel}`,
  /** Followed by the person, rendered with `PersonName context="authority"`. */
  postAs: (channel: string) => `Post in ${channel} as`,
  postsAs: (channel: string) => `Posts in ${channel} as`,
  expiry: 'Access ends when you revoke it, or after an hour.',
  alsoRead: 'Also read',
  /** Beside Advanced while it is closed: what the chat reads, not what it doesn't (Q3-30). */
  alsoReadSummary: (count: number, channel: string) =>
    count === 0
      ? `Reads only ${channel}`
      : `Also reads ${count} ${count === 1 ? 'channel' : 'channels'}`,
  /** Pinned. */
  allow: 'Allow this conversation to read and post here',
  /** The Allow button when the chat's title is known: the person sees which chat they let in. */
  allowChat: (chat: string, channel: string) =>
    `Allow ${quoted(chat)} to read and post in ${channel}`,
  connected: 'Connected.',
  backToChat: 'Back to chat',
  openChat: 'Open chat',
  revokeButton: 'Revoke access',
  confirm: (chat: string | null, channel: string) =>
    chat
      ? `Stop ${quoted(chat)} reading and posting in ${channel}?`
      : `Stop this chat reading and posting in ${channel}?`,
  /**
   * Under the revoke question: the whole chat stops, not only its posts in Crew (live QA round 4,
   * Q4-15). The question above already names the chat.
   */
  confirmStops: 'This chat will stop until you grant access again.',
  confirmRevoke: 'Revoke',
  confirmKeep: 'Keep access',
  // One way forward, said once: "…, or start a new chat" read as a second, unrelated instruction
  // (live QA round 2, Q2-72).
  revoked: (chat: string | null) =>
    chat
      ? `Access revoked. ${quoted(chat)} can’t use Crew until you grant access again.`
      : 'Access revoked. This chat can’t use Crew until you grant access again.',
  unconfirmed: 'Stopped on this device. Reconnect to confirm with the workspace.',
  notRevoked: 'Not revoked. This chat can still read and post.',
  retry: 'Retry',
  done: 'Done',
  // "for “Plot review”", never "“Plot review”’s": a possessive after a closing quote reads as a
  // typo (live QA round 1, T-55).
  paneRevoked: (chat: string | null) =>
    chat ? `Crew access for ${quoted(chat)} was revoked.` : 'This chat’s Crew access was revoked.',
  paneExpired: (chat: string | null) =>
    chat ? `Crew access for ${quoted(chat)} expired.` : 'This chat’s access expired.',
  paneTaskFinished: 'This task is finished. Its access ended when it finished.',
  /** A revoked chat that did not arrive here with /crew can only be connected from inside it. */
  reconnectHow: 'To connect it again, type /crew in that chat.',
  /** The pane with no chat to show: a chat is connected from inside it. */
  paneNoChat: 'To connect a chat, send it a message, then type /crew in it.',

  // ── Access rows (Access tab, Agent access tab, Agents section) ─────────────────────────────
  /** One name wherever this list appears: the channel's Access tab and Workspace settings. */
  tabTitle: 'Agent access',
  status: {
    /** An active grant whose end time is not known yet (the moment after Allow). */
    active: 'Active',
    /** One wording for the same state everywhere: never "Active" here and "Expires …" there. */
    expires: (time: string) => `Active · ends ${time}`,
    expired: 'Expired',
    revoked: 'Revoked',
    unconfirmed: 'Stopped on this device',
    /** A task's grant after the task: it ended with the task, nobody revoked it (Q2-09). */
    ended: 'Ended',
  },
  /** Its rows read Ended, Revoked or Expired: one name for all of them (Q3-30). */
  showOld: (count: number) => `Show past access (${count})`,
  oldListName: 'Past access',
  listLoading: 'Loading agent access…',
  // The list holds this computer's own chats and tasks, never anyone else's agents: from a
  // member's seat "No chats or agents can post" was false whenever another person's agent posts
  // there (live QA round 3, Q3-29). It says whose, and how to connect one.
  empty: (channel: string) => `None of your chats can post in ${channel} yet.`,
  emptyWorkspace: (workspace: string) => `None of your chats can post in ${workspace} yet.`,
  /** Follows `empty`: a chat is connected from inside it. `/crew` is drawn as code. */
  emptyHow: 'To connect one, open that chat and type /crew.',
  untitled: 'Untitled chat',
  yourTask: 'Your task',
  /**
   * What tells two task rows apart after "Your task": when it started and its first words,
   * `1:16 PM · Please work out…` (Q2-74). Either part may be missing.
   */
  taskDetail: (time: string | null, words: string | null) =>
    [time, words].filter((part): part is string => Boolean(part)).join(' · '),
  unknownChannel: 'a channel you can’t see',
  moreSources: (count: number) => `+${count}`,
  moreSourcesName: (count: number) =>
    `and ${count} more ${count === 1 ? 'channel' : 'channels'} it can read`,
  open: 'Open',
  openName: (title: string) => `Open ${title}`,
  /** Pinned elsewhere (the timeline's task row): a task's Open names the conversation it opens. */
  openTaskName: 'Open agent conversation',
  revokeRow: 'Revoke',
  revokeRowName: (title: string) => `Revoke access for ${title}`,
  retryRowName: (title: string) => `Retry revoking ${title}`,
  stopRow: 'Stop',
  stopRowName: 'Stop task',
  stopConfirm: 'Stop your agent?',
  stopConfirmBody: 'It stops working on this task. Anything it already did stays done.',
  stopKeep: 'Keep running',
  stopConfirmAction: 'Stop task',
  listFailed: 'Couldn’t load which chats have access.',
  listRetryName: 'Retry loading chat access',

  // ── The Agents sidebar section ─────────────────────────────────────────────────────────────
  agents: 'Agents',
  agentsOptions: 'Agents options',
  agentsShowAll: 'Show revoked and finished',
  /** Between a row's first and second part: `#methods · Working…`, `Plot review · #methods`. */
  agentsSeparator: ' · ',
  needsYou: 'Needs you',

  // ── The header chip ────────────────────────────────────────────────────────────────────────
  chipChats: (count: number) => `${count} ${count === 1 ? 'chat' : 'chats'}`,
  chipTasks: (count: number) => `${count} ${count === 1 ? 'task' : 'tasks'}`,
  chipAgents: (count: number) => `${count} ${count === 1 ? 'agent' : 'agents'}`,
  chipName: (count: number) =>
    `${count} ${count === 1 ? 'chat or agent' : 'chats or agents'} can post here`,

  // ── The ordinary chat (outside Crew) ───────────────────────────────────────────────────────
  chatChip: (destination: string) => `Crew · ${destination}`,
  /** Contains the visible text, so a voice command naming what is shown still reaches it. */
  chatChipName: (destination: string) => `Crew · ${destination}, manage access`,
  chatChipTip: (destination: string) => `This chat can read and post in ${destination}.`,
  // "Team content" was unexplained (Q2-72): say what it is — messages from the channel.
  chatRevoked: (destination: string) =>
    `Crew access to ${destination} was removed, so this chat can’t continue. It holds messages from the channel. Grant access again to continue, or start a new chat.`,
  chatExpired: (destination: string) =>
    `Crew access to ${destination} expired. This chat has team content, so it can’t continue.`,
  /** A task's chat after the task: its grant ended with it (Q2-09). Neutral, not a warning. */
  chatTaskFinished: (destination: string) =>
    `This task is finished. Its access to ${destination} ended when it finished.`,
  /** An active grant whose Crew connection is down (Q2-08). */
  chatOffline: (destination: string) =>
    `Crew is offline. This chat can’t read or post in ${destination} until you connect.`,
  /**
   * The same, when the connection went down for a network reason: the daemon dials it again by
   * itself once the network is back (Q4-01), so the person need not act (live QA round 4, Q4-06).
   */
  chatOfflineNetwork: 'Crew is offline. It will reconnect by itself when the network is back.',
  chatConnectInCrew: 'Connect in Crew',
  /** Beside {@link accessCopy.chatOfflineNetwork}: the same one-hop connect, not a requirement. */
  chatConnectNow: 'Connect now',
  chatNewChat: 'Start a new chat',
  chatGrantAgain: 'Grant access again',
  chatBlockedReason: 'This chat can’t continue without Crew access.',
  /** What Enter says in a held chat instead of doing nothing. */
  chatBlockedSendTitle: 'Can’t send',
  chatBlockedSendRevoked: (destination: string) =>
    `Crew access to ${destination} was removed. Grant it again or start a new chat.`,
  chatBlockedSendExpired: (destination: string) =>
    `Crew access to ${destination} expired. Grant it again or start a new chat.`,
  chatBlockedSendTaskFinished: (destination: string) =>
    `This task is finished, and its access to ${destination} ended with it. Start a new chat to continue.`,
  /**
   * The held composer's placeholder: Send is grey, and the empty box says why before anyone types
   * (live QA round 4, Q4-15). A finished task is not granted again, so it names the other way on.
   */
  chatBlockedPlaceholder: 'Grant access again to continue this chat',
  chatBlockedPlaceholderTaskFinished: 'Start a new chat to continue',
  /** A destination whose channel name this computer has not seen: the workspace instead. */
  chatDestinationWorkspace: (workspace: string) => `a channel in ${workspace}`,
  chatDestinationUnknown: 'a Crew channel',
  /** The Crew row of the chat's extension menu while a grant is active. */
  extensionLocked: 'On while this chat has Crew access. Revoke access to turn it off.',

  // ── Errors ─────────────────────────────────────────────────────────────────────────────────
  revokeFallback: 'Crew couldn’t complete that action.',
} as const;
