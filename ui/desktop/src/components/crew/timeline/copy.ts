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
   * For a member who does not own the channel: why they may not see every channel, and who to
   * ask. `owner` is `@username`; without one the first sentence stands alone.
   */
  introOtherChannels: (owner: string | null) =>
    owner
      ? `Only channels you’ve been added to appear here. Ask ${owner} to add you to others.`
      : 'Only channels you’ve been added to appear here.',

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
  copyMessageId: 'Copy message ID',
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
  /** A message image is never fetched; it is a link the person may choose to open. */
  image: 'Image',
  imageNamed: (alt: string) => `Image: ${alt}`,

  /** Pinned. */
  viewingEarlier: 'Viewing earlier messages',
  jumpToLatest: 'Jump to latest',

  taskOpen: 'Open',
  taskOpenLabel: 'Open agent conversation',
  taskReview: 'Review',
  taskReviewLabel: 'Review in agent conversation',
  taskStop: 'Stop',
  taskStopLabel: 'Stop task',
  taskStopAgain: 'Try stopping again',
  taskMoreActions: 'More task actions',
  taskCopyId: 'Copy task ID',
  taskOpenHistory: 'Open chat history',
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
