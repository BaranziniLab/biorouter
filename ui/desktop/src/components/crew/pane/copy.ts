/**
 * The details pane's strings (ui-redesign-spec, copy deck "Channel header, menu and pane" and
 * "Ask my agent").
 *
 * Tests import these instead of retyping them. A string marked pinned is asserted by a regression
 * test or cited by acceptance evidence; change it only together with that test.
 */

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

/** `"A"`, `"A or B"`, `"A, B or C"`. */
function joinOr(parts: readonly string[]): string {
  if (parts.length <= 1) return parts[0] ?? '';
  return `${parts.slice(0, -1).join(', ')} or ${parts[parts.length - 1]}`;
}

/** How many channels "Also reads …" names before it counts the rest. */
const SUMMARY_CHANNELS = 3;

export const paneCopy = {
  /**
   * The ×'s name in each mode: it says what it closes (Q2-68). "Close details" on Ask my agent
   * named a pane that was not open.
   */
  close: 'Close details',
  closeAgent: 'Close Ask my agent',
  closeChatAccess: 'Close chat access',
  /** `channel` is already `#slug`. */
  back: (channel: string) => `Back to ${channel}`,
  /**
   * The details mode's title while the pane covers the channel. "Back to #name" already names the
   * channel beside it, so the title would otherwise read "← Back to #general  #general" (T-46).
   */
  coverTitle: 'Details',
  /** The details mode's landmark name. */
  detailsName: (channel: string) => `${channel} details`,
  chatAccessTitle: 'Chat access',
  tabsLabel: 'Channel details',
  /**
   * "Agent access", not "Access": one name for what the header chip, this tab and Workspace
   * settings each used to call something different (Q2-66), and the one that says whose access
   * the tab lists.
   */
  tabs: { about: 'About', members: 'Members', files: 'Files', access: 'Agent access' },
} as const;

export const aboutCopy = {
  name: 'Name',
  /** The classification row, said as the question a person asks of it (T-67). */
  whoCanRead: 'Who can read',
  owner: 'Owner',
  createdBy: 'Created by',
  team: 'Team',
  restricted: 'Private models only',
  /** Ties the answer to the "Restricted" chip the channel header shows. */
  restrictedHint: 'Marked Restricted, so public models can’t read it.',
  publicSafe: 'Any model the workspace allows',
  publicSafeHint: 'Marked Public-safe, so public models may read it when the workspace allows.',
  offeredTo: 'Offered to',
  waiting: 'waiting',
  rename: 'Rename…',
  renameName: 'Rename channel',
  transfer: 'Transfer ownership…',
  dangerZone: 'Danger zone',
  archive: 'Archive channel…',
  copyId: 'Copy channel ID',
  /** Copy channel ID's label for a moment after a copy (Q2-34). */
  copied: 'Copied',
  copyFailed: 'Couldn’t copy',
} as const;

export const membersCopy = {
  addPeople: 'Add people…',
  count: (count: number) => plural(count, 'member', 'members'),
  /**
   * The owner's badge. "Channel owner", not "Owner": the workspace's People list calls its own
   * role "Host", and a bare "Owner" beside it read as a second word for the same thing (Q2-69).
   */
  owner: 'Channel owner',
  invited: 'invited',
  makeOwner: 'Make owner…',
  /** `channel` is already `#slug`. */
  remove: (channel: string) => `Remove from ${channel}…`,
  /** Without the `@`, as Workspace settings copies it. */
  copyUsername: 'Copy username',
  copyPersonId: 'Copy person ID',
  /** A copy item's label for a moment after it is chosen; the menu then closes (Q2-34). */
  copied: 'Copied',
  copyFailed: 'Couldn’t copy',
  more: (person: string) => `More actions for ${person}`,
  /** Split around the owner's name, which renders as a `PersonName`. */
  askBefore: 'Ask ',
  askAfter: ' to add people.',
  listLabel: (channel: string) => `${channel} members`,
} as const;

export const agentCopy = {
  /** Pinned. */
  title: 'Ask my agent',
  /** `channel` is already `#slug`. */
  destination: (channel: string, team: string, workspace: string) =>
    `Posts to ${channel} in ${team} · ${workspace}`,
  /** Pinned. */
  task: 'Task',
  taskPlaceholder: 'What should your agent do?',
  /**
   * The daemon posts the whole task into the channel as "Task: {prompt}" before the agent starts
   * (`routes/crew.rs`, the run's first post), so the field says so where it is written (T-24).
   * `channel` is already `#slug`.
   */
  taskPosted: (channel: string) =>
    `Your task is posted in ${channel} so everyone there can see what your agent was asked.`,
  /** Added once the task is long enough to be pasted data (see `LONG_TASK_LINES`). */
  taskLong: 'Long pasted data will be visible to the channel.',
  /** Pinned accessible name of the model control. */
  model: 'Model',
  modelEmpty: 'Choose a model',
  /** A model as the composer's model chip names it: its display name, then its provider's. */
  modelChoice: (model: string, provider: string) => `${model} · ${provider}`,
  modelChange: 'Change',
  modelChangeName: 'Change model',
  modelRequired: 'Choose a model.',
  modelUse: (text: string, provider: string) => `Use “${text}” with ${provider}`,
  modelPrivate: 'Private',
  modelsLabel: 'Models',
  searchModels: 'Search models',
  loadingModels: 'Loading models…',
  noMatch: 'No models match.',
  noModels: 'No models are set up.',
  openSettings: 'Open Settings',
  /**
   * Advanced's closed summary (Q2-67): "Reads only #general" or "Also reads #methods, #qc". A
   * remote folder is added as its own line's words, because "Reads only" would then be false.
   * `channel` and each of `others` are already `#slug` (or `Team / #slug`).
   */
  advancedSummary: (channel: string, others: readonly string[], folder: string | null = null) => {
    const shown = others.slice(0, SUMMARY_CHANNELS);
    const rest = others.length - shown.length;
    const reads =
      others.length === 0
        ? folder
          ? null
          : `Reads only ${channel}`
        : `Also reads ${shown.join(', ')}${rest > 0 ? ` and ${rest} more` : ''}`;
    return [reads, folder].filter(Boolean).join(' · ');
  },
  alsoRead: 'Also read',
  folderExec: (path: string) => `Can run commands in ${path}`,
  folderRead: (path: string) => `Can read ${path}`,
  publicHint: 'This model is Public, so it can’t read Restricted channels.',
  /**
   * The model's tier mark, said in the task's words (Q2-67), never the chat badge's "this chat".
   * `privateOnly` holds only where the task's context is protected (a Private workspace or
   * connection, a Restricted channel, a remote folder), which is where the daemon refuses a public
   * model; elsewhere the mark says what a private model adds.
   */
  privateOnly: 'Private. Only private models can run this task.',
  privateModel: 'Private. Unlike a public model, it may read Restricted channels.',
  publicModel: 'Public. It can’t read Restricted channels.',
  /**
   * The task names a file the channel's loaded messages do not share (Q2-15). It does not block
   * Start; the agent is told to say what it used instead. `channel` is already `#slug`.
   */
  fileNotShared: (names: readonly string[], channel: string) =>
    names.length === 1
      ? `No file named ${names[0]} is shared in ${channel}. Your agent will say what it used instead.`
      : `No files named ${joinOr(names)} are shared in ${channel}. Your agent will say what it used instead.`,
  /** A picker row whose model the workspace's institution has not approved. */
  notApproved: (institution: string) => `Not approved for ${institution}`,
  /**
   * Said before Start, and in place of the daemon's "…the model's resolved affiliation…" refusal,
   * which is the same fact in internal words (T-47). `affiliation` is who approved the model.
   */
  institutionMismatch: (
    model: string,
    affiliation: string,
    workspace: string,
    institution: string
  ) =>
    `${model} is approved for ${affiliation}. ${workspace} uses ${institution}. Choose a model approved for ${institution}, or a local model.`,
  /** The same, for a private model that states no institution. */
  institutionUnstated: (model: string, workspace: string, institution: string) =>
    `${model} doesn’t say which institution approved it. ${workspace} uses ${institution}. Choose a model approved for ${institution}, or a local model.`,
  /** The daemon's refusal when the pane cannot tell who approved the model. */
  institutionRefused: (model: string, institution: string | null) =>
    institution
      ? `${model} isn’t approved for ${institution}. Choose a model approved for ${institution}, or a local model.`
      : `${model} isn’t approved for this workspace’s institution. Choose a model approved for it, or a local model.`,
  /**
   * True only because the daemon revokes the run's posting grant once the task finishes, on
   * success as well as failure and cancel (T-25). Keep the two together.
   */
  scope: (channel: string) => `Your agent can read ${channel} and post there, for this task only.`,
  /** Pinned. */
  start: 'Start my agent and allow posting here',
  /** Start's label, beside a spinner, from the click until the pane closes (Q2-67). */
  starting: 'Starting…',
} as const;

/** A task longer than this many lines (or `LONG_TASK_CHARS` characters) reads as pasted data. */
export const LONG_TASK_LINES = 10;
export const LONG_TASK_CHARS = 1000;

export const unknownOutcomeCopy = {
  /** Pinned. */
  title: 'Inspect the previous task before starting again',
  body: (destination: string) =>
    `The request to ${destination} was received, but Crew can’t tell whether it started. Repeating it could duplicate its effects.`,
  /** `channel` is already `#slug`. */
  destination: (channel: string, team: string) => `${channel} in ${team}`,
  showTask: 'Show task in channel',
  openHistory: 'Open chat history',
  checked: 'I checked the previous task and its effects.',
  /** Pinned. */
  restart: 'Start a new task',
} as const;
