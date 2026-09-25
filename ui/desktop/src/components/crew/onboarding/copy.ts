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
   * it. The host reads it from Crew; the joiner cannot see the host's screen (Q2-04). The host is
   * named, never "they" or "theirs" (Q3-36, Q4-45).
   */
  fingerprintHelper: (host: string) =>
    `This isn’t the code you send; your code appears after you choose Join. To double-check the invitation, ask ${host} to read the fingerprint from Crew (${host}’s workspace menu shows it).`,
  username: (server: string) => `Your username on ${server}`,
  usernameFallback: 'Your username on the server',
  /**
   * The folded privacy line (Q2-36). It says what the choice is FOR — which AI can read the
   * workspace through this connection — because "AI models: …" left a first-day joiner unsure
   * whether it was about their chats, the lab or their agent (Q3-49, Q4-44), and "You'll join as
   * Private" read as an identity. `institution` is already formatted for display.
   */
  privacyLine: (mode: 'private' | 'public', institution: string | null, workspace: string) =>
    mode === 'public'
      ? `Which AI can read ${workspace}: public models allowed for public-safe work`
      : institution
        ? `Which AI can read ${workspace}: private, ${institution}-approved models only`
        : `Which AI can read ${workspace}: private, institution-approved models only`,
  /** The join line while the invitation states no privacy and the person hasn't chosen one. */
  privacyChoose: 'Choose which AI models may work here: Private or Public.',
  change: 'Change',
  /**
   * The joiner's way to change it, inside Advanced (Q4-44): the host's policy decides what the
   * workspace allows, so an accent Change on the direct path to Join invited a click nobody needed.
   */
  privacyChange: (workspace: string) => `Change which AI can read ${workspace}…`,
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
  /**
   * Advanced holds the SSH settings only; the agent's permissions have their own row (Q2-37). The
   * summary says what is inside in plain words; the port is one of the fields there (Q3-49).
   */
  advancedSummary: 'server connection details',
  /**
   * The agent's permissions on the server, in their own labelled row outside Advanced (Q2-37): a
   * permission to run commands on a lab server is not an SSH setting. Whose agent, and one state
   * word beside it while folded — "Your agent on lab-server: off" (Q4-44) — rather than three
   * unfamiliar ideas on one line.
   */
  agentHeading: (server: string) => `Your agent on ${server}:`,
  /** The row's state while folded; off unless the person turned it on. */
  agentSummary: (folder: string, commands: boolean) =>
    folder ? (commands ? `can use ${folder} and run commands there` : `can use ${folder}`) : 'off',
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
  /** Plain words for an absolute path (Q3-49): what it looks like, then what it allows. */
  remoteFolderHelper: (server: string) =>
    `Optional. A folder on ${server}, starting with /. Your agent can read and write files there.`,
  remoteFolderInvalid: 'Start with / — the full path of a folder on the server.',
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
  /**
   * The code's name wherever it is named rather than pointed at ("Copy your code"): one noun for
   * one code (Q3-47). The card's sentence keeps "this code", beside it.
   */
  codeLabel: 'your code',
  waiting: (first: string) => `Waiting for ${first} to let you in…`,
  /**
   * Under the wait (Q4-46): the wait is on a person and can take hours, and nothing here needs the
   * app open. The host enters the code whenever they get to it and the broker records it; this
   * computer's code comes from its saved key, so it stays the same; and the join finishes by
   * itself the next time this connection is connected (the join probe finds the approval and
   * claims it). A reopened app starts with the connection offline, so the one step is Connect.
   */
  closeNote: (first: string, workspace: string) =>
    `You can close Biorouter: ${first} can still let you in with the same code. Next time, open Crew and connect to ${workspace}.`,
  /** The join status's `expires_at`, as "Sat 1:41 AM" (Q4-36). */
  expires: (when: string) => `This invitation expires ${when}.`,
  /** The same once that time has passed, before the next poll says `expired`. */
  expiredNow: 'This invitation has expired.',
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
  /**
   * A member the workspace no longer admits (Q3-50): this computer was a member this session, or
   * the daemon recorded that its membership ended. Removed, not "not yet" invited, so there is no
   * invitation request to send.
   */
  removedTitle: (workspace: string) => `You’re no longer in ${workspace}`,
  removedBody: (workspace: string, person: string) =>
    `This computer or your account was removed from ${workspace}. If you didn’t expect that, ask ${person}.`,
  expired: (person: string) => `This invitation expired. Ask ${person} to invite you again.`,
  yourHost: 'your host',
  /** "your host" at the start of a sentence. */
  yourHostSubject: 'Your host',
  theWorkspace: 'the workspace',
  /** The legacy token path, collapsed: it is needed only when the host says so. */
  other: 'Having trouble joining?',
  /**
   * The first thing "Having trouble joining?" says while the code is out (Q4-43): waiting is the
   * normal state, not trouble. True to the mechanism: the host lets the person in by entering the
   * code they were sent (`letInCopy.helper`), not by seeing it.
   */
  troubleWaiting: (host: string) =>
    `${host} hasn’t let you in yet. That’s normal: ${host} lets you in by entering your code in Crew.`,
  /**
   * The condition first, so the join request never reads as a second thing to send after the code
   * (Q2-35), and named for what it introduces (Q4-43): "send this instead:" led into a link.
   */
  otherBody: (host: string) => `If ${host} asks for a join request:`,
  /** The token path, behind its own quiet link (Q4-43): only a host who sent one makes it needed. */
  tokenInstead: (host: string) => `${host} sent me a token instead`,
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
  /**
   * Under "Having trouble joining?", the device key stays folded behind these (Q2-35). They say
   * what is shown and for whom (Q3-48).
   */
  showRequest: (host: string) => `Show the join request for ${host}`,
  /** Under "If Alice asks for a join request:", which already names whom it is for (Q4-43). */
  showJoinRequest: 'Show the join request',
  hideRequest: 'Hide the join request',
  /** The accessible name of the token field (pinned). */
  tokenName: 'Enrollment invitation',
  tokenPlaceholder: 'Token from an older invitation',
  /** Only the host's older invitation form makes a token: never something to go looking for. */
  tokenHelper: (host: string) => `Only if ${host} sent you a token.`,
  submit: 'Join workspace',
  /**
   * The same button under "Having trouble joining?", where the person already pressed Join: it
   * names the other way in, not the join again (Q3-48).
   */
  submitToken: 'Join with a token',
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
  /** Advanced holds the SSH settings only; the agent row states its own (Q2-37, Q3-49). */
  advancedSummary: joinCopy.advancedSummary,
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
  /**
   * Folded under the start commands (Q4-37): named for the situation the host is in, not for a
   * program they have never heard of. The content is the same install commands.
   */
  notInstalled: (server: string) => `Crew isn’t on ${server} yet?`,
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
  pasteNotInstalled: (server: string) =>
    `Crew isn’t installed on ${server} yet. Open “Crew isn’t on ${server} yet?” below.`,
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
   * Once the connection is saved and the daemon's name for its server differs from the one this
   * dialog has been using (Q4-34): the dialog keeps its word, and this says how the two relate —
   * the words Connection settings uses. Every surface after the dialog uses `label`.
   */
  serverAlias: (label: string) => `Your SSH settings call this server ${label}.`,
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
  // Label step (Q3-45). Only what the broker and daemon enforce: an institution-labelled Private
  // workspace admits an agent only on a model approved for that institution
  // (`enforce_institution_policy`, `institution::admission`), an unlabelled one admits no agent at
  // all ("unlabelled private workspaces allow human collaboration only"), and `policy.set` refuses
  // to change or clear the label once set. The composer's institution note shares these words.
  // `id` is the institution as it reads (`institutionLabel`: "UCSF", Q4-47); the label written is
  // still the canonical ID.
  labelTitle: (workspace: string, id: string) => `Mark ${workspace} as a ${id} workspace?`,
  /**
   * Why it asks again (Q4-47). Step 1's Institution is saved on this computer's connection
   * (`institution_id` on the saved connection: which models this computer's agent may use there);
   * this writes the workspace's own label with `policy.set`, which every member's agent is held to.
   */
  labelWhy: (workspace: string, institution: string) =>
    `Step 1 set ${institution} for your connection on this computer. This sets it for ${workspace} itself, for everyone who works there.`,
  /** Workspace-free, for the composer's note, whose title names the workspace. */
  labelBody:
    'Agents working here can then use only models approved for that institution. This can’t be undone.',
  labelEffect: (workspace: string, id: string) =>
    `Agents working in ${workspace} can then use only models approved for ${id}. This can’t be undone.`,
  /** Under the choice: what Not now leaves open, and where to finish it later. */
  labelLaterHelper: (workspace: string) =>
    `Until then, people can chat and share files in ${workspace}, but agents can’t work there. You can do this later from Get ${workspace} ready, or from Privacy… in the workspace menu.`,
  labelSet: (id: string) => `Mark as ${id}`,
  labelLater: 'Not now',
  done: 'Done',
} as const;

export const checklistCopy = {
  title: (workspace: string) => `Get ${workspace} ready`,
  /**
   * The server-account name, offered to the host before Invite (Q3-51), so the first invitation
   * names them. Offered, never applied: Use and Edit… are the answers.
   */
  name: (name: string) => `Your name: Use “${name}”?`,
  nameSet: (name: string) => `Your name: ${name}`,
  /**
   * Under the title when the daemon's name for the server differs from its address (Q4-34): the
   * host typed the address, and every surface from here on says the alias.
   */
  serverAlias: (workspace: string, address: string, label: string) =>
    `${workspace} is on ${address}, which your SSH settings call ${label}.`,
  useName: nameSuggestionCopy.use,
  editName: nameSuggestionCopy.edit,
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
  /**
   * Under Connect after a connect of this connection failed (Q4-07): a repeat click that fails the
   * same way still visibly did something. `time` carries seconds for exactly that reason.
   */
  triedAgain: (time: string, reason: string) => `Tried again at ${time}. ${reason}.`,
  /** The failure's reason, by kind, as a clause (no final period). */
  failureReason: (kind: string, server: string) => {
    switch (kind) {
      case 'unreachable':
        return `Couldn’t reach ${server}`;
      case 'auth_required':
        return `${server} asked you to sign in`;
      case 'host_key_unknown':
      case 'host_key_changed':
      case 'workspace_identity_mismatch':
        return `Crew couldn’t verify ${server}`;
      case 'bridge_missing':
      case 'handoff_failed':
        return `Crew isn’t running for you on ${server}`;
      default:
        return 'It didn’t connect';
    }
  },
  /**
   * For a network failure only (Q4-06): the daemon re-dials a connection a network failure took
   * down, for up to an hour (D-KEEPALIVE, Q4-01). Never said for a sign-in or host-key failure,
   * which nothing retries by itself.
   */
  keepsTrying: 'Crew keeps trying by itself while the network is down.',
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
