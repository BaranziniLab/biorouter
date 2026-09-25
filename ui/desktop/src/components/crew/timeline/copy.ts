/**
 * The timeline's strings (ui-redesign-spec, copy deck "Timeline").
 *
 * Tests import these rather than retyping them. A string marked pinned is
 * asserted by a regression test or cited by acceptance evidence; change it only
 * together with that test. People and agents are named by the identity layer
 * (`identityCopy`: "Your agent", "{name}'s agent", "Unknown member", "former
 * member"), and run status words come from `state/crewStatus.ts`, so neither is
 * repeated here.
 */
export const timelineCopy = {
  /** The log's accessible name, `{name} messages` (the channel slug, no `#`). */
  logLabel: (name: string) => `${name} messages`,
  /** The log's description: its keyboard model, which nothing on screen shows. */
  logDescription:
    'Use Up and Down to move between messages, and Home and End to reach the first and last. Tab reaches the message’s actions.',
  /** Pinned. The sentinel at the top of a full page of history. */
  older: 'Older messages',
  loadingOlder: 'Loading earlier messages…',
  loadingMessages: 'Loading messages…',

  /** Pinned: `Welcome to #{name}`. */
  introTitle: (name: string) => `Welcome to #${name}`,
  /** Follows the creator, formatted by `PersonName` in its inline context. */
  introCreatedBy: ' created this channel.',
  introAddPeople: 'Add people',
  /**
   * For anyone but the workspace's host: whom to ask for the channels they are not in. The
   * sidebar already says that other channels appear once someone adds you, so this says only
   * the next step (Q2-64). `host` is the host in the authority form, "Iris Wong (@crew_iris)"
   * (Q4-21); without one there is nothing to say (`''`).
   */
  introOtherChannels: (host: string | null) =>
    host ? `Ask ${host} to add you to other channels.` : '',

  today: 'Today',
  yesterday: 'Yesterday',

  /** The unread rule's label. Accent ink, never danger: unread is live state, not a failure. */
  newLine: 'New',
  /** The rule's accessible name; contains the visible "New". */
  newLineLabel: 'New messages',

  agentBadge: 'Agent',
  restricted: 'Restricted',
  restrictedTooltip: 'Only private models can read this message.',
  /** The visible label (tooltip, menu item) of a message's copy action. */
  copyText: 'Copy text',
  /**
   * A row action's accessible name: the action, then whose message and when, so a list of
   * buttons never reads "Copy text, Copy text, …". `who` comes from `personLabel`.
   */
  copyTextOf: (who: string, time: string) => `Copy text of ${who}’s message, ${time}`,
  /**
   * The time part of a row's action names when the same author posted more than once that
   * minute: "10:02 AM, 2 of 2", so no two buttons read the same (Q2-57).
   */
  timeInMinute: (time: string, index: number, count: number) => `${time}, ${index} of ${count}`,
  copyMessageId: 'Copy message ID',
  /**
   * The submenu every menu keeps its machine-ID copies in, last and after a separator (Q3-26):
   * a person's own copies stay at the top level, and an ID is one step further away.
   */
  copyForSupport: 'Copy for support',
  moreActions: 'More actions',
  moreActionsFor: (who: string, time: string) => `More actions for ${who}’s message, ${time}`,
  /** Pinned: what the copy control itself says for two seconds after a copy. */
  copied: 'Copied',
  /** Pinned: what the copy control itself says when the clipboard refused. */
  copyFailedShort: 'Couldn’t copy',
  /** The polite announcement of a refused copy: what to do instead. */
  copyFailed: 'Couldn’t copy. Select the text and copy it instead.',

  /** A post the broker accepted, until the observer delivers it into the log. */
  sending: 'Sending…',

  /** Long-message fold (`utils/messageClamp.ts` decides; this names the control). */
  showMore: 'Show more',
  showLess: 'Show less',

  /** An agent's step-by-step tool updates, folded behind one control. */
  showDetails: 'Show details',
  detailsSummary: (count: number) => `${count} ${count === 1 ? 'update' : 'updates'}`,

  /** Markdown code block chrome. */
  code: 'code',
  copyCode: 'Copy code',
  /** A code block wide enough to scroll: its region's name, with its language when it has one. */
  codeRegion: (language: string) => (language ? `Code: ${language}` : 'Code'),
  /**
   * A table wide enough to scroll: its region's name, from its header cells, so no two tables
   * read the same (Q2-57). `headers` is the cells joined with ", ".
   */
  tableNamed: (headers: string) => (headers ? `Table: ${headers}` : 'Table'),
  /** A message image is never fetched; it is a link the person may choose to open. */
  image: 'Image',
  imageNamed: (alt: string) => `Image: ${alt}`,

  /** Pinned. */
  viewingEarlier: 'Viewing earlier messages',
  jumpToLatest: 'Jump to latest',
  /**
   * The live pill, when messages arrived below while the reader was scrolled up (Q3-27): how many,
   * so the pill says what it would take them to. The arrow is the pill's glyph, after the words.
   */
  newMessages: (count: number) => `${count} new ${count === 1 ? 'message' : 'messages'}`,
  /** Read after the count, so the button still says what it does. */
  newMessagesAction: ', jump to latest',

  /**
   * The head of the viewer's own agent's post, when a chat of theirs posted it (Q3-22): "Your
   * agent" and then the chat's title, which opens that chat. Only the viewer's own grants carry a
   * title, so another person's agent is never named by their chat.
   */
  openAgentChat: (title: string) => `Open ${title}`,

  taskOpen: 'Open',
  taskOpenLabel: 'Open agent conversation',
  taskReview: 'Review',
  taskReviewLabel: 'Review in agent conversation',
  taskStop: 'Stop',
  taskStopLabel: 'Stop task',
  taskStopAgain: 'Try stopping again',
  taskMoreActions: 'More task actions',
  taskCopyId: 'Copy task ID',
  /** Goes to the chat history list; "Open" is the row's own way into the task's chat (Q2-62). */
  taskOpenHistory: 'Show in chat history',
  taskCopyError: 'Copy error',
  /** The task row's accessible name, when the task's first line is not loaded. */
  taskRowLabel: 'Your agent’s task',

  /**
   * The Stop confirmation. `dialogs/confirmations.tsx` renders it for the
   * `stop-task` confirm intent this timeline opens; kept here because the copy
   * deck files it under Timeline.
   */
  stopConfirm: {
    title: 'Stop your agent?',
    body: 'It stops working on this task. Anything it already did stays done.',
    cancel: 'Keep running',
    confirm: 'Stop task',
  },
} as const;
