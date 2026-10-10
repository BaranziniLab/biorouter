/**
 * The chat composer's strings: the card, its `+` menu, Send and Stop, the
 * attachments row, the banners, the queue and the mention menu.
 *
 * One place, so the tests import what the interface says instead of restating
 * it (Crew's `copy.ts` pattern). Sentence case, the noun is "chat", the
 * ellipsis is "…", and nothing here is punctuated with a dash.
 */

export const COMPOSER_COPY = {
  /** Home, a new tab and an empty chat. `Hub.startFailure.test.tsx` reads it. */
  placeholderEmpty: 'Ask Biorouter anything…',
  /** A chat that already has messages. */
  placeholderFollowUp: 'Ask a follow-up',

  send: {
    label: 'Send message',
    tooltip: 'Send',
    waitingForImages: 'Waiting for images to save…',
    processingFiles: 'Processing dropped files…',
    restarting: 'Restarting chat…',
    starting: 'Starting chat…',
    loading: 'Loading chat…',
    /** The steer chord, named only while a turn is running. */
    steerHint: (chord: string) => `${chord} adds it to the running turn`,
  },

  stop: {
    label: 'Stop response',
    labelAcknowledged: 'Stopping response',
    tooltip: 'Stop',
    tooltipAcknowledged: 'Stopping…',
  },

  plus: {
    label: 'Add to message',
    attachFile: 'Attach file…',
    mention: 'Mention',
    mentionHint: '@',
    commands: 'Commands',
    commandsHint: '/',
  },

  attachments: {
    removeImage: 'Remove image',
    removeFile: 'Remove file',
    retry: 'Retry',
    retryLabel: 'Retry saving image',
    imageAlt: (name: string) => `Attached image ${name}`,
    pastedImageAlt: 'Pasted image',
    failed: 'Not attached',
    unknownType: 'File',
  },

  vision: {
    banner: "This model can't read images",
    help: 'Switch to a model that reads images, or remove the images to send.',
    action: 'Switch model',
  },

  queue: {
    header: 'Queued',
    paused: 'Paused',
    expand: (count: number) => `${count} message${count === 1 ? '' : 's'} queued. Expand queue.`,
    collapse: 'Collapse queue',
    clear: 'Clear queue',
    addNow: 'Add now',
    addNowLabel: 'Add this message to the current turn',
    addNowTooltip: 'Add to the running turn without stopping',
    stopAndSend: 'Stop & send',
    stopAndSendLabel: 'Stop the current turn and send this message as a new turn',
    stopAndSendTooltip: 'Stop the running turn, then send this',
    remove: 'Remove from queue',
    edit: 'Edit',
    save: 'Save',
    cancel: 'Cancel',
    cannotSendWhileEditing: 'Finish editing first',
    dragToReorder: 'Drag to reorder',
    attachments: (count: number) => `${count} attachment${count === 1 ? '' : 's'}`,
    fallbackLabel: 'Queued message',
  },

  mention: {
    searching: 'Searching…',
    noMatch: (query: string) => `Nothing matches "${query}"`,
    builtIn: 'Built in. Ships with Biorouter.',
  },
} as const;
