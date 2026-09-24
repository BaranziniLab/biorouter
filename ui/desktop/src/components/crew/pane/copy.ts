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

export const paneCopy = {
  close: 'Close details',
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
  tabs: { about: 'About', members: 'Members', files: 'Files', access: 'Access' },
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
} as const;

export const membersCopy = {
  addPeople: 'Add people…',
  count: (count: number) => plural(count, 'member', 'members'),
  owner: 'Owner',
  invited: 'invited',
  makeOwner: 'Make owner…',
  /** `channel` is already `#slug`. */
  remove: (channel: string) => `Remove from ${channel}…`,
  /** Without the `@`, as Workspace settings copies it. */
  copyUsername: 'Copy username',
  copyPersonId: 'Copy person ID',
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
  advancedSummary: (count: number) =>
    count === 0 ? 'Also reads nothing else' : `Also reads ${plural(count, 'channel', 'channels')}`,
  alsoRead: 'Also read',
  folderExec: (path: string) => `Can run commands in ${path}`,
  folderRead: (path: string) => `Can read ${path}`,
  publicHint: 'This model is Public, so it can’t read Restricted channels.',
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
