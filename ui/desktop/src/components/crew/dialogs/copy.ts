import type { LocalPlatform } from './localPath';

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
  /**
   * Under the login, when the person's own SSH settings name its server (D-ALIAS): the field keeps
   * the real target, and this says why everywhere else calls it by the alias (QA Q3-39).
   */
  loginAlias: (label: string) => `Your SSH settings call this server ${label}.`,
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
  port: 'Port',
  identityFile: 'Identity file',
  /**
   * A full path in this computer's own shape (W2-UIW-13). It used to be `~/.ssh/id_ed25519`, the
   * one shape the daemon refuses on every OS.
   */
  identityFilePlaceholder: (platform: LocalPlatform) =>
    platform === 'windows'
      ? 'C:\\Users\\you\\.ssh\\id_ed25519'
      : platform === 'mac'
        ? '/Users/you/.ssh/id_ed25519'
        : '/home/you/.ssh/id_ed25519',
  jumpHosts: 'Jump hosts',
  jumpHostsPlaceholder: 'gateway.example.edu',
  remoteRoot: 'Remote work folder',
  remoteRootPlaceholder: '/home/you/project',
  remoteRootPattern: 'Use an absolute path that starts with /.',
  /**
   * Under Identity file, typed or refused by the daemon ("Identity file must be an absolute path"):
   * a path starting with `~` is refused too, since the daemon never expands it (DW-04). Worded for
   * this computer's OS, which is the daemon's (W2-UIW-13): a Windows path starts with a drive
   * letter, not with /. Without a known OS it names neither.
   */
  identityFileAbsolute: (platform: LocalPlatform) =>
    platform === 'windows'
      ? 'Use the key file’s full path, such as C:\\Users\\you\\.ssh\\id_ed25519. A path starting with ~ isn’t accepted.'
      : platform === 'unknown'
        ? 'Use the key file’s full path. A path starting with ~ isn’t accepted.'
        : 'Use the key file’s full path, starting with /. A path starting with ~ isn’t accepted.',
  /** The daemon's other field refusals on a save, each under its own field (DW-04). */
  nameLength: 'Use a connection name of 1 to 120 characters.',
  loginInvalid:
    'Use a server login or SSH alias: letters, numbers and _ . / : @ % -, not starting with a dash.',
  jumpHostsInvalid:
    'Use SSH aliases or user@host names for jump hosts, separated by commas, without spaces or shell characters.',
  /**
   * Under Institution when the daemon refuses it (SF-F4): a connection's institution must be the
   * workspace's, which never changes once set. The daemon's own words are kept when it wrote them
   * for a person (`crew_institution_mismatch`); an older daemon's "Crew aliases have different
   * institutions…" is said as this.
   */
  institutionMismatch: (typed: string, workspace: string, institution: string) =>
    `This connection is for ${typed}, but ${workspace} belongs to ${institution}. Use ${institution} here.`,
  institutionMismatchUnknown: (workspace: string) =>
    `This connection’s institution has to be the one ${workspace} belongs to, which can’t be changed.`,
  /**
   * What the connection's agents may do on the server (OW-6), the person's own choice for their
   * own account: nothing, today's confined work folder, or commands as them. `server` is the
   * server as the person names it.
   */
  access: (server: string) => `What can your agent do on ${server || 'the server'}?`,
  accessOff: 'Off',
  accessOffDetail: (server: string) =>
    `It can’t use files or run commands on ${server || 'the server'}.`,
  accessFolder: 'In a folder',
  accessFolderDetail: 'It reads and writes files in one folder, and runs confined commands there.',
  accessFull: 'As me on the server',
  /** `user` is the server account, `@crew_alice`, or null. */
  accessFullDetail: (user: string | null) =>
    `It runs commands as ${user ?? 'you'}, with your files, network and programs.`,
  /** Why As me on the server can't be chosen: the broker or this account's bridge predates it. */
  accessFullUnavailable: (server: string) => `Update Crew on ${server || 'the server'} first.`,
  /**
   * Under In a folder (HPC-N1): a command there runs confined, with no network, no other
   * processes and no user lookups, so an agent asked to submit a cluster job found out only by
   * trying. Quoted whole by the manual (`check-crew-manual.mjs`, app-sentences).
   */
  remoteExecutionHelp:
    'Commands run with no network and can’t start other programs, so cluster tools such as sbatch won’t run there.',
  /** In a folder on a kernel that cannot confine a command (no Landlock ABI 3): files only. */
  confinedUnavailable: (server: string) =>
    `${server || 'This server'} can’t confine commands, so in a folder your agent only reads and writes files. To run commands there, choose As me on the server.`,
  /** Under the folder while As me on the server is chosen: it is where commands start. */
  fullFolderHelp: 'Commands start in this folder and can reach anything your account can.',
  /**
   * The one warning before full access (privacy-policy constraint 4.1): said in place, and Save is
   * the person's consent. `user` is the account (`@crew_alice`), `server` the server's name.
   */
  fullWarning: (user: string, server: string) =>
    `Your agent will run commands as ${user} on ${server || 'the server'}: it can read and change anything your account can, use the network, and start programs that keep running after access ends. Other people’s messages and files in channels it reads can try to steer it. Use a private model you trust.`,
  /** The host's extra sentence: the broker's own files live in this account. */
  fullWarningHost: (workspace: string) =>
    `You host ${workspace} in this account, so it can also read everything ${workspace} keeps on the server, including channels you aren’t in.`,
  portRange: 'Use a port from 1 to 65535.',
  /** Its own disclosure, closed until opened, whatever Advanced holds (QA T-33). */
  workspaceDetails: 'Workspace details',
  workspaceDetailsSummary: 'IDs for support',
  workspaceId: 'Workspace ID',
  fingerprint: 'Fingerprint',
  workspaceKey: 'Workspace key',
  socketPath: 'Socket path',
  /** Kept for callers; the numeric UID is never rendered (spec identity rule 6). */
  hostUserId: 'Host user ID',
  deviceId: 'Device ID',
  clusterId: 'Cluster ID',
  /** The button that copies a machine value the dialog does not show (spec rule 13). */
  copyValue: (what: string) => `Copy ${what}`,
  copied: 'Copied',
  copyFailed: 'Copy failed',
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
  /** The Copy button beside the server's address: "Copy server address" (QA Q3-39). */
  serverAddress: 'server address',
  /**
   * How full the workspace is, for its host only (M1, W2-UIW-20): of the space ordinary changes
   * may use, from the fuller of its state and its journal.
   */
  storage: 'Storage',
  storageUsed: (percent: number) => `${percent}% full`,
  storageFull: 'Full',
  rename: 'Rename…',
  /** The tab list's accessible name (QA T-39). */
  tabsLabel: 'Workspace settings sections',
  waiting: 'Waiting to join',
  members: 'Members',
  invite: 'Invite people…',
  /**
   * Where the host sees Invite people…, what a member sees instead (UXN-5): the menus had no entry
   * and no word on who invites. `host` is named in full; null when the view does not know the host.
   */
  onlyHostInvites: (host: string | null) =>
    host
      ? `Only the host, ${host}, can invite new people.`
      : 'Only the host can invite new people.',
  host: 'Host',
  /** The Host badge's tooltip: what the role is (QA Q2-69). */
  hostTooltip: (workspace: string) => `Workspace host: runs ${workspace} on the server`,
  you: 'you',
  letIn: 'Let in…',
  letInLabel: (username: string) => `Let @${username} in`,
  /** The first sentence of `letInCopy.mismatch`; the Let in dialog says what to do about it. */
  otherDevice: (username: string) =>
    `A computer trying to join as @${username} showed a different code.`,
  cancelInvitation: 'Cancel invitation',
  cancelInvitationLabel: (username: string) => `Cancel @${username}’s invitation`,
  cancelInvitationConfirm: (username: string) => `Cancel @${username}’s invitation?`,
  keepInvitation: 'Keep',
  memberOptions: (name: string) => `${name} options`,
  copyUsername: 'Copy username',
  /**
   * The one submenu that holds a menu's machine-ID copies, last, after a separator (QA Q3-26):
   * everyday menus keep only the copies a person uses.
   */
  copyForSupport: 'Copy for support',
  copyPersonId: 'Copy person ID',
  removeFrom: (workspace: string) => `Remove from ${workspace}…`,
  yourConnection: 'Your connection',
  workspace: 'Workspace',
  privateForEveryone: 'Private for everyone',
  allowsPublic: 'Allows Public',
  allowPublic: 'Allow Public…',
  makePrivateForEveryone: 'Make Private for everyone…',
  /** The privacy popover's words for the same action (QA Q2-29): only this person's connection. */
  makePublic: 'Make my connection public…',
  makePrivate: 'Make private',
  institution: 'Institution',
  notSet: 'Not set',
  setInstitution: (id: string) => `Set institution to ${id}…`,
  institutionNeedsConnection: 'Add your institution in Connection settings first.',
  /** Under Privacy, for a member: names what "this" was (QA Q4-39). */
  hostOnly: (workspace: string) => `Only the host can change ${workspace}’s privacy.`,
  noMembers: (workspace: string) => `No one else has joined ${workspace} yet.`,
  /**
   * While the view is observed again with no verified copy to draw (SF2-N7): the value of a row
   * the snapshot decides (who hosts, the workspace's privacy and institution), which is not
   * guessed. Without a snapshot nobody reads as the host, so a guess would give the host a
   * member's words.
   */
  checking: 'Checking…',
  /** People's whole tab in the same state: who is in, and who may invite, come from the view. */
  checkingPeople: (workspace: string) => `Checking who is in ${workspace}…`,
  done: 'Done',
  /**
   * Who manages teams and channels (OW-1, OW-2): the host's choice, applied at once. Either way
   * only the host removes people, changes privacy or deletes the workspace.
   */
  governance: {
    label: 'Who can manage teams and channels',
    open: 'Everyone can manage',
    openDetail:
      'Every member can rename, archive and delete teams and channels, add people to them and join any of them.',
    managed: 'Owners manage',
    managedDetail: 'A team’s creator and a channel’s owner change them, and nobody else.',
    hostOnly: (workspace: string) =>
      `Either way, only you remove people from ${workspace}, change its privacy or delete it.`,
    /** The same, read-only, for a member: who manages, in a word. */
    row: 'Who manages',
    everyone: 'Everyone',
    owners: 'Owners',
  },
  /** General's last section: removing the workspace here, and deleting it for everyone. */
  dangerZone: 'Danger zone',
  removeFromComputer: 'Remove from this computer…',
  deleteWorkspace: 'Delete workspace…',
  /** Why the host's Delete workspace… is disabled: the server's Crew predates deletion. */
  deleteUnsupported: (server: string) =>
    `Update Crew on ${server || 'the server'} to delete this workspace.`,
} as const;

/**
 * The Context tab (OW-8): one Markdown document about the workspace and its server that every
 * member's agent receives as reference information. Anyone edits it in an open workspace, only the
 * host in a managed one; every save names the revision it replaces.
 */
export const contextCopy = {
  tab: 'Context',
  label: 'Context for agents',
  /** Under the editor: what it is for, and what it never does. */
  help: (workspace: string) =>
    `Every member’s agent reads this about ${workspace} and its server: where data lives, how to run jobs. It’s reference only and never tells an agent to do anything.`,
  placeholder: 'For example: where the shared data lives, and how jobs run on this server.',
  empty: 'No context yet.',
  /** `who` is `personLabel(…, 'inline')`; `when` is a date and time. */
  saved: (revision: number, who: string | null, when: string | null) =>
    [`Revision ${revision}`, who ? `saved by ${who}` : null, when]
      .filter((part): part is string => Boolean(part))
      .join(' · '),
  save: 'Save context',
  saving: 'Saving…',
  savedNotice: 'Context saved.',
  discard: 'Discard changes',
  draft: 'Draft from server',
  drafting: 'Drafting…',
  /** After a draft: it is in the editor, unsaved. */
  drafted: (server: string) =>
    `Drafted from what ${server || 'the server'} reports. Review it, then save it.`,
  /**
   * `revision_conflict`: someone saved while this person edited. Their own words stay in the
   * editor; saving again replaces the newer version on purpose, or they take it instead.
   */
  conflict: (who: string | null) =>
    `${who ?? 'Someone'} saved a newer version while you were editing. Your text is still here: save again to replace theirs, or use their version.`,
  useTheirs: 'Use their version',
  tooLarge: 'The context can be at most 64 KB. Shorten it, then save.',
  /** For whoever may not edit it: a managed workspace's members. */
  hostOnly: (host: string | null) =>
    host
      ? `Only the host, ${host}, can change this context.`
      : 'Only the host can change this context.',
} as const;

/** Browse channels (OW-2): an open workspace's teams and channels the person is not in yet. */
export const browseCopy = {
  title: 'Browse channels',
  titleTeam: (team: string) => `Browse channels in ${team}`,
  teams: 'Teams',
  channels: 'Channels',
  /** `team` is the team as typed; channels are `#name`. */
  channelIn: (team: string) => `in ${team}`,
  members: (count: number) => `${count} ${count === 1 ? 'member' : 'members'}`,
  restricted: 'Restricted',
  join: 'Join',
  joinTeamLabel: (team: string) => `Join ${team}`,
  joinChannelLabel: (channel: string) => `Join ${channel}`,
  /** Joining a team brings its #general (`team.join`). */
  teamDetail: 'Joining adds you to its #general.',
  nothing: 'You’re in every team and channel you can join.',
  done: 'Done',
} as const;

export const makePrivateCopy = {
  title: (workspace: string) => `Make your ${workspace} connection private`,
  submit: 'Make private',
} as const;

export const inviteCopy = {
  title: (workspace: string) => `Invite people to ${workspace}`,
  username: 'Username',
  /** Used only where no server is known; `loginPlaceholder` otherwise. */
  usernamePlaceholder: 'their server login',
  /** The field wants the account name on the server, not a display name (QA T-23). */
  loginPlaceholder: (server: string) =>
    server ? `their login on ${server}` : 'their server login',
  /** Under the field, and again beside a refusal: what a username is here, with the host's own. */
  loginHelper: (server: string, me: string | null) =>
    `The name they sign in to ${server || 'the server'} with${me ? `; yours is @${me}` : ''}.`,
  submit: 'Invite',
  invited: 'invited',
  sendInvitation: (first: string) => `Send ${first} this invitation:`,
  /**
   * Under the invitation: the invitation carries no host key, so the joiner checks the server's
   * SSH fingerprint against one the host (or IT) gives them the first time they connect
   * (`docs/crew/hosting-a-workspace.md`, step 4; F7). Not the workspace fingerprint in the menu.
   */
  serverFingerprint: (first: string) =>
    `Also tell ${first} the server’s SSH fingerprint, or where to get it from IT, so they can check it the first time they connect.`,
  invitationLabel: 'invitation message',
  /**
   * Under the invitation (OW-9): the joiner's own Biorouter installs Crew in their account when it
   * isn't there, so there is nothing to install for them first.
   */
  setsUp: (first: string) => `${first}’s Biorouter sets Crew up in their account.`,
  /**
   * Collapsed under the result: what to do if the joiner's Crew still says it isn't set up — in the
   * words the joiner's screen uses (`notSetUpCopy.title`), not a question only IT can answer (QA
   * Q4-37).
   */
  installed: (first: string) => `If ${first} sees “Crew isn’t set up”`,
  /** One sentence, then the commands: who to send them to, and whose account they install into. */
  installLead: (username: string, server: string) =>
    `Send this to whoever runs ${server || 'the server'}, to run in @${username}’s account:`,
  /** Runnable as written: copies the server-wide install, with no placeholder path (QA T-44). */
  installCommands: [
    'mkdir -p "$HOME/.local/bin"',
    'install -m 0755 "$(command -v biorouter-crew)" "$HOME/.local/bin/biorouter-crew"',
    '"$HOME/.local/bin/biorouter-crew" --version',
  ].join('\n'),
  installCommandsLabel: 'install commands',
  nextStep: (first: string) =>
    `When ${first} sends you a code, choose Let in… next to their name in the sidebar.`,
  /** After `nextStep`: when the invitation runs out (QA Q4-36). `phrase` is `expiryPhrase`'s. */
  expires: (phrase: string) => `This invitation ${phrase}.`,
  done: 'Done',
  /** After an invitation, start again with an empty field (QA T-44). */
  inviteAnother: 'Invite another',
  cancel: 'Cancel',
  retry: 'Retry',
  invitationUnavailable: 'The invitation message couldn’t be loaded.',
  addDevice: (username: string) => `Add another device for @${username}`,
  addDeviceUnnamed: 'Add another device for this person',
  refusal: {
    noAccount: (text: string) => `No account named @${text} on this server.`,
    /**
     * The broker's "already a member … Choose Add device" (F3): a member's new or re-added
     * computer. The switch is turned on with it, and the sentence says so.
     */
    alreadyMember: (username: string, workspace: string) =>
      `@${username} is already in ${workspace}. To add this computer of theirs, Add another device for @${username} is now on: choose Invite again.`,
    canonical: (canonical: string) =>
      `Invite @${canonical} instead: that’s the account’s exact name.`,
  },
  legacy: {
    /** Nothing the host can know about the joiner's version, asked as a question (QA Q4-37). */
    toggle: 'Other ways to invite (older Biorouter)',
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

/**
 * An invitation's expiry, the one wording the Invite result, Let in and the sidebar share (QA
 * Q4-36): "expires Sat 1:41 AM", or "expired" once past.
 */
export const expiryCopy = {
  expires: (when: string) => `expires ${when}`,
  expired: 'expired',
} as const;

export const letInCopy = {
  /** "Let {name} into {workspace}", after `titlePrefix` and the joiner's one name (QA Q4-38). */
  title: (workspace: string) => `into ${workspace}`,
  titlePrefix: 'Let',
  code: (first: string) => `Code from ${first}`,
  /**
   * Under the code field: a security hint, not "just paste it" (QA Q3-36). The code is only worth
   * approving if it came from the person themselves.
   */
  helper: (first: string) =>
    `Paste the code ${first} sent you. Only use a code that came from ${first}.`,
  submit: (first: string) => `Let ${first} in`,
  /**
   * Before the field, when a computer with a different code has tried to join as this person. The
   * sidebar (`sidebar/copy.ts`) and the CLI (`enroll pending`) say the same (QA T-13).
   */
  mismatch: (username: string) =>
    `A computer trying to join as @${username} showed a different code. Check the code @${username} sent you, then enter it and choose Replace code. Don’t approve a code you didn’t get from @${username}.`,
  /** Back from the saved-code view to the field, when a mismatch shows the code was wrong. */
  enterAgain: 'Enter the code again',
  /**
   * `already_approved`: a code is saved for them already. Never "let a device in", which was false
   * after a mismatch (QA Q2-23); names the control that exists, Replace code. `who` is the one name
   * Let in uses for the joiner, never "they" (QA Q3-36); without it, `@username`.
   */
  alreadyApproved: (username: string, who: string = `@${username}`) =>
    `You already entered a code for ${who}, and it didn’t match ${who}’s computer. Enter the code ${who} sent you and choose Replace code.`,
  replaceHelp: (who: string) => `Replace the code only if ${who} sent you a new one.`,
  replace: 'Replace code',
  /**
   * The broker only records the code: it cannot tell yet whether it is the right one, so this
   * never says "Approved" (QA T-13), and says the host need not wait (QA Q2-23). `who` is the one
   * name the dialog uses for the joiner, never "they" (QA Q3-36).
   */
  approved: (who: string) =>
    `Code saved. ${who} is in as soon as ${who}’s Crew checks in; you can close this.`,
  /** Replaces `approved` once the directory shows the person as a member. */
  joined: (who: string, workspace: string) => `${who} joined ${workspace}`,
  /**
   * A member adding another computer (SC2-N6): the title's name is followed by this, "Let Henry
   * Ito’s new computer into chen-lab", since the person is in the workspace already.
   */
  deviceOf: '’s new computer',
  deviceSubmit: (who: string) => `Let ${who}’s new computer in`,
  /** `approved`, for another computer of a member. */
  deviceApproved: (who: string) =>
    `Code saved. ${who}’s new computer is in as soon as it checks in; you can close this.`,
  /** Replaces `deviceApproved` once the computer checked in and its waiting row went. */
  deviceJoined: (who: string, workspace: string) => `${who}’s new computer is in ${workspace}`,
  /** An older broker: the team is an invitation the person accepts in Crew. */
  addToTeam: (who: string, team: string) => `Invite ${who} to ${team}`,
  /**
   * The invitation was sent. `who` is the person in the one rule for inline text, "Eve Park (@eve)",
   * as every Added sentence names people (M11); `first` is the dialog's one name for them after
   * that, never "they" (QA Q3-36).
   */
  addedToTeam: (who: string, first: string) =>
    `Invited ${who}. ${first} will see it in Crew and needs to accept.`,

  /** A broker that adds members directly (`direct_add_v1`). */
  directAddToTeam: (who: string, team: string) => `Add ${who} to ${team}`,
  /** The footer's primary action when there is one team to add them to (QA Q2-03). */
  footerAdd: (team: string) => `Add to ${team}`,
  /**
   * A direct team addition landed: `channels` is `#general and #methods` (QA Q2-23, Q3-36). `who`
   * and `first` as in {@link letInCopy.addedToTeam}.
   */
  directAdded: (who: string, team: string, channels: string, first: string) =>
    `Added ${who} to ${team}. ${first} can now see ${channels}.`,
  /** The same, for an older broker that invites. */
  footerInvite: (team: string) => `Invite to ${team}`,
  /**
   * A team's channel choices, whole on screen (QA Q4-38): "Also add to" was half a sentence whose
   * team was named only on the button. The group is named by this visible heading, so what a
   * screen reader announces and what a voice-control user says are the words on screen.
   */
  channelsIn: (team: string) => `Channels in ${team}`,
  /** Under the fingerprint while the code is awaited (QA Q4-36). `phrase` is `expiryPhrase`'s. */
  expires: (first: string, phrase: string) => `${first}’s invitation ${phrase}.`,
  /**
   * Under the team offers (M19), as Add people says it: a member reads a channel's whole past, files
   * included. Never "they" (QA Q3-36).
   */
  history: (first: string) =>
    `Once added, ${first} can read everything already posted in the channels ${first} is added to, including files.`,
  /** Closes with the team additions still undone. */
  notNow: 'Not now',
  /** Under the team offers while the joiner's Crew has not checked in yet (QA Q3-36). */
  addAfterJoin: (first: string) => `You can add ${first} to a team once ${first} joins.`,
  /**
   * The same row once they have joined. The row keeps the room of the longer of this and
   * `addAfterJoin` in both views, so the footer stays under the host's pointer (QA Q3-35).
   */
  channelsWithTeam: 'Ticked channels are added with the team.',
  /** The workspace key's fingerprint, for the host to read out if asked (QA Q2-04). */
  fingerprintFor: (who: string) => `Fingerprint ${who} should see:`,
  fingerprintHelper: (who: string) =>
    `If ${who} asks, read this out. It should match what ${who}’s Crew shows.`,
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
  /**
   * Never the name of a team that exists (QA Q3-38): "e.g. Analysis Lab" sat beside the lab's own
   * Analysis Lab. `placeholderTaken` when this one does.
   */
  placeholder: 'e.g. Imaging Group',
  placeholderTaken: 'e.g. new-team',
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
  /** Never the name of a channel that exists (QA Q2-31): `placeholderTaken` when this one does. */
  placeholder: 'e.g. journal-club',
  placeholderTaken: 'e.g. new-channel',
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
  /**
   * Under a channel name: what the refusal of a taken name tells people, and so what never to put
   * in one — said in a lab's words, not "identifiers" (QA Q3-38).
   */
  consequence:
    'Everyone in this team can see whether a name is taken, so don’t put patient or sample IDs in channel names.',
  /**
   * The same, under a team name (F9). Team names are unique across the workspace and a taken one is
   * refused for teams the person is not in, so it tells more than a channel name does.
   */
  teamConsequence:
    'Everyone in this workspace can see whether a team name is taken, so don’t put patient or sample IDs in team names.',
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
  /** The same dialog for someone who may not add people to the team: its member list (QA Q3-44). */
  membersOf: (team: string) => `Members of ${team}`,
  /**
   * Above a list of who is in the team or channel already (UXN-6): how many people it holds, as
   * the Members tab's header says, since nine rows fit and the tenth was out of sight.
   */
  memberCount: (count: number) => `${count} ${count === 1 ? 'member' : 'members'}`,
  /** A team's creator, marked as the Members tab marks a channel's owner. */
  teamOwner: 'Team owner',
  person: 'Person',
  search: 'Search by name or @username',
  choose: 'Choose a person',
  noMatch: (query: string) => `No one matches “${query}”.`,
  allInTeam: (team: string) => `Everyone in ${team} is already here.`,
  allInWorkspace: (workspace: string) => `Everyone in ${workspace} is already here.`,
  noOne: (workspace: string) => `No one else has joined ${workspace} yet.`,
  /** A channel whose team has no one else in it yet. */
  noOneInTeam: (team: string) => `No one else is in ${team} yet.`,
  /** People invited to the team (or channel) who have not accepted: named, so nothing is hidden. */
  waitingToAccept: (names: string) =>
    `Invited, not accepted yet: ${names}. They’ll appear here once they accept.`,
  waiting: (names: string) => `Invited, not accepted yet: ${names}.`,
  allInvited: (workspace: string, team: string, names: string) =>
    `Everyone else in ${workspace} has been invited to ${team}. Waiting for: ${names}.`,
  /** The way out of an empty picker, for the host (QA T-22). */
  inviteToWorkspace: (workspace: string) => `Invite people to ${workspace}…`,
  /** The way out of an empty channel picker, for whoever can add people to the team. */
  addToTeam: (team: string) => `Add people to ${team}…`,
  /** The channel choices beside a direct team addition. */
  channels: 'Also add to',
  generalIncluded: 'comes with the team',
  submit: 'Add',
  cancel: 'Cancel',
  /** An older broker invites: say that it is waiting on the person, never that they are in. */
  sent: (person: string) => `Invited. ${person} will see it in Crew and needs to accept.`,
  alreadyIn: (person: string, place: string) => `${person} is already in ${place}.`,
  /**
   * Before Add (M19): the broker lets a member read a channel's whole past, files included, so an
   * addition shows them everything already posted, and a person removed and added again what was
   * posted meanwhile. `channel` is `#name`; for a team, `general` is its #general, with the
   * channels ticked beside it when there are any to tick.
   */
  historyChannel: (channel: string) =>
    `They’ll be able to read everything already posted in ${channel}, including files.`,
  historyTeam: (general: string, picked: boolean) =>
    picked
      ? `They’ll be able to read everything already posted in ${general} and the channels you pick, including files.`
      : `They’ll be able to read everything already posted in ${general}, including files.`,
  /** The checklist (QA Q2-05): everyone it shows, ticked at once. */
  people: 'People',
  selectAll: (count: number) => `Select all (${count})`,
  /** The submit with more than one person ticked; `submit` with one. */
  addMany: (count: number) => `Add ${count} people`,
  done: 'Done',
  /** One line for a whole run of additions, shown in the dialog, which stays open until Done. */
  addedToChannel: (people: string, channel: string) => `Added ${people} to ${channel}.`,
  addedToTeam: (people: string, team: string, channels: string) =>
    `Added ${people} to ${team}. They can now see ${channels}.`,
  invitedMany: (people: string) => `Invited ${people}. They’ll see it in Crew and need to accept.`,
  alreadyInMany: (people: string, place: string) => `${people} are already in ${place}.`,
  couldNotAdd: (people: string, reason: string) => `Couldn’t add ${people}: ${reason}`,
  /** Instead of an empty picker (QA Q2-22): who is there already, host and you first. */
  alreadyInPlace: (place: string) => `Already in ${place}`,
  /** The host's invitees who have not joined the workspace yet (QA Q2-22). */
  invitedNotJoined: (workspace: string, names: string) =>
    `Invited to ${workspace}, not joined yet: ${names}.`,
  /**
   * Someone who may not add people here, told who may (the broker's rule, QA Q2-22). `owner` is
   * named in full, "Alice Chen (@crew_alice)" (W2-UIW-7); when the owner is the host too, the
   * sentence names one person, never the owner "or the host" as if they were two (UXN-9).
   */
  onlyOwnerOrHost: (owner: string | null, place: string, ownerIsHost = false) =>
    owner
      ? ownerIsHost
        ? `Only ${owner}, the host, can add people to ${place}.`
        : `Only ${owner} or the host can add people to ${place}.`
      : `Only the owner of ${place} or the host can add people to it.`,
  /** The same under an older broker, where only the owner invites. */
  onlyOwner: (owner: string | null, place: string) =>
    owner
      ? `Only ${owner} can add people to ${place}.`
      : `Only the owner of ${place} can add people to it.`,
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
  /**
   * The slug a channel rename stores, once the name differs from the current one (M12). Never
   * Create channel's "Will be created as", which a rename read before anything was typed.
   */
  previewChannel: (slug: string) => `Will be renamed to #${slug}`,
  /**
   * A team's new command-line name (its handle, `team_handle` in the broker), once it changes: the
   * old one stops working in the CLI (M12).
   */
  previewTeamHandle: (handle: string) => `CLI name: ${handle}`,
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
  /**
   * Under Display name while it holds the server account's name and nothing is saved yet: it is a
   * suggestion, not a name already in use (QA Q3-43).
   */
  prefilled: (server: string) =>
    server
      ? `Filled in from your account on ${server}. Save to use it.`
      : 'Filled in from your server account. Save to use it.',
  submit: 'Save profile',
  cancel: 'Cancel',
  handleMark: 'Display name can’t contain @ or #.',
} as const;

export const keysCopy = {
  title: 'Keys and security',
  checking: 'Checking where your keys are stored…',
  /** The status could not be read, or had not answered after a few seconds (QA Q2-02). */
  statusFailed: 'Couldn’t check where your keys are stored.',
  /** Reads the status again. */
  retry: 'Retry',
  keychain: 'Stored in your system keychain.',
  /** A development profile keeps keys in plain files; never claim a keychain it is not using. */
  file: 'Stored in a file on this computer (development profile).',
  vault: 'Stored in an encrypted vault',
  locked: 'Locked',
  unlocked: 'Unlocked',
  unlock: 'Unlock',
  lock: 'Lock',
  /** This device's key, shown as its grouped fingerprint — never the 64-hex key (QA T-33). */
  deviceKey: 'This device',
  deviceKeyLabel: 'this device’s fingerprint',
  thisDevice: 'This device',
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
  /**
   * Says what it is for: a file already on the server, not one from this computer (QA Q2-32), and
   * names the server as the person does — their SSH alias, not its address (QA Q3-39).
   */
  title: (server: string) => `Share a file that’s already on ${server}`,
  path: 'Path',
  /** Under the path: when to use this rather than an upload. */
  helper:
    'For a file on the lab server, such as a large dataset. To share a file from this computer, use Upload a file.',
  /** The person's own home on the server, from the connection's login; never someone else's. */
  placeholder: (login: string | null) => (login ? `/home/${login}/…` : '/home/…'),
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
    // The way back is Add people, which on a broker that adds directly asks nobody to accept (M11).
    description:
      'They’ll lose access to its messages and files. You can add them again with Add people.',
    confirm: 'Remove',
  },
  /**
   * Removing deletes this computer's device key, and the member stays enrolled, so the old
   * invitation cannot bring the workspace back (`docs/crew/command-line.md`, "Change or remove a
   * saved connection"). A host who removes it from their last computer loses the host controls
   * for good (`docs/crew/hosting-a-workspace.md`, "Limits of the host role").
   */
  removeConnection: {
    title: (workspace: string) => `Remove ${workspace} from this computer?`,
    description:
      'It disconnects, ends every chat’s access through it and deletes this computer’s key for the workspace. Your messages stay on the server. To use the workspace here again, the host has to add this computer. If you host it and no other computer of yours still has it, nobody can let people in, change its privacy or remove anyone there again.',
    /**
     * A member's removal (F3): the old invitation cannot bring it back, and a plain invitation is
     * refused for a member, so it names the one that works. `username` is the person's, or null.
     */
    memberDescription: (workspace: string, username: string | null) =>
      `It disconnects, ends every chat’s access through it and deletes this computer’s key for the workspace. Your messages stay on the server. To use ${workspace} on this computer again, ask the host to invite you with Add another device for ${username ? `@${username}` : 'you'}.`,
    /**
     * The host removing it from the only computer of theirs the workspace lists (CLI-1): the host
     * controls end for good, so it asks for the workspace's name first, as the CLI asks for
     * `--give-up-host-controls`.
     */
    onlyHostDescription: (workspace: string, username: string | null) =>
      `You host ${workspace}, and no other computer of yours can act as its host. Removing it here ends the host controls for good: nobody could let people in, change its privacy or remove anyone again. To keep them, first invite another computer of yours with Add another device for ${username ? `@${username}` : 'yourself'}.`,
    onlyHostConfirm: 'Remove and give up hosting',
    confirm: 'Remove',
  },
  stopTask: {
    title: 'Stop your agent?',
    description: 'It stops working on this task. Anything it already did stays done.',
    confirm: 'Stop task',
    cancel: 'Keep running',
  },
  /**
   * `channel.delete` (OW-4). "Removed from the workspace", never "erased": the broker's journal
   * keeps the old records until the workspace itself is deleted (design, "Risks accepted").
   * `channel` is `#name`. The first sentence is pinned.
   */
  deleteChannel: {
    title: (channel: string) => `Delete ${channel}?`,
    description: (channel: string) =>
      `Removes ${channel}, its files and its messages from the workspace. Nobody can bring it back.`,
    confirm: 'Delete channel',
    toast: (channel: string) => `Deleted ${channel}.`,
  },
  /**
   * `team.delete` (OW-4): every channel in the team goes with it, including channels the person
   * deleting it is not in, which their own view does not list, so no count is promised here.
   */
  deleteTeam: {
    title: (team: string) => `Delete ${team}?`,
    description: (team: string) =>
      `Removes ${team} and every channel in it, including channels you aren’t in, with their files and messages, from the workspace. Nobody can bring them back.`,
    confirm: 'Delete team',
    /** `channels` is how many the broker says went with it, when it says. */
    toast: (team: string, channels: number | null) =>
      channels === null
        ? `Deleted ${team}.`
        : `Deleted ${team} and its ${channels} ${channels === 1 ? 'channel' : 'channels'}.`,
  },
  /**
   * `workspace.delete` (OW-5), the host's: Crew stops on the server and erases what the workspace
   * keeps there, and the daemon removes it from this computer. Typed, with the workspace's name.
   */
  deleteWorkspace: {
    title: (workspace: string) => `Delete ${workspace} for everyone?`,
    description: (workspace: string, server: string) =>
      `Crew stops on ${server || 'the server'} and deletes everything ${workspace} keeps there: its teams, channels, messages, files and members. It’s removed from this computer too, and members can only remove it from theirs. This can’t be undone.`,
    confirm: 'Delete workspace',
    toast: (workspace: string) => `Deleted ${workspace}.`,
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
  /**
   * `quota_exceeded: retained audit journal exceeds …` and `quota_exceeded: workspace logical state
   * exceeds …`, the startup-only `quota_exceeded: journal exceeds …`, and an older broker's
   * `quota_exceeded: workspace operation quota requires maintenance` (`STORAGE_FULL_TEXT` in
   * `refusals.ts`). The CLI prints this sentence byte for byte (`STORAGE_FULL` in
   * `commands/crew/output.rs`, pinned by a test that reads this file).
   */
  storageFull:
    'This workspace has grown past the size Crew supports and cannot take more changes. Ask the host about starting a new workspace.',
  /**
   * `quota_exceeded: workspace logical state is full; …` and `quota_exceeded: retained audit
   * journal is nearly full; …`: the limits that stop ordinary changes a little short of full, so
   * the host can still remove members and change policy (`commit`'s headroom in `broker.rs`,
   * `FULL_BUT_HOST_CAN_ADMINISTER_TEXT` in `refusals.ts`). The CLI prints the same sentence
   * (`FULL_BUT_HOST_CAN_ADMINISTER` in `commands/crew/output.rs`).
   */
  fullButHostCanAdminister:
    'This workspace is full. Reading still works, and the host can still remove members and change its privacy, but nothing else can change. Ask the host about starting a new workspace.',
  /**
   * `storageFull` said to the host, who is the one to start a new workspace (MSG2-N6): the member
   * sentence told the host to ask the host.
   */
  storageFullHost:
    'This workspace has grown past the size Crew supports and cannot take more changes. To keep working together, start a new workspace.',
  /** `fullButHostCanAdminister` said to the host (MSG2-N6). */
  fullButHostCanAdministerHost:
    'This workspace is full. Reading still works, and you can still remove members and change its privacy. To keep posting, start a new workspace.',
  /** `rate_limited: too many live challenges`. */
  tooManyAttempts: 'Too many attempts at once. Wait a minute, then try again.',
} as const;

export const dialogErrorCopy = {
  /** "Privacy not yet verified" from the copy deck's error strings. */
  privacyUnverified: 'Crew is still checking this workspace’s privacy. Try again in a moment.',
  /** The one fallback a dialog shows when the daemon gave no words. */
  fallback: 'Crew couldn’t complete that action.',
} as const;
