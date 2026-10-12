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
  /** The field's accessible name; the placeholder changes with the chat, the name does not. */
  inputLabel: 'Message',

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
    saving: 'Saving image…',
    removeImage: 'Remove image',
    removeFile: 'Remove file',
    retryLabel: 'Retry saving image',
    imageAlt: (name: string) => `Attached image ${name}`,
    pastedImageAlt: 'Pasted image',
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
    addNowLabel: 'Add this message to the current turn',
    addNowTooltip: 'Add to the running turn without stopping',
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
    /** The menu's accessible names: `@` lists mentions, `/` lists commands. */
    mentionsLabel: 'Mentions',
    commandsLabel: 'Commands',
    searching: 'Searching…',
    noMatch: (query: string) => `Nothing matches "${query}"`,
    /** An `@` with nothing typed after it, in a folder with nothing to list. */
    emptyFolder: 'No files in this folder',
  },

  /**
   * The one-line descriptions of the commands the client adds to the `/` list.
   * A menu description says what the command does, with no period and no
   * instruction to press a key.
   */
  commands: {
    knowledge: 'Use the Knowledge capability',
    crew: 'Connect this chat to a Crew channel',
    /** `/crew` in a chat with no session yet: Enter can only say to send a message first. */
    crewBeforeFirstMessage: 'Connect this chat to a Crew channel after your first message',
    /** The Crew extension's row, when a query reaches it without the `/crew` command. */
    crewExtension: 'Crew tools extension (advanced)',
    diverge: 'Continue in a new chat with the full history',
  },

  /**
   * The slash commands drawn as a chip (`utils/composerCommand.ts` keys its
   * registry on these): what the chip reads as, what it says on hover, and the
   * textarea's placeholder while the chip is there and the prose is empty.
   */
  chipCommands: {
    bug: {
      label: 'Report a bug',
      title: '/bug: Biorouter investigates this chat and drafts an issue for you to approve',
      placeholder: 'Describe what went wrong (optional)',
    },
  },
  /** A command chip's words for a screen reader, before its label. */
  chipCommandSpoken: (token: string) => `Command ${token}: `,
  /** A command chip's remove control. Removing it keeps the prose. */
  removeChipCommand: (token: string) => `Remove command ${token}`,

  /** Why a pasted or dropped image did not attach; each sits behind the chip's help. */
  imageErrors: {
    undecodable: 'Image preview could not be decoded.',
    unreadable: 'Failed to read image file.',
    saveFailed: 'Failed to save image via Electron.',
  },

  /** Warnings the composer raises when something is kept or refused. */
  toasts: {
    queued: {
      title: 'Message still queued',
      busy: 'Biorouter could not send it while the chat was busy. It is still in the queue, ready to send.',
      notStopped:
        'Biorouter could not confirm that the current turn stopped. Your message is still in the queue.',
    },
    region: {
      title: 'Region not attached',
      tooMany: (max: number) => `A message can contain at most ${max} images.`,
      unreadable: 'The selected region could not be read.',
    },
    crewDraftKept: {
      title: 'Draft kept',
      message:
        'Remove the attached files, images, and reference chips before using /crew, or open Crew from the sidebar. Nothing was sent.',
    },
    crewStartFirst: {
      title: 'Start the chat first',
      message:
        'Send this chat a message, then type /crew to connect it to a Crew channel. To just open Crew, use the sidebar.',
    },
    divergeStartFirst: {
      title: 'Start the chat first',
      message: '/diverge continues a chat in a new one. Send a message before using it.',
    },
  },
} as const;
