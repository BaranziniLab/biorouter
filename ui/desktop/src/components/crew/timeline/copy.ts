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
  /** Pinned. The sentinel at the top of a full page of history. */
  older: 'Older messages',
  loadingOlder: 'Loading earlier messages…',
  loadingMessages: 'Loading messages…',

  /** Pinned: `Welcome to #{name}`. */
  introTitle: (name: string) => `Welcome to #${name}`,
  /** Follows the creator, formatted by `PersonName` in its inline context. */
  introCreatedBy: ' created this channel.',
  introAddPeople: 'Add people',

  today: 'Today',
  yesterday: 'Yesterday',

  /** The unread rule's label. Accent ink, never danger: unread is live state, not a failure. */
  newLine: 'New',
  /** The rule's accessible name; contains the visible "New". */
  newLineLabel: 'New messages',

  agentBadge: 'Agent',
  restricted: 'Restricted',
  restrictedTooltip: 'Only private models can read this message.',
  copyText: 'Copy text',
  copyMessageId: 'Copy message ID',
  moreActions: 'More actions',
  copied: 'Copied',
  copyFailed: 'Couldn’t copy. Select the text and copy it instead.',

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
