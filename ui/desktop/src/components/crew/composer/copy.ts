/**
 * The composer's strings (ui-redesign-spec, copy deck "Composer and files": `composer.*`).
 *
 * Tests import these instead of retyping them. A string marked pinned is asserted by a
 * regression test or cited by acceptance evidence; change it only together with that test.
 * `name` is the channel's slug, without the `#`.
 */
export const composerCopy = {
  /** Pinned: the textarea's accessible name. */
  label: (name: string) => `Message #${name}`,
  placeholder: (name: string) => `Message #${name}`,
  attach: 'Attach',
  upload: 'Upload a file…',
  sharePath: 'Share a server path…',
  /** Pinned. */
  askAgent: 'Ask my agent',
  /** Pinned: Send's accessible name. */
  send: 'Send message',
  sendTooltip: 'Send',
  sending: 'Sending…',
  /** Pinned: the attachment chip's remove control. */
  removeFile: (name: string) => `Remove ${name}`,
  /** Pinned: the server-path chip's remove control. */
  removeRef: (label: string) => `Remove remote reference ${label}`,
  archived: 'This channel is archived.',
  verifying: 'Verifying access…',
  /** The send error's lead; the error follows in its own text node. */
  sendErrorLead: 'Couldn’t send.',
  postedMetadata: 'Message sent, but its upload record couldn’t be cleared. Remove it from Files.',
} as const;
