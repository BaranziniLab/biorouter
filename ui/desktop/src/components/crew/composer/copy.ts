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
  sending: 'Sending…',
  /** The chips row: what goes with the message. */
  chips: 'Attachments',
  /**
   * Pinned: the attachment chip's remove control. It takes the file out of THIS message; the
   * upload itself stays on the server, which its tooltip says, because the broker cannot delete
   * an uploaded file yet (Q3-14).
   */
  removeFile: (name: string) => `Remove ${name} from this message`,
  removeFileHelp: (server: string) =>
    `Removes it from this message. The copy already uploaded stays on ${server || 'the server'}.`,
  /**
   * Under the chips, once, while a finished file waits in the draft: a drop or a paste only
   * attaches, and nothing reaches the channel until Send (Q3-14).
   */
  pressSend: (count: number) =>
    count === 1 ? 'Press Send to share it.' : 'Press Send to share them.',
  /**
   * Under a chip whose file is already in the channel — the same name, or the same contents once
   * its checksum is known (Q3-13). A note, not a question: sharing it again stays allowed.
   * `channel` is the slug, `when` the earlier post's time.
   */
  alreadyShared: (name: string, channel: string, when: string) =>
    `${name} is already in #${channel}${when ? ` (shared ${when})` : ''}. Remove this one if it’s the same file.`,
  /** Pinned: the server-path chip's remove control. */
  removeRef: (label: string) => `Remove remote reference ${label}`,
  archived: 'This channel is archived.',
  verifying: 'Verifying access…',
  /**
   * The upload failure's dismiss control: its accessible name and tooltip. One word, as every
   * other dismiss control says (Q2-61); the note it closes is the context.
   */
  dismissUploadError: 'Dismiss',
  /** The send error's lead; the error follows in its own text node. */
  sendErrorLead: 'Couldn’t send.',
  postedMetadata: 'Message sent, but its upload record couldn’t be cleared. Remove it from Files.',
} as const;
