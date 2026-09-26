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
   * Under a chip whose file has the name of one already in the channel, while a checksum is not
   * known yet, so it cannot say whether it is the same file (Q3-13). A note, not a question:
   * sharing it again stays allowed. `channel` is the slug, `when` the earlier post's time.
   */
  alreadyShared: (name: string, channel: string, when: string) =>
    `${name} is already in #${channel}${when ? ` (shared ${when})` : ''}. Remove this one if it’s the same file.`,
  /**
   * Under a chip whose contents are a file already in the channel: both checksums known and equal
   * (Q4-18). `earlier` is that file's name, `when` its post time ('' when not known).
   */
  sameFileShared: (name: string, earlier: string, channel: string, when: string) => {
    const where = when ? `shared at ${when}` : `already in #${channel}`;
    const which = earlier === name ? `the one ${where}` : `${earlier}, ${where}`;
    return `${name} is the same file as ${which}. You can remove it.`;
  },
  /**
   * Under a chip that has a file's name but not its contents: a corrected file (Q4-18). The run
   * that names it reads the newest copy, which is this one once it is sent.
   */
  differentFileShared: (name: string, channel: string, when: string) =>
    when
      ? `A different ${name} was shared at ${when}. Agents will use this newer one once you send it.`
      : `A different ${name} is already in #${channel}. Agents will use this newer one once you send it.`,
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
