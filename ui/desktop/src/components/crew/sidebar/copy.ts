/**
 * The Crew sidebar's strings (ui-redesign-spec, copy deck "Sidebar and menus" and "Privacy
 * popover").
 *
 * Tests import these instead of retyping them. The status words and the pinned verified sentence
 * are not here: more than one area shows them, so they live in `state/copy.ts`
 * (`crewStatusCopy`), and this file only re-exports them for convenience.
 *
 * American English, sentence case, typographic apostrophes; an item that opens a dialog ends in
 * "…". A string that names a person takes it already formatted by `personLabel`, so nothing here
 * ever formats a person or prints an ID.
 */
import { crewStatusCopy } from '../state/copy';

export { crewStatusCopy };

export const sidebarCopy = {
  /** The Crew sidebar's `<nav>` accessible name. */
  navLabel: 'Crew',
  /**
   * The `<nav>`'s `aria-description` (T-64): the rows are ONE roving-focus list, so Tab alone
   * skips most of them. Said once, when the landmark is entered.
   */
  navDescription:
    'Use the Up and Down arrow keys to move between teams and channels. Left and Right collapse and expand a team. On a team, Tab reaches its Create channel and options buttons.',
  /** The status row's `role="status"` name. */
  statusLabel: 'Connection status',

  /**
   * What a status word waits for, in its tooltip and after it for a screen reader (Q2-17, Q2-43).
   * Only a status that has more to say gets one: a tooltip that repeats the word says nothing.
   */
  statusHint: {
    /** `workspace` is the switcher's name. The Retry it names is the connection bar's. */
    updatesUnavailable: (workspace: string) =>
      `Crew isn’t receiving updates for ${workspace}. Retry below.`,
    /** `host` is `personLabel(…, 'inline')`, or null when the invitation named nobody. */
    notJoined: (host: string | null) =>
      host ? `Waiting for ${host} to let you in` : 'Waiting for your host to let you in',
  },

  /** A joiner the host has not let in yet: what the empty column will hold (Q2-43). */
  pendingColumn: (host: string | null) =>
    host
      ? `Your channels appear here once ${host} lets you in.`
      : 'Your channels appear here once your host lets you in.',

  switcher: {
    /** Before a connection is selected (the selection lands with the saved list). */
    loading: 'Loading workspaces…',
  },

  chip: {
    /**
     * The chip's accessible name (the test anchor): `Privacy: Private · ucsf` / `Privacy: Public`.
     * `institution` is already an institution label (`UCSF` when a configured provider publishes
     * that name for `ucsf`, else the ID itself).
     */
    name: (mode: 'private' | 'public', institution: string | null) =>
      mode === 'public'
        ? 'Privacy: Public'
        : institution
          ? `Privacy: Private · ${institution}`
          : 'Privacy: Private',
    /**
     * Shown, with no padlock, while the observer (re-)verifies this connection's privacy. It
     * resolves: a member's connection is verified on the next snapshot.
     */
    checking: 'Checking privacy…',
    /** The tooltip that says what "Checking privacy…" is waiting for (T-06, T-68). */
    checkingHint:
      'Checking privacy: Crew is confirming this connection’s privacy with the workspace. It shows here once confirmed.',
    /**
     * A joiner the host has not let in yet (T-06). No longer shown (Q2-43): the status row then
     * says only "Not joined yet", and the chip renders nothing until privacy can be verified.
     */
    notJoined: 'Privacy shown after you join',
    notJoinedHint:
      'Privacy shown after you join: the workspace reports your connection’s privacy once the host lets you in.',
  },

  section: {
    /** Authored in sentence case; `text-caps` uppercases them. */
    invitations: 'Invitations',
    waiting: 'Waiting to join',
    /**
     * The host's reminder of people who joined and are in none of their teams (Q3-52). "Your
     * teams", not "a team": the host's snapshot holds only the teams the host is in, so someone in
     * a team the host is not in would otherwise be told falsely they are in none.
     */
    joined: 'Joined, not in your teams',
  },

  invitation: {
    /**
     * `{target}` over a muted `from {inviter}`. The inviter is a person, so it renders through
     * `PersonName` (`inline`, as the identity rules place invitation rows), never as text here.
     */
    from: 'from',
    /** Before the broker names the target (S1a): never an ID. */
    untitled: 'Invitation',
    /**
     * One verb for one action (T-41): the invitation's button and the main area's
     * "Join {team}" both say Join.
     */
    accept: 'Join',
    acceptLabel: (target: string) => `Join ${target}`,
    /** `inviter` is `personLabel(…, 'inline')`. */
    acceptFromLabel: (inviter: string) => `Join, invited by ${inviter}`,
  },

  waiting: {
    /** After the joiner's name: they have an invitation and no code has been entered (Q2-42). */
    invited: 'invited',
    /** Under it: whose turn it is. The host acts once the joiner sends their code. */
    nextStep: 'Let in… when they send their code',
    letIn: 'Let in…',
    letInLabel: (username: string) => `Let @${username} in`,
    /**
     * The host entered a code (T-13). Not "Approved": the broker compares the code only when the
     * joiner's computer checks in, so a code entered here may still turn out not to match.
     */
    approved: 'Code entered',
    /**
     * A claim was refused because its code did not match (T-13). The same words as the Let-in
     * dialog and the CLI: the host's own typo and a different computer look the same from here,
     * so the sentence covers both and says what to do about each.
     */
    otherDevice: (username: string) =>
      `A computer trying to join as @${username} showed a different code. Check the code @${username} sent you; if you typed it wrong, let them in again with the right code. Don’t approve a code you didn’t get from @${username}.`,
    /** Spoken, never shown (T-17): the Waiting to join list changed. */
    announceWaiting: (username: string, workspace: string) =>
      `@${username} is waiting to join ${workspace}.`,
    announceCodeEntered: (username: string) =>
      `Code entered for @${username}; joins when their computer confirms.`,
    /** The mismatch, as a one-time `role="alert"`: the visible sentence, with a lead word. */
    alertOtherDevice: (username: string) =>
      `Warning: a computer trying to join as @${username} showed a different code.`,
    /** A join whose invitation ran out: it can no longer be let in, only invited again. */
    expired: 'Invitation expired',
    /** Between `expired` and `inviteAgain`, read as one line. */
    separator: '·',
    inviteAgain: 'Invite again…',
    inviteAgainLabel: (username: string) => `Invite @${username} again`,
  },

  /**
   * A person who joined and is in none of the host's teams (Q3-52): the host who let them in and
   * closed the dialog ("you can close this") is brought back to add them. `person` is
   * `personLabel(…, 'inline')`; `team` is the team as typed.
   */
  joined: {
    /** After the person's name, muted: `Gina Rossi (@crew_gina) · joined`. */
    state: 'joined',
    addToTeam: 'Add to a team…',
    addToTeamLabel: (person: string) => `Add ${person} to a team`,
    addToTeamNamedLabel: (person: string, team: string) => `Add ${person} to ${team}`,
    /** Spoken once, when someone new appears in the section. */
    announce: (person: string) => `${person} isn’t in any of your teams yet.`,
  },

  team: {
    /** The team header's accessible name. */
    toggleLabel: (team: string, count: number) =>
      `${team}, ${count} ${count === 1 ? 'channel' : 'channels'}`,
    /**
     * The team owner's pending team invitations (P0-2): the people who have been invited and
     * have not accepted yet, so "Invited" is never read as "added".
     */
    invited: (count: number) => `${count} invited`,
    invitedNames: (names: readonly string[]) => `Invited, not accepted yet: ${names.join(', ')}`,
    /**
     * For a member (T-28): the snapshot holds only the channels they are in, so the rest of a
     * team's channels are invisible until someone adds them.
     */
    memberHint: (team: string) => `Other channels in ${team} appear once someone adds you.`,
    options: (team: string) => `${team} options`,
    addChannel: (team: string) => `Create channel in ${team}`,
    add: 'Add team',
  },

  channel: {
    /** A channel row's accessible name. */
    rowLabel: (name: string, unread: number) => (unread > 0 ? `${name}, ${unread} unread` : name),
    add: 'Add channel',
    archivedGroup: (count: number) => `Archived (${count})`,
    /** The unread count badge caps here. */
    unreadCap: 99,
    unreadOverCap: '99+',
  },

  channelMenu: {
    markRead: 'Mark as read',
    copyName: 'Copy channel name',
    copyId: 'Copy channel ID',
  },

  teamMenu: {
    /**
     * First, for everyone (Q3-44): the team's member list. It opens the same Add people dialog,
     * which shows someone who can't add people there only the list (the `people.ts` rule).
     */
    members: (team: string) => `Members of ${team}…`,
    createChannel: 'Create channel…',
    addPeople: (team: string) => `Add people to ${team}…`,
    rename: 'Rename team…',
    /**
     * The submenu that holds a menu's machine-ID copies, and only those, last after a separator
     * (Q3-26, the shared "Copy for support" contract): a PI's everyday menu leads with what a
     * person does, and an ID is what a support conversation asks for.
     */
    copyForSupport: 'Copy for support',
    copyId: 'Copy team ID',
    /** The item confirms the copy itself, then the menu closes still saying so (Q2-34, Q3-57). */
    copied: 'Copied',
    copyFailed: 'Couldn’t copy',
  },

  workspaceMenu: {
    hostedBy: 'Hosted by',
    /** `Signed in as @alice on hpc.ucsf.edu`: the person, then the server (T-40). */
    signedInAs: 'Signed in as',
    signedInOn: 'on',
    /** Before this connection's identity is verified there is no person to name, only a server. */
    server: 'Server',
    /**
     * The workspace key's fingerprint, grouped as the Join dialog shows it, so a host asked
     * "does the fingerprint match?" finds it where the identity is (Q2-04).
     */
    fingerprint: 'Fingerprint',
    /**
     * Under Reconnect and Disconnect while a join waits for the host (Q2-43, Q3-47): what each one
     * does, then that the code already sent survives it — two helpers, where one sentence under
     * both said nothing about the difference. True: the code is computed from this computer's
     * saved device key and the pinned workspace key (`device_code_of`), which neither action
     * touches — a Disconnect closes the SSH transport and forgets the broker's announcement, and
     * the join screen's in-memory counters (`joinClaimState`) are forgotten only when the join
     * finishes — and the broker keeps the pending join, so the host can still enter that code.
     * `host` is `personLabel(…, 'inline')`, or null when the invitation named nobody.
     */
    joinCodeKept: {
      reconnect: 'Try the connection again. Your code doesn’t change.',
      disconnect: (host: string | null) =>
        `Stop waiting for now. ${host ?? 'Your host'} can still let you in with the same code.`,
    },
    invite: (workspace: string) => `Invite people to ${workspace}…`,
    people: 'People…',
    privacy: 'Privacy…',
    /** The same name as Workspace settings' tab and the pane's Access heading. */
    access: 'Agent access…',
    createTeam: 'Create team…',
    /**
     * Pinned. Offered only while the connection is not connected and verified, or while a join
     * waits for the host (Q3-57): beside "Connected" it did nothing but invite a stray Enter.
     */
    reconnect: 'Reconnect',
    signIn: 'Sign in…',
    disconnect: 'Disconnect',
    settings: 'Connection settings…',
    switchWorkspace: 'Switch workspace',
    add: 'Add a workspace',
    addJoin: 'Join a workspace…',
    addHost: 'Host a new workspace…',
  },

  /**
   * Why a menu item is disabled (T-40, T-71). A disabled item says so, never silently greys out.
   */
  unavailable: {
    notJoined: 'Available after you join',
    notConnected: 'Available once you’re connected',
    /** The connection is up (checking, updating, updates unavailable) but not verified yet. */
    notVerified: 'Available once the connection is verified',
  },

  you: {
    /** A dev profile's badge, in the You menu's header (T-71): the row keeps its width for names. */
    devProfile: (name: string) => `Profile: ${name}`,
    editProfile: 'Edit profile…',
    keys: 'Keys and security…',
    copyUsername: 'Copy my username',
    /**
     * The item itself confirms the copy, and the menu closes a moment later still saying so, as
     * every sidebar menu's copy does (Q3-57): a copy result is shown where it was asked for, never
     * in the channel's connection bar.
     */
    copiedUsername: 'Copied',
    copyUsernameFailed: 'Couldn’t copy',
    announceCopiedUsername: (username: string) => `Copied @${username}`,
    announceCopyUsernameFailed: (username: string) =>
      `Couldn’t copy. Your username is @${username}.`,
  },

  privacy: {
    /** The one-line summary. `institution` is already an institution label. */
    private: (workspace: string, institution: string | null) =>
      institution
        ? `Only private and ${institution}-approved models can read ${workspace}.`
        : `Only private models can read ${workspace}.`,
    public: (workspace: string) =>
      `Public models can read public-safe channels in ${workspace}. Restricted channels stay private.`,
    rows: {
      connection: 'Your connection',
      workspace: 'Workspace',
      institution: 'Institution',
    },
    values: {
      private: 'Private',
      public: 'Public',
      workspacePrivate: 'Private for everyone',
      workspacePublic: 'Allows Public',
      notSet: 'Not set',
    },
    /**
     * Why the mode is what it is, first in the popover's one note (Q2-44). `workspace` is the
     * switcher's name.
     */
    why: {
      workspace: (workspace: string) => `Private because ${workspace} is Private for everyone.`,
      connection: () => 'Private because your connection is Private.',
      both: (workspace: string) =>
        `Private because both your connection and ${workspace} are Private.`,
      public: (workspace: string) =>
        `Public because your connection is Public and ${workspace} allows it.`,
    },
    /**
     * Who can see the workspace at all (Q2-44): "Private" is about models, never about people.
     * `host` is `personLabel(…, 'inline')`; null when the viewer is the host.
     */
    audience: (workspace: string, host: string | null) =>
      host
        ? `Only people ${host} lets in can see ${workspace}.`
        : `Only people you let in can see ${workspace}.`,
    /** Names what it changes (T-38): only this person's connection, never the workspace. */
    makePublic: 'Make my connection public…',
    /**
     * What the downgrade changes, as its description (T-38, Q2-44). Checked against the broker: a
     * Public connection keeps every channel it can see; only which models may read it changes
     * (`privacy_denied` refuses a public model a Restricted channel, never a person).
     */
    makePublicEffect: (workspace: string, workspaceMode: 'private' | 'public') =>
      workspaceMode === 'private'
        ? `Makes only your connection Public. ${workspace} is Private for everyone, so the models that can read it stay the same.`
        : `Makes only your connection Public: public models could then read the public-safe channels you can see in ${workspace}. Restricted channels stay private.`,
    makePrivate: 'Make private',
    more: 'Privacy…',
    /** After the why, for a member (Q2-44). */
    hostOnly: (workspace: string) => `Only the host can change ${workspace}.`,
    /** The popover's name is its title: `Privacy: Private · ucsf` (T-38). */
    titlePrefix: 'Privacy:',
    /** The inline step "Make private" takes when the connection has no institution yet. */
    institutionField: 'Institution',
    /** Pinned (the same placeholder Connection settings uses). */
    institutionPlaceholder: 'For example, ucsf or sdsc',
    institutionHelp: 'Your organization’s short ID, as your host uses it.',
    cancel: 'Cancel',
  },

  clipboard: {
    copied: 'Copied',
    failed: 'Couldn’t copy. Try again.',
  },
} as const;
