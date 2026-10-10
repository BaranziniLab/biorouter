/**
 * The chat transcript's strings: messages, their actions, markdown code blocks
 * and tables, the activity line, turn errors and the subagent line.
 *
 * Tests import these rather than retyping them (Crew's `copy.ts` pattern). Copy
 * rules (implementation spec, section 1): sentence case, "chat" never
 * "conversation", "Biorouter", the "…" ellipsis, no dashes as punctuation, and
 * no trailing period on a label. Help text that moved into an InfoTip is at most
 * two full sentences.
 */
export const transcriptCopy = {
  /** Message actions: the floating cluster on a message (Crew's row actions). */
  message: {
    edit: 'Edit',
    /** An Edit button's accessible name: the action and the start of the message. */
    editNamed: (text: string) =>
      `Edit message: ${text.length > 50 ? `${text.substring(0, 50)}…` : text}`,
    copy: 'Copy',
    copied: 'Copied',
    copyFailed: 'Copy failed',
    copyFailedHelp:
      'Biorouter could not write to the clipboard. Select the message and copy it with your keyboard.',
    more: 'More actions',
    divergeFromHere: 'Diverge from here',
    diverging: 'Diverging…',
    divergeHelp: 'Opens a new chat in a new window with the history up to here.',
    /** The time label, with the edit folded into it. */
    edited: (time: string) => (time ? `Edited · ${time}` : 'Edited'),
    showMore: 'Show more',
    showLess: 'Show less',
    sendAgain: 'Send again',
    sendAgainLabel: 'Send this message again',
  },

  /** Editing a sent message: one composer-recipe card inside the column. */
  edit: {
    textbox: 'Edit message content',
    placeholder: 'Edit message…',
    cancel: 'Cancel',
    cancelLabel: 'Cancel editing',
    save: 'Save',
    saveHelp: 'Updates this message and continues the chat from here.',
    saveAsNew: 'Save as new chat',
    saveAsNewHelp: 'Starts a new chat from the edited message. This chat stays as it is.',
    empty: 'Message cannot be empty',
  },

  /** A steer the turn ended without reading (D4), as a state word plus help. */
  steer: {
    notDelivered: 'Not delivered',
    notDeliveredHelp: 'Delivery is unconfirmed. Check the chat before sending it again.',
    notAnswered: 'Not answered',
    notAnsweredHelp: 'The turn ended before the agent read this message.',
  },

  /** Fenced code blocks and tables in markdown. */
  code: {
    /** The language label when a fence names none. */
    plain: 'text',
    copy: 'Copy',
    copyTip: 'Copy code',
    copied: 'Copied',
    copyFailed: 'Copy failed',
    copyFailedTip: 'Could not copy. Select the code and copy it with your keyboard.',
    run: 'Run',
    runTip: 'Run in the terminal below',
    sent: 'Sent',
    sentTip: 'Sent to the terminal below',
    terminalClosed: 'Terminal closed',
    terminalClosedTip: 'The terminal below has exited, so nothing was sent.',
    region: 'Scrollable code',
    tableRegion: 'Scrollable table',
    previewInPanel: (title: string) => `Preview ${title} in the side panel`,
    imageUnavailable: 'Image unavailable',
    imageUnavailableNamed: (alt: string) => `Image unavailable: ${alt}`,
    loadingImage: 'Loading image',
  },

  /** Inline images in messages. */
  image: {
    loading: 'Loading…',
    expand: 'Expand image',
    collapse: 'Collapse image',
    invalidPath: (path: string) => `Invalid image path: ${path}`,
    unavailable: (label: string) => `Unable to load image: ${label}`,
  },

  /** The trailing activity line and the loading line. */
  activity: {
    thinking: 'Thinking',
    thought: 'Thought',
    /** The chain-of-thought row's accessible name, by state. */
    showThinking: 'Show thinking',
    hideThinking: 'Hide thinking',
    compacting: 'Compacting the chat',
    waitingForAnswer: 'Waiting for your answer',
    runningOne: 'Running the tool',
    runningMany: (count: number) => `Running ${count} tools`,
    steering: 'Steering the current turn',
    steeringAfter: (tool: string) => `Your message will be added when ${tool} finishes`,
    loadingMessages: 'Loading messages…',
    loadingChat: 'Loading chat…',
    working: 'Working…',
    waiting: 'Waiting…',
    compactingShort: 'Compacting…',
    restarting: 'Restarting…',
    thinkingShort: 'Thinking…',
  },

  /** Turn errors and the stopped line. */
  turn: {
    retry: 'Retry',
    details: 'Details',
    stopped: 'Stopped',
  },

  /** The subagent line at the top of a delegated subagent's chat. */
  subagent: {
    of: 'Subagent of',
    context: 'Context',
    stop: 'Stop',
    stopLabel: 'Stop subagent',
    extensions: (count: number) => `${count} extension${count === 1 ? '' : 's'}`,
    knowledgeBases: (count: number) => `${count} knowledge base${count === 1 ? '' : 's'}`,
  },

  /** A workflow chat's starter prompts. */
  workflow: {
    label: 'Workflow',
  },
} as const;
