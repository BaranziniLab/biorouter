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
  /** The details mode's landmark name. */
  detailsName: (channel: string) => `${channel} details`,
  chatAccessTitle: 'Chat access',
  tabsLabel: 'Channel details',
  tabs: { about: 'About', members: 'Members', files: 'Files', access: 'Access' },
} as const;

export const aboutCopy = {
  name: 'Name',
  content: 'Content',
  owner: 'Owner',
  createdBy: 'Created by',
  team: 'Team',
  restricted: 'Restricted',
  restrictedHint: 'Public models can’t read it.',
  publicSafe: 'Public-safe',
  publicSafeHint: 'Public models may read it when the workspace allows.',
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
  /** Pinned accessible name of the model control. */
  model: 'Model',
  modelEmpty: 'Choose a model',
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
  scope: (channel: string) => `Your agent can read ${channel} and post there, for this task only.`,
  /** Pinned. */
  start: 'Start my agent and allow posting here',
} as const;

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
