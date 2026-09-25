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
  /**
   * The quiet disclosure the fingerprint sits behind (Q2-04). It looks like the join code (four
   * groups of four), so it is folded away, has no Copy, and says what it is not.
   */
  fingerprintCheck: 'Check this invitation (optional)',
  /**
   * After "Fingerprint 6682 327B A040 C709.": what it is for, and how the joiner can actually check
   * it. The host reads theirs from Crew; the joiner cannot see the host's screen (Q2-04).
   */
  fingerprintHelper: (host: string) =>
    `This isn’t the code you send; your code appears after you choose Join. To double-check the invitation, ask ${host} to read theirs from Crew (their workspace menu shows it).`,
  username: (server: string) => `Your username on ${server}`,
  usernameFallback: 'Your username on the server',
  /**
   * The folded privacy line (Q2-36). It names what the choice governs, the models, because "You'll
   * join as Private" read as an identity. `institution` is already formatted for display.
   */
  privacyLine: (mode: 'private' | 'public', institution: string | null) =>
    mode === 'public'
      ? 'Models: public allowed for public-safe work'
      : institution
        ? `Models: private and ${institution}-approved only`
        : 'Models: private and institution-approved only',
  /** The join line while the invitation states no privacy and the person hasn't chosen one. */
  privacyChoose: 'Choose which models may work here: Private or Public.',
  change: 'Change',
  /** Folds the open privacy choice back into its one-line summary. */
  privacyDone: 'Done',
  /**
   * Public chosen for a workspace the invitation states is Private (Q2-36). Every clause is what
   * the daemon and broker enforce, not a guess: a Private workspace blocks public models whatever
   * the connection says (`institution::admission`, broker `validate_run`), and still holds the agent
   * to the workspace's institution; public models reach public-safe channels only once both the
   * workspace and the connection are Public; and the connection's privacy is saved on this
   * computer and sent to nobody (the join claim carries no mode). A Public connection does NOT
   * limit what the person reads: Restricted is about models only.
   */
  publicConsequence: (
    workspace: string,
    institution: string | null,
    host: string,
    hostSubject: string
  ) =>
    `${workspace} is Private, so nothing changes yet: your agent still uses only private and ${
      institution ? `${institution}-approved` : 'institution-approved'
    } models here. If ${host} makes ${workspace} Public, public models could read its public-safe channels through your agent. ${hostSubject} isn’t told what you chose.`,
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
  /** Advanced holds the SSH settings only; the agent's permissions have their own row (Q2-37). */
  advancedSummary: (port: number) => `Port ${port} · your SSH settings`,
  /**
   * The agent's permissions on the server, in their own labelled row outside Advanced (Q2-37): a
   * permission to run commands on a lab server is not an SSH setting.
   */
  agentHeading: (server: string) => `Agent on ${server}`,
  /** The row's state while folded; off unless the person turned it on. */
  agentSummary: (folder: string, commands: boolean) =>
    `${folder || 'No work folder'} · agent commands ${commands ? 'on' : 'off'}`,
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
  remoteFolderHelper:
    'Optional. An absolute path on the server; your agent can read and write files there.',
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
  /**
   * The condition first, so the join request never reads as a second thing to send after the code
   * (Q2-35).
   */
  otherBody: (host: string) => `If ${host} asks for it, send this instead:`,
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
  /** Under "Having trouble joining?", the device key stays folded behind these (Q2-35). */
  showRequest: 'Show join request',
  hideRequest: 'Hide join request',
  /** The accessible name of the token field (pinned). */
  tokenName: 'Enrollment invitation',
  tokenPlaceholder: 'Token from an older invitation',
  /** Only the host's older invitation form makes a token: never something to go looking for. */
  tokenHelper: (host: string) => `Only if ${host} sent you a token.`,
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
  /** Advanced holds the SSH settings only; the agent row states its own (Q2-37). */
  advancedSummary: 'Port 22 · your SSH settings',
  jumpHosts: 'Jump hosts',
  // Step 2
  startHeading: (server: string) => `Start Crew on ${server}`,
  /** Above the start commands, which both ways run exactly (D-HOST). */
  runThis: (server: string, user: string | null) =>
    user
      ? `These commands start Crew on ${server} as ${user}:`
      : `These commands start Crew on ${server}:`,
  commandLabel: 'start command',
  // "Start it for me" (D-HOST): the daemon runs exactly the commands above, over SSH, on a click.
  startForMe: 'Start it for me',
  startForMeHint: (server: string, user: string | null) =>
    `Biorouter signs in to ${server}${
      user ? ` as ${user}` : ''
    } with your SSH settings and runs exactly these commands. If the server asks for a password or a code, run them yourself instead.`,
  startRunning: (server: string) => `Starting Crew on ${server}…`,
  startReading: 'Reading what Crew printed…',
  stop: 'Stop',
  /** The accessible name of the box that shows the commands' output as it arrives. */
  startOutput: 'What the server printed',
  startStarting: 'Crew was still starting. Wait a few seconds, then choose Start it for me again.',
  startUnreadable:
    'Biorouter couldn’t find what Crew prints in the output. Run the commands yourself to see what the server says.',
  startCommandChanged:
    'Biorouter stopped because the commands it was about to run weren’t the ones shown here. Run them yourself instead.',
  startFailed:
    'Starting Crew didn’t finish. Run the commands yourself to see what the server says.',
  startStaleDaemon:
    'Start it for me needs a newer Biorouter background service. Quit and reopen Biorouter, or run the commands yourself.',
  /** The manual path, folded (D-HOST): the same commands, in the person's own terminal. */
  runYourself: 'Run it yourself in a terminal',
  runYourselfBody: (server: string, user: string | null) =>
    user
      ? `Run the commands above in a terminal signed in to ${server} as ${user}, then paste what they printed.`
      : `Run the commands above in a terminal signed in to ${server}, then paste what they printed.`,
  openTerminal: 'Open a terminal here',
  hideTerminal: 'Hide terminal',
  pasted: 'Paste what it printed',
  pastedPlaceholder: 'Everything the commands printed',
  notSignedIn: 'Not signed in to the server in a terminal yet? Sign in first:',
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
  /**
   * The host's own fingerprint: what a joiner may ask them to read out (Q2-04). The joiner's Join
   * dialog tells them to ask, and the workspace menu shows it after this dialog closes.
   */
  fingerprintHelper:
    'People you invite may ask you to read this to check their invitation. Your workspace menu shows it too.',
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
