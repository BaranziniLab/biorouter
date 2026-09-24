/**
 * The dialogs area's strings (ui-redesign-spec, copy deck "Dialogs and confirmations", "Onboarding,
 * join, host and admit" for invite and let in, and "Error strings").
 *
 * Tests import these instead of retyping them. A string marked pinned is asserted by a regression
 * test; change it only together with that test. `{workspace}` is the workspace's S2 name, else the
 * connection name; `{first}` is a person's first name, else `@username` (`firstName` in
 * `people.ts`). People are formatted by `personLabel`, never here.
 */
export const connectionSettingsCopy = {
  title: 'Connection settings',
  name: 'Connection name',
  login: 'Your server login',
  loginPlaceholder: 'you@server.example.edu',
  privacy: 'Privacy',
  private: 'Private',
  privateDetail: 'Only private and institution-approved models',
  public: 'Public',
  publicDetail: 'Public models allowed for public-safe work',
  institution: 'Institution',
  /** Pinned: the institution field's placeholder. */
  institutionPlaceholder: 'For example, ucsf or sdsc',
  /** Shown under the institution field only while it is empty. */
  institutionHelper: 'Your organization’s short ID, as your host uses it.',
  institutionPattern:
    'Use lowercase letters, numbers, hyphens and underscores, starting with a letter or number.',
  advanced: 'Advanced',
  advancedSummary: (parts: string[]) => parts.join(' · '),
  summaryPort: (port: string) => `Port ${port}`,
  summarySshDefaults: 'your SSH settings',
  summaryIdentity: 'identity file',
  summaryJump: (host: string) => `via ${host}`,
  summaryFolder: (path: string) => `folder ${path}`,
  port: 'Port',
  identityFile: 'Identity file',
  identityFilePlaceholder: '~/.ssh/id_ed25519',
  jumpHosts: 'Jump hosts',
  jumpHostsPlaceholder: 'gateway.example.edu',
  remoteRoot: 'Remote work folder',
  remoteRootPlaceholder: '/home/you/project',
  remoteRootPattern: 'Use an absolute path that starts with /.',
  remoteExecution: 'Let my agent run commands in this folder',
  remoteExecutionNeedsFolder: 'Set a remote work folder first.',
  portRange: 'Use a port from 1 to 65535.',
  workspaceDetails: 'Workspace details',
  workspaceId: 'Workspace ID',
  fingerprint: 'Fingerprint',
  workspaceKey: 'Workspace key',
  socketPath: 'Socket path',
  hostUserId: 'Host user ID',
  deviceId: 'Device ID',
  clusterId: 'Cluster ID',
  /** Pinned: the submit button. */
  save: 'Save connection',
  cancel: 'Cancel',
  remove: (workspace: string) => `Remove ${workspace} from this computer…`,
} as const;

export const workspaceSettingsCopy = {
  title: (workspace: string) => `${workspace} settings`,
  tabs: {
    general: 'General',
    people: 'People',
    privacy: 'Privacy',
    agentAccess: 'Agent access',
  },
  hostedBy: 'Hosted by',
  server: 'Server',
  rename: 'Rename…',
  waiting: 'Waiting to join',
  members: 'Members',
  invite: 'Invite people…',
  host: 'Host',
  you: 'you',
  letIn: 'Let in…',
  letInLabel: (username: string) => `Let @${username} in`,
  otherDevice: (username: string) =>
    `A device with a different code tried to join as @${username}.`,
  cancelInvitation: 'Cancel invitation',
  cancelInvitationLabel: (username: string) => `Cancel @${username}’s invitation`,
  cancelInvitationConfirm: (username: string) => `Cancel @${username}’s invitation?`,
  keepInvitation: 'Keep',
  memberOptions: (name: string) => `${name} options`,
  copyUsername: 'Copy username',
  copyPersonId: 'Copy person ID',
  removeFrom: (workspace: string) => `Remove from ${workspace}…`,
  yourConnection: 'Your connection',
  workspace: 'Workspace',
  privateForEveryone: 'Private for everyone',
  allowsPublic: 'Allows Public',
  allowPublic: 'Allow Public…',
  makePrivateForEveryone: 'Make Private for everyone…',
  makePublic: 'Make public…',
  makePrivate: 'Make private',
  institution: 'Institution',
  notSet: 'Not set',
  setInstitution: (id: string) => `Set institution to ${id}…`,
  institutionNeedsConnection: 'Add your institution in Connection settings first.',
  hostOnly: 'Only the host can change this.',
  noMembers: (workspace: string) => `No one else has joined ${workspace} yet.`,
  done: 'Done',
} as const;

export const makePrivateCopy = {
  title: (workspace: string) => `Make your ${workspace} connection private`,
  submit: 'Make private',
} as const;

export const inviteCopy = {
  title: (workspace: string) => `Invite people to ${workspace}`,
  username: 'Username',
  usernamePlaceholder: 'bob',
  submit: 'Invite',
  invited: 'invited',
  sendInvitation: (first: string) => `Send ${first} this invitation:`,
  invitationLabel: 'invitation message',
  installed: (username: string, server: string) =>
    `Is Crew installed for @${username} on ${server}?`,
  installLead: (username: string, server: string) =>
    `Crew runs from ~/.local/bin/biorouter-crew in each person’s own account. @${username} runs this on ${server}, or asks your IT team to:`,
  installCommands: [
    'mkdir -p "$HOME/.local/bin"',
    'install -m 0755 /path/to/biorouter-crew "$HOME/.local/bin/biorouter-crew"',
    '"$HOME/.local/bin/biorouter-crew" --version',
  ].join('\n'),
  installCommandsLabel: 'install commands',
  nextStep: (first: string) =>
    `When ${first} sends you a code, choose Let in… next to their name in the sidebar.`,
  done: 'Done',
  cancel: 'Cancel',
  retry: 'Retry',
  invitationUnavailable: 'The invitation message couldn’t be loaded.',
  addDevice: (username: string) => `Add another device for @${username}`,
  refusal: {
    noAccount: (text: string) => `No account named @${text} on this server.`,
    alreadyMember: (username: string, workspace: string) =>
      `@${username} is already in ${workspace}.`,
    canonical: (canonical: string) =>
      `Invite @${canonical} instead: that’s the account’s exact name.`,
  },
  legacy: {
    toggle: 'Invite someone using an older version of Biorouter',
    joinRequest: 'Their join request',
    joinRequestPlaceholder: 'Paste the join request they sent you',
    joinRequestInvalid: 'This join request has no device key. Ask them to copy it again.',
    userId: (server: string) => `Their user ID on ${server}`,
    userIdHelp: 'Run on the server:',
    userIdCommandLabel: 'user ID command',
    userIdCommand: (username: string) => `id -u ${username}`,
    submit: 'Create invitation',
    tokenLabel: 'invitation token',
    sendToken: (first: string) => `Send this to ${first}. It works once and expires in an hour.`,
    someone: 'them',
  },
} as const;

export const letInCopy = {
  title: (workspace: string) => `into ${workspace}`,
  titlePrefix: 'Let',
  code: (first: string) => `Code from ${first}`,
  helper: (first: string) => `Paste the code ${first} sends you directly.`,
  submit: (first: string) => `Let ${first} in`,
  /** `already_approved`: a device was let in already. Same words as the CLI (`enroll approve`). */
  alreadyApproved: (username: string) => `You already let a device in for @${username}.`,
  replaceHelp: 'Replace the code only if they sent you a new one.',
  replace: 'Replace code',
  approved: (first: string) => `Approved. ${first} joins as soon as their Crew checks in.`,
  addToTeam: (first: string, team: string) => `Add ${first} to ${team}`,
  addedToTeam: (first: string, team: string) => `Invited ${first} to ${team}`,
  addAfterJoin: (first: string) => `You can add ${first} to a team once they’ve joined.`,
  done: 'Done',
  cancel: 'Cancel',
} as const;

/** `DeviceCodeInput`'s refusals, worded as the shared library words them (`DeviceCodeError`). */
export const deviceCodeCopy = {
  wrongLength: 'A device code has 16 letters and numbers.',
  containsU: 'Device codes never contain the letter U. Check the code.',
  invalidCharacter: 'A device code has only letters and numbers.',
  /** Generic on purpose: the host's screen shows no code it was not given. */
  placeholder: 'XXXX-XXXX-XXXX-XXXX',
} as const;

export const createTeamCopy = {
  title: 'Create team',
  name: 'Name',
  placeholder: 'e.g. Analysis Lab',
  helper: (workspace: string) => `Team names are unique in ${workspace}.`,
  submit: 'Create team',
  addTitle: (team: string) => `Add people to ${team}`,
  skip: 'Skip for now',
  add: 'Add',
  cancel: 'Cancel',
  reservedCharacter: 'Team name can’t contain @, #, / or :.',
} as const;

export const createChannelCopy = {
  title: 'Create channel',
  inTeam: (team: string) => `in ${team}`,
  name: 'Name',
  placeholder: 'e.g. methods',
  preview: (slug: string) => `Will be created as #${slug}`,
  content: 'Content',
  restricted: 'Restricted',
  restrictedDetail: 'For unpublished or sensitive work',
  publicSafe: 'Public-safe',
  publicSafeDetail: 'Public models may read it',
  submit: 'Create channel',
  cancel: 'Cancel',
} as const;

/** The S2 name refusal and consequence line (naming design D5), byte for byte. */
export const nameRuleCopy = {
  teamTaken:
    'A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.',
  channelTaken:
    'A channel with this name, or one that looks like it, already exists in this team. Choose a different name.',
  consequence:
    'Everyone in this team can tell whether a name is taken. Keep identifiers out of names.',
  channelEmpty: 'Channel name can’t be empty.',
  channelTooLong: 'Channel name is too long. Choose a shorter name.',
  channelReserved: 'Channel name can’t contain @, #, / or :.',
  channelDisallowed:
    'Channel name can use lowercase letters, numbers, hyphens and underscores only.',
  channelStart: 'Channel name must start with a letter or number.',
  channelLooksLikeId: 'Channel name can’t look like an ID.',
  workspacePattern:
    'Workspace name can use lowercase letters a-z, numbers and hyphens, and must start and end with a letter or number.',
} as const;

export const addPeopleCopy = {
  titleChannel: (channel: string) => `Add people to ${channel}`,
  titleTeam: (team: string) => `Add people to ${team}`,
  person: 'Person',
  search: 'Search by name or @username',
  choose: 'Choose a person',
  noMatch: (query: string) => `No one matches “${query}”.`,
  allInTeam: (team: string) => `Everyone in ${team} is already here.`,
  allInWorkspace: (workspace: string) => `Everyone in ${workspace} is already here.`,
  noOne: (workspace: string) => `No one else has joined ${workspace} yet.`,
  submit: 'Add',
  cancel: 'Cancel',
  sent: (person: string) => `Invitation sent to ${person}`,
} as const;

export const transferCopy = {
  title: (channel: string) => `Transfer ownership of ${channel}`,
  owner: 'New owner',
  helper: 'They’ll need to accept.',
  submit: 'Offer ownership',
  cancel: 'Cancel',
  noOne: 'No one else in this channel can take it over yet.',
  offered: (person: string) => `Ownership offered to ${person}`,
} as const;

export const renameCopy = {
  titleTeam: 'Rename team',
  titleChannel: 'Rename channel',
  titleWorkspace: 'Rename workspace',
  name: 'Name',
  submit: 'Rename',
  cancel: 'Cancel',
} as const;

export const profileCopy = {
  title: 'Edit profile',
  displayName: 'Display name',
  initials: 'Initials (optional)',
  usernameLead: 'Your username:',
  suggestion: (name: string) => `Use “${name}”`,
  suggestionDetail: 'the name on your server account',
  submit: 'Save profile',
  cancel: 'Cancel',
  handleMark: 'Display name can’t contain @ or #.',
} as const;

export const keysCopy = {
  title: 'Keys and security',
  checking: 'Checking where your keys are stored…',
  keychain: 'Stored in your system keychain.',
  vault: 'Stored in an encrypted vault',
  locked: 'Locked',
  unlocked: 'Unlocked',
  unlock: 'Unlock',
  lock: 'Lock',
  deviceKey: 'This device’s key',
  deviceKeyLabel: 'this device’s key',
  devices: 'Devices on your account',
  deviceAdded: (date: string) => `Added ${date}`,
  addedVia: {
    bootstrap: 'when the workspace was created',
    token: 'with an invitation token',
    invitation_code: 'with an invitation',
  } as Record<string, string>,
  vaultToggle: 'Use an encrypted vault instead',
  vaultNote: 'Only for a new Crew profile. Existing identities aren’t moved.',
  setUpVault: 'Set up vault…',
  failed: 'Crew couldn’t change how your keys are stored.',
  done: 'Done',
} as const;

export const sharePathCopy = {
  title: (host: string) => `Share a path on ${host}`,
  path: 'Path',
  placeholder: '/home/you/project/data.h5ad',
  pattern: 'Use an absolute path that starts with /.',
  label: 'Label (optional)',
  labelSummary: 'Label: the file name',
  advanced: 'Advanced',
  submit: 'Add to message',
  cancel: 'Cancel',
} as const;

export const confirmCopy = {
  cancel: 'Cancel',
  typeToConfirm: (phrase: string) => `Type ${phrase} to confirm`,
  makeConnectionPublic: {
    title: (workspace: string) => `Make your ${workspace} connection public?`,
    description:
      'Public models will be able to read public-safe work you can see here. Restricted content stays private. Your unsent draft will be cleared.',
    confirm: 'Make public',
  },
  allowWorkspacePublic: {
    title: (workspace: string) => `Allow Public in ${workspace}?`,
    description:
      'Members will be able to choose Public. Agents with access will need permission again.',
    confirm: 'Allow Public',
  },
  makeWorkspacePrivate: {
    title: (workspace: string) => `Make ${workspace} Private for everyone?`,
    description: 'Agents with access will need permission again.',
    confirm: 'Make Private for everyone',
    toast: (workspace: string) =>
      `${workspace} is now Private for everyone. Agents with access need permission again.`,
  },
  setInstitution: {
    title: (workspace: string, id: string) => `Set ${workspace}’s institution to ${id}?`,
    description: (id: string) =>
      `This can’t be changed later. Private data can then be used only with models approved for ${id}.`,
    confirm: (id: string) => `Set ${id} permanently`,
  },
  removePerson: {
    title: (person: string, workspace: string) => `Remove ${person} from ${workspace}?`,
    description: 'Removes all of their devices and agent access. Their messages stay in history.',
    confirm: (workspace: string) => `Remove from ${workspace}`,
    /** The case-sensitive check on top of the primitive's case-folded one. */
    mismatch: 'Type the exact username to remove access.',
  },
  archiveChannel: {
    title: (channel: string) => `Archive ${channel} for everyone?`,
    description: 'Nobody can post in it after this. Its history stays readable.',
    confirm: 'Archive channel',
  },
  removeChannelMember: {
    title: (person: string, channel: string) => `Remove ${person} from ${channel}?`,
    description: 'They’ll lose access to its messages and files. You can invite them again.',
    confirm: 'Remove',
  },
  removeConnection: {
    title: (workspace: string) => `Remove ${workspace} from this computer?`,
    description:
      'Chats connected to it lose access. Your messages stay on the server, and you can add it again.',
    confirm: 'Remove',
  },
  stopTask: {
    title: 'Stop your agent?',
    description: 'It stops working on this task. Anything it already did stays done.',
    confirm: 'Stop task',
    cancel: 'Keep running',
  },
} as const;

/**
 * Broker refusals whose own text is written for a program rather than a person, in words for a
 * person (`refusals.ts`). The CLI prints the same sentences (`commands/crew/output.rs`,
 * `broker_refusal_text`), so they avoid apostrophes and stay byte-identical across the two.
 */
export const refusalCopy = {
  /** `identity_conflict: another active member is @x; remove the old @x first`. */
  identityConflict: (username: string | null) =>
    username
      ? `Another active member is already @${username}. Remove the old @${username} first.`
      : 'Another active member already has this username. Remove the old member first.',
  /** `device_conflict: this device key is already enrolled in this workspace; …`. */
  deviceConflict:
    'This device key is already enrolled in this workspace. Join with a new device key.',
  /** `identity_mismatch: enrollment principal changed; …` and `UID account name changed; …`. */
  identityMismatch:
    'This server account no longer matches the member it joined as. Remove the old member first, then invite them again.',
  /** `quota_exceeded: journal exceeds …` and `quota_exceeded: workspace logical state exceeds …`. */
  storageFull:
    'This workspace has grown past the size Crew supports and cannot take more changes. Ask the host about starting a new workspace.',
  /** `rate_limited: too many live challenges`. */
  tooManyAttempts: 'Too many attempts at once. Wait a minute, then try again.',
} as const;

export const dialogErrorCopy = {
  /** "Privacy not yet verified" from the copy deck's error strings. */
  privacyUnverified: 'Crew is still checking this workspace’s privacy. Try again in a moment.',
  /** The one fallback a dialog shows when the daemon gave no words. */
  fallback: 'Crew couldn’t complete that action.',
} as const;
