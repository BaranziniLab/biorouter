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

  // A send refusal, in words for a person (QA M1, M5, R-2, FILES-F9, `composer/sendFailure.ts`).
  // Each follows "Couldn't send." in the note above the card.
  /**
   * `invalid_params: message too long`, and the same limit found before sending: the broker takes
   * at most 64 KB of text (65,536 UTF-8 bytes) in one message. The manual says to attach it.
   */
  tooLong: 'Messages can be up to 64 KB. Attach long text as a file.',
  /**
   * Above the card while the draft is over what Crew keeps when the person switches channel or
   * leaves Crew (`DRAFT_STASH_MAX_BODY_BYTES`, 1 MB), so they learn it before the words go
   * (MSG2-N4).
   */
  draftTooLongToKeep:
    'This draft is too long to keep when you switch channel. Attach it as a file.',
  /**
   * `storage_failed` / `storage_full`, or the host's disk full as a post was written: nothing more
   * can be saved until the host restarts the workspace server. `host` is the host in the authority
   * form ("Iris Wong (@crew_iris)"), or null when the viewer cannot see who that is.
   */
  storageFailed: (host: string | null) =>
    `The workspace server can’t save messages right now. Ask ${host ?? 'the workspace host'} to restart Crew.`,
  /** The same, said to the host. */
  storageFailedHost:
    'The workspace server can’t save messages right now. Restart Crew on the server, then send again.',
  /**
   * `storage_full`, or the host's disk full as a post was written (`No space left on device`), said
   * to the host: a restart alone would stall again, so space comes first (MSG2-N6). A member is
   * told as for any storage failure (`storageFailed`), since what they do is the same.
   */
  diskFullHost: 'The server is out of disk space. Free space on it, then restart Crew there.',
  /**
   * `forbidden: attachment provenance cannot be dropped`: a file uploaded in another channel.
   * `file` and `channel` (`#name`) when this computer knows them.
   */
  fileElsewhere: (file: string | null, channel: string | null) =>
    file && channel
      ? `${file} was shared in ${channel}. Share it there, or upload it again here.`
      : file
        ? `${file} was shared in another channel. Share it there, or upload it again here.`
        : 'A file in this message was shared in another channel. Remove it, then share it there or upload it again here.',
  /** `forbidden: reference provenance cannot be dropped`: a server path from another channel. */
  pathElsewhere:
    'A server path in this message was shared in another channel. Remove it, then share the path again here.',
  /** `channel_archived: channel is read-only`. */
  archivedRefusal: 'This channel is archived, so nothing more can be posted in it.',
  /** `crew_not_sent`: the bridge was lost before the post was written. */
  notSent: 'Nothing was sent. Check the connection, then send again.',
  /** The daemon's SSH failure, or its "disconnected" refusal, from before it had a code. */
  notReached:
    'Crew lost its connection to the workspace, so nothing was sent. Send again once it’s back.',

  // A post whose outcome is unknown (QA R-4): the bridge died after it was written.
  /** While the channel is read again to find it. A status, not an error. */
  checking: 'Checking whether your message reached the channel…',
  /** It was not there once the channel had been read again. The draft and its key stay. */
  unconfirmed: 'Couldn’t confirm this was sent. Check the channel, then send again.',
  /** It was there: the draft goes, as after any send. */
  confirmed: 'Your message was sent.',

  /**
   * Several files dropped or pasted at once (DW-18): Crew takes the first, which the note names,
   * and says so in a note the person closes, as the manual's table of refused files says.
   */
  oneFileAtATime: (name: string) => `Crew shares one file at a time. Only ${name} was added.`,

  // Starting an agent from the composer: `@agent`, `/agent`, `/task` (design OW-10). `channel` is
  // the slug, without the `#`; a model is named as Ask my agent names it ("GPT-5.5 · Versa").
  /**
   * Pinned: Send's accessible name while the draft starts a task. Never "Ask my agent", which is the
   * button beside it (ui-redesign-spec C6); "Send message" stays the name for every other draft.
   */
  startTask: 'Start agent task',
  /** Send's tooltip while the draft starts a task. */
  startTaskTooltip: 'Start your agent',
  /** A start from the composer that failed; the reason follows in its own text node. */
  startErrorLead: 'Couldn’t start your agent.',
  /** The suggestion list's name. */
  suggestions: 'Suggestions',
  /** The `@agent` row's second line. `model` is null while it is not known yet. */
  agentRowDetail: (model: string | null, channel: string) =>
    model ? `Your agent · ${model} · posts in #${channel}` : `Your agent · posts in #${channel}`,
  /** The `/agent` and `/task` rows' second line. */
  slashRowDetail: (channel: string) => `Start your agent on a task in #${channel}`,
  /** Someone is called `agent`, so `@agent` mentions them. `person` in the authority form. */
  agentNameTaken: (person: string) => `@agent is ${person}. Type /agent to ask your agent.`,
  /**
   * The consent line under the text while the draft starts a task: what Enter does, before Enter,
   * because Enter is the consent (OW-10). The same facts Ask my agent shows: the model and its tier,
   * where it posts, what it reads, and what it may do on the server. The model's tier mark follows
   * `agentStartsWith` as its own chip.
   */
  agentStartsWith: (model: string) => `Enter starts your agent with ${model}`,
  /** True only because the daemon revokes the task's posting grant once it finishes (T-25). */
  agentReads: (channel: string) =>
    `It reads #${channel} and posts its result there, for this task only.`,
  /** The same, in an open workspace where a task reads every channel the person is in. */
  agentReadsWide: (count: number, channel: string) =>
    `It reads ${count} channels and posts its result in #${channel}, for this task only.`,
  /** The model the line names would be refused here; `blockText` (Ask my agent's words) first. */
  agentOpensPicker: 'Enter opens Ask my agent to choose another model.',
  agentNoModel: 'No model is set for Crew tasks. Enter opens Ask my agent to choose one.',
  agentLoading: 'Checking your model for Crew tasks…',
  /** A trigger with no task after it. */
  agentEmpty: 'Type the task after it, or press Enter to open Ask my agent.',
  /** The last start's outcome is unknown: the pane's gate decides what happens next (C9 to C12). */
  agentUnknownOutcome: 'Check your previous task first. Enter opens Ask my agent.',
  /** Attachments with `@agent`: a task is words only, so nothing starts. */
  agentFiles: '@agent starts a task from words only. Send the files first, then ask your agent.',
  /** Over the daemon's 32,768 bytes for a task (`TASK_PROMPT_MAX_BYTES`). Nothing starts. */
  agentTooLong: 'A task can be up to 32 KB. Attach long text as a file.',
  /** Opens Ask my agent on its model picker, with the task's words. */
  agentChange: 'Change',
  agentChangeName: 'Change the model for this task',
  /** Sends the draft as an ordinary message, `@agent` and all. */
  postAsMessage: 'Post as a message',
  /** While the person chose to post a draft that would start a task. */
  postingAsMessage: 'Enter posts this as a message.',
  startTaskInstead: 'Start a task instead',
  /** A start's refusal of the model: Ask my agent opens on its picker with the task's words. */
  chooseAnotherModel: 'Choose another model',
  /** A start whose outcome is unknown: Ask my agent opens on its gate. */
  openAskMyAgent: 'Open Ask my agent',
} as const;
