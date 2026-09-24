/**
 * The onboarding area's strings: first run, joining by invitation, hosting a new workspace, the
 * join states, the connection-problem panes and the empty states outside a channel
 * (ui-redesign-spec, copy deck "Onboarding, join, host and admit" and "Connection problems and
 * sign in").
 *
 * American English, sentence case, typographic apostrophes. A string a test asserts is read from
 * here, so the component and the assertion cannot drift apart. Names arrive already formatted
 * (`personLabel`, `workspaceName`, `connectionServer`); nothing here formats a person or an ID.
 */

export const welcomeCopy = {
  title: 'Work together in Crew',
  body: 'Chat, share files and run agents with your lab.',
  join: 'Join a workspace',
  host: 'Host a new workspace',
} as const;

export const joinCopy = {
  title: 'Join a workspace',
  invitation: 'Invitation from your host',
  invitationPlaceholder: 'Paste the whole message your host sent you',
  checking: 'Reading the invitation…',
  /** The invitation box after the daemon read it: the summary below says what it holds. */
  invitationRead: 'Invitation read',
  editInvitation: 'Edit',
  editInvitationLabel: 'Edit the invitation',
  invalid: 'This doesn’t look like a Crew invitation. Ask your host to copy it again.',
  hostedBy: 'Hosted by',
  on: 'on',
  fingerprint: 'Fingerprint',
  fingerprintLabel: 'workspace fingerprint',
  /** What to do with the fingerprint: compare it with the host's. */
  fingerprintHelper: (host: string) => `Check this matches the fingerprint ${host} sees.`,
  username: (server: string) => `Your username on ${server}`,
  usernameFallback: 'Your username on the server',
  privacyLine: 'You’ll join as',
  /** The join line while the invitation states no privacy and the person hasn't chosen one. */
  privacyChoose: 'Choose how you’ll join: Private or Public.',
  change: 'Change',
  privacy: 'Privacy',
  /** The summary's privacy when the invitation doesn't state the workspace's. */
  privacyUnstated: 'The invitation doesn’t say',
  private: 'Private',
  privateHint: 'Only private and institution-approved models',
  public: 'Public',
  publicHint: 'Public models allowed for public-safe work',
  institution: 'Institution',
  institutionPlaceholder: 'For example, ucsf or sdsc',
  institutionHelper: 'Your organization’s short ID. It must match the one your host uses.',
  /** The invitation states no institution, so the joiner can't know it: say whom to ask. */
  institutionUnknown: (host: string, workspace: string) =>
    `Your invitation didn’t include the lab’s institution. Ask ${host} which institution ${workspace} uses.`,
  institutionRequired: 'Private needs an institution. Enter it, or choose Public.',
  institutionInvalid: 'Use the short ID: lowercase letters, numbers, - or _, like ucsf.',
  /** The visible mark beside a field that must be filled in. */
  required: 'Required',
  fieldRequired: 'Fill this in to continue.',
  fieldInvalid: 'Check this value.',
  /** "{workspace} is Private for ucsf. Your connection will be Public." */
  mismatch: (workspace: string, workspaceMode: string, choice: string) =>
    `${workspace} is ${workspaceMode}. Your connection will be ${choice}.`,
  privateFor: (institution: string) => `Private for ${institution}`,
  submit: (workspace: string) => `Join ${workspace}`,
  submitFallback: 'Join workspace',
  connecting: (server: string) => `Connecting to ${server}…`,
  cancel: 'Cancel',
  unnamedWorkspace: 'this workspace',
  advancedSummary: (port: number) => `Port ${port} · your SSH settings`,
  serverLogin: 'Server login',
  serverLoginHelper: (defaultLogin: string) =>
    `An SSH alias from your SSH config, instead of ${defaultLogin}.`,
  serverLoginInvalid:
    'Use a server login or SSH alias: letters, numbers and _ . / : @ % -, not starting with a dash.',
  port: 'Port',
  portInvalid: 'Use a port from 1 to 65535.',
  identityFile: 'Identity file',
  identityFileHelper: 'Leave empty to use your SSH config.',
  jumpHost: 'Jump host',
  connectionName: 'Connection name',
  remoteFolder: 'Remote work folder',
  remoteFolderHelper: 'An absolute path on the server.',
  remoteFolderInvalid: 'Start with / — an absolute path on the server.',
  remoteExecution: 'Let my agent run commands in this folder',
  /** Why the agent-commands switch is off limits: it needs the folder first. */
  remoteExecutionNeedsFolder: 'Add a remote work folder first.',
  manual: 'Enter workspace details manually',
  manualServerLogin: 'Your server login',
  manualServerLoginPlaceholder: 'e.g. bob@hpc.example.edu',
  socketPath: 'Socket path',
  socketPathInvalid: 'Start with / — the full path Crew printed.',
  workspaceId: 'Workspace ID',
  hostUserId: 'Host user ID',
  hostUserIdInvalid: 'A number, like 1000.',
  workspaceKey: 'Workspace key',
  workspaceKeyHelper: '64 characters, 0–9 and a–f.',
  staleDaemon:
    'This feature needs a newer Biorouter background service. Quit and reopen Biorouter, or enter the workspace details manually under Advanced.',
  /** The invitation names a workspace this computer already has a connection for. */
  existing: (workspace: string) => `You already have ${workspace} on this computer.`,
  openExisting: (connection: string) => `Open ${connection}`,
  /** A legacy paste names no server: the login has to come from Advanced. */
  serverMissing: 'This invitation doesn’t name its server. Type your server login here.',
} as const;

export const joinStateCopy = {
  checking: 'Checking your invitation…',
  invited: (person: string, workspace: string) => `${person} invited you to ${workspace}.`,
  /** The invitation adds this computer to the person's existing account. */
  invitedDevice: (person: string, workspace: string) =>
    `${person} invited this computer to your account in ${workspace}.`,
  sendCode: (first: string) => `Send ${first} this code:`,
  codeLabel: 'device code',
  waiting: (first: string) => `Waiting for ${first} to let you in…`,
  approved: (workspace: string) => `Joining ${workspace}…`,
  approvedDevice: (workspace: string) => `Adding this computer to ${workspace}…`,
  mismatchCode: (first: string) =>
    `The code ${first} entered doesn’t match this computer. Send it again:`,
  notInvitedTitle: (workspace: string) => `You’re not in ${workspace} yet`,
  notInvitedBody: (person: string, username: string | null) =>
    username
      ? `Ask ${person} to invite @${username}. This page updates by itself.`
      : `Ask ${person} to invite you. This page updates by itself.`,
  notInvitedMessage: (first: string | null, username: string | null, workspace: string) =>
    `${first ? `Hi ${first}, please` : 'Please'} invite ${username ? `@${username}` : 'me'} to ${workspace} in Crew.`,
  notInvitedMessageLabel: 'message to your host',
  expired: (person: string) => `This invitation expired. Ask ${person} to invite you again.`,
  yourHost: 'your host',
  /** "your host" at the start of a sentence. */
  yourHostSubject: 'Your host',
  theWorkspace: 'the workspace',
  /** The legacy token path, collapsed: it is needed only when the host says so. */
  other: 'Having trouble joining?',
  otherBody:
    'Only needed if your host’s Crew can’t let you in with a code. Your host will tell you if so.',
  pollFailed: 'Crew couldn’t check your invitation. It tries again by itself.',
  claimFailed: 'Joining didn’t finish.',
  retry: 'Try again',
  /** The join route answered `crew_not_connected`: Crew is reconnecting once by itself. */
  reconnecting: (workspace: string) => `Reconnecting to ${workspace}…`,
  /** That reconnect didn't help: the person reconnects (and signs in, if the server asks). */
  notConnected: (workspace: string) =>
    `Crew isn’t connected to ${workspace}, so it can’t check your invitation.`,
  reconnect: 'Reconnect',
  hostPendingTitle: (workspace: string) => `Finish creating ${workspace}`,
  hostPendingBody: 'Crew is running on the server. Create the workspace to become its first admin.',
  hostPendingAction: 'Finish creating…',
} as const;

export const legacyJoinCopy = {
  title: 'Join with an invitation token',
  sendRequest: 'Send this join request to your host:',
  requestLabel: 'join request',
  /** The accessible name of the token field (pinned). */
  tokenName: 'Enrollment invitation',
  tokenPlaceholder: 'Token from an older invitation',
  submit: 'Join workspace',
  submitting: 'Joining…',
  /** The body of the join request, which a host pastes into the older invitation form. */
  request: (workspace: string, username: string | null, key: string) =>
    [
      `Crew join request for ${workspace}`,
      ...(username ? [`Username: ${username}`] : []),
      `Device key: ${key}`,
    ].join('\n'),
} as const;

export const nameSuggestionCopy = {
  prompt: (name: string, workspace: string) => `Use “${name}” as your name in ${workspace}?`,
  use: 'Use',
  edit: 'Edit…',
  dismiss: 'Dismiss',
} as const;

export const hostCopy = {
  title: 'Host a new workspace',
  steps: ['Name', 'Start', 'Create'] as const,
  stepOf: (step: number, total: number, name: string) => `Step ${step} of ${total} · ${name}`,
  stepsLabel: 'Steps',
  // Step 1
  nameHeading: 'Name your workspace',
  workspaceName: 'Workspace name',
  workspaceNamePlaceholder: 'e.g. lab',
  workspaceNameHelper: 'Lowercase letters, numbers and dashes.',
  workspaceNameInvalid: 'Use at least one letter or number.',
  preview: (slug: string) => `Your workspace: ${slug}`,
  serverLogin: 'Your server login',
  serverLoginPlaceholder: 'e.g. alice@hpc.example.edu',
  /** The host is the one whose institution everyone else must match. */
  institutionHelper: 'Your organization’s short ID. People who join as Private use the same one.',
  continue: 'Continue',
  preparing: 'Preparing…',
  advancedSummary: 'Port 22 · your SSH settings · agent commands off',
  jumpHosts: 'Jump hosts',
  // Step 2
  startHeading: (server: string) => `Start Crew on ${server}`,
  runThis: (server: string, user: string | null) =>
    user
      ? `Run this in a terminal signed in to ${server} as ${user}:`
      : `Run this in a terminal signed in to ${server}:`,
  commandLabel: 'start command',
  openTerminal: 'Open a terminal here',
  hideTerminal: 'Hide terminal',
  pasted: 'Paste what it printed',
  pastedPlaceholder: 'Everything the commands printed',
  notSignedIn: 'Not signed in to the server in a terminal yet?',
  sshCommandLabel: 'sign-in command',
  confirmServer:
    'If your terminal asks you to confirm the server, compare the fingerprint with the one from your IT team. Type yes only if they match.',
  notInstalled: 'biorouter-crew isn’t installed yet?',
  installLabel: 'install commands',
  consequence: 'Anyone who can sign in to this server can see the workspace name.',
  bad: 'That isn’t what Crew prints. Copy everything after the command ran and paste again.',
  /** The paste, checked inline before Continue. */
  checkingPaste: 'Checking what you pasted…',
  found: (workspace: string, server: string) => `Found ${workspace} on ${server}`,
  theServer: 'the server',
  pasteStarting:
    'Crew was still starting when this was printed. Wait a few seconds, run the last command again and paste what it prints.',
  pasteCutOff:
    'The paste stops partway through what Crew printed. Copy the whole line, from { to }, and paste again.',
  pasteNotInstalled:
    'biorouter-crew isn’t installed on the server yet. Open “biorouter-crew isn’t installed yet?” below.',
  pasteServerError: (detail: string) => `Crew on the server said: ${detail}`,
  /** A paste the daemon read, but whose preview lacks the details a new workspace pins. */
  detailsMissing:
    'Biorouter read the workspace but not every detail it needs. Enter the rest from what the commands printed.',
  staleDaemon:
    'This feature needs a newer Biorouter background service. Quit and reopen Biorouter, or enter the workspace details below.',
  reading: 'Reading…',
  back: 'Back',
  // Step 3
  createHeading: (workspace: string, server: string) => `${workspace} on ${server}`,
  createBody: (workspace: string) =>
    `Creating ${workspace} makes this computer its first admin device.`,
  /** The host's own fingerprint: what the people they invite compare. */
  fingerprintHelper:
    'People you invite see this fingerprint when they join. Tell them to check it matches.',
  create: 'Create workspace',
  creating: 'Creating…',
  signingIn: 'Waiting for you to sign in…',
  signInEnded: 'Sign-in didn’t finish. Choose Create workspace to try again.',
  // Label step
  labelTitle: (workspace: string, id: string) => `Label ${workspace} as ${id}?`,
  labelBody: 'This can’t be changed later.',
  labelSet: (id: string) => `Set ${id} permanently`,
  labelLater: 'Not now',
  done: 'Done',
} as const;

export const checklistCopy = {
  title: (workspace: string) => `Get ${workspace} ready`,
  institution: 'Confirm the institution',
  setInstitution: (id: string) => `Set institution to ${id}…`,
  institutionMissing: 'Add your institution in Connection settings first.',
  connectionSettings: 'Connection settings…',
  institutionPublic: 'Not needed while the workspace allows Public.',
  team: 'Create a team',
  createTeam: 'Create team',
  invite: 'Invite people',
  invitePeople: 'Invite people…',
  /** The compact nudge a channel keeps while the host is still alone in the workspace. */
  aloneTitle: (workspace: string) => `No one else has joined ${workspace} yet.`,
  invitePeopleTo: (workspace: string) => `Invite people to ${workspace}…`,
  done: 'Done',
  hide: 'Hide',
  label: 'Setup checklist',
} as const;

export const trustCopy = {
  unknownTitle: (host: string) => `Can’t verify ${host} yet`,
  unknownBody: 'Crew only connects to servers you’ve already verified.',
  offered: 'Fingerprint the server offered',
  offeredLabel: 'server fingerprint',
  howToVerify: 'How do I verify it?',
  steps: (host: string) =>
    [
      `Get ${host}’s fingerprint from your IT team or your institution’s directory. Check jump hosts too.`,
      'Compare it using your usual SSH setup, then add the full key to your known-hosts file. A fingerprint alone isn’t enough.',
      'Come back and choose Try again.',
    ] as const,
  tryAgain: 'Try again',
  changedTitle: (host: string) => `${host}’s identity changed`,
  changedBody:
    'Don’t connect until your IT team confirms this change. Crew won’t connect while the old key is in your known-hosts file.',
  previous: 'Fingerprint Crew knew',
  newKey: 'Fingerprint the server offered now',
  copyForIt: 'Copy details for IT',
  workspaceTitle: 'This isn’t the workspace you joined',
  workspaceBody: (host: string) =>
    `The server answered with a different workspace key. Don’t continue until ${host} confirms what changed.`,
  copyDetails: 'Copy details',
  yourHost: 'your host',
  connectionSettings: 'Connection settings…',
  copied: 'Copied',
  copyFailed: 'Copy failed',
  detailsFallbackLabel: 'details',
  /** The first lines of "Copy details for IT"; OpenSSH's own words follow. */
  detailsHeader: (host: string, problem: string) => [`Server: ${host}`, `Problem: ${problem}`],
  changedProblem: 'The server’s host key changed since Crew last connected.',
  workspaceProblem: 'The server answered with a different workspace key than the one saved.',
} as const;

export const notSetUpCopy = {
  title: (server: string) => `Crew isn’t set up for your account on ${server}`,
  body: 'It’s installed once per account, usually by your host or IT team.',
  message: (hostFirst: string | null, username: string | null, server: string) =>
    `${hostFirst ? `Hi ${hostFirst}, Crew` : 'Crew'} isn’t set up for my account${
      username ? ` (@${username})` : ''
    } on ${server} yet. Could you or IT install biorouter-crew in ~/.local/bin for me?`,
  messageLabel: 'message to your host',
  installYourself: 'Install it yourself',
  installIntro: 'Get a verified biorouter-crew file for this server, then run:',
  installLabel: 'install commands',
  tryAgain: 'Try again',
} as const;

export const emptyCopy = {
  connecting: (server: string) => `Connecting to ${server}…`,
  offlineTitle: (workspace: string) => `${workspace} is offline`,
  offlineBody: 'Connect to see your channels.',
  offlineAction: (workspace: string) => `Connect to ${workspace}`,
  signInTitle: (host: string) => `Sign in to ${host}`,
  signInBody: 'The server needs your password or a verification code.',
  signInAction: 'Sign in',
  memberTitle: (workspace: string) => `You’re in ${workspace}`,
  memberBody: (host: string) => `Ask ${host} to add you to a team.`,
  memberAction: 'Create a team',
  invitedTitle: (team: string) => `You’re invited to ${team}`,
  invitedBody: (person: string) => `${person} invited you.`,
  invitedBodyUnknown: 'You have an invitation to this team.',
  invitedAction: (team: string) => `Join ${team}`,
  aTeam: 'a team',
  yourHost: 'your host',
  noChannelTitle: (team: string) => `No open channels in ${team}`,
  noChannelBody: 'Create one to start talking.',
  noChannelAction: 'Create channel',
} as const;

/** The per-account install commands (cli-guide.md, "Install the remote executable"). */
export const INSTALL_COMMANDS = [
  "VERIFIED_BINARY='/replace/with/path/to/verified/linux/biorouter-crew'",
  'mkdir -p "$HOME/.local/bin"',
  'install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"',
  '"$HOME/.local/bin/biorouter-crew" --version',
].join('\n');
