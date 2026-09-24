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
    'Use the Up and Down arrow keys to move between teams and channels. Left and Right collapse and expand a team.',
  /** The status row's `role="status"` name. */
  statusLabel: 'Connection status',

  switcher: {
    /** Before a connection is selected (the selection lands with the saved list). */
    loading: 'Loading workspaces…',
  },

  chip: {
    /** The chip's accessible name (the test anchor): `Privacy: Private · ucsf` / `Privacy: Public`. */
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
     * A joiner the host has not let in yet (T-06): the workspace cannot report their privacy
     * until they are a member, so "Checking…" would never resolve. Plain text, no padlock.
     */
    notJoined: 'Privacy shown after you join',
    notJoinedHint:
      'Privacy shown after you join: the workspace reports your connection’s privacy once the host lets you in.',
  },

  section: {
    /** Authored in sentence case; `text-caps` uppercases them. */
    invitations: 'Invitations',
    waiting: 'Waiting to join',
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
    createChannel: 'Create channel…',
    addPeople: (team: string) => `Add people to ${team}…`,
    rename: 'Rename team…',
    copyId: 'Copy team ID',
  },

  workspaceMenu: {
    hostedBy: 'Hosted by',
    /** `Signed in as @alice on hpc.ucsf.edu`: the person, then the server (T-40). */
    signedInAs: 'Signed in as',
    signedInOn: 'on',
    /** Before this connection's identity is verified there is no person to name, only a server. */
    server: 'Server',
    invite: (workspace: string) => `Invite people to ${workspace}…`,
    people: 'People…',
    privacy: 'Privacy…',
    /** The same name as Workspace settings' tab and the pane's Access heading. */
    access: 'Agent access…',
    createTeam: 'Create team…',
    /** Pinned. */
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
  },

  you: {
    /** A dev profile's badge, in the You menu's header (T-71): the row keeps its width for names. */
    devProfile: (name: string) => `Profile: ${name}`,
    editProfile: 'Edit profile…',
    keys: 'Keys and security…',
    copyUsername: 'Copy my username',
    /**
     * The item itself confirms the copy for a moment, and the menu stays open: a copy result is
     * shown where it was asked for, never in the channel's connection bar.
     */
    copiedUsername: 'Copied',
    copyUsernameFailed: 'Couldn’t copy',
    announceCopiedUsername: (username: string) => `Copied @${username}`,
    announceCopyUsernameFailed: (username: string) =>
      `Couldn’t copy. Your username is @${username}.`,
  },

  privacy: {
    /** The one-line explanation. `institution` is already an institution label. */
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
    why: {
      workspace: 'Private because the workspace is Private for everyone.',
      connection: 'Private because your connection is Private.',
      both: 'Private because your connection and the workspace are both Private.',
      public: 'Public because your connection is Public and the workspace allows it.',
    },
    /** Names what it changes (T-38): only this person's connection, never the workspace. */
    makePublic: 'Make my connection public…',
    /** The one line under it, saying what changes. `workspace` is the switcher's name. */
    makePublicEffect: (workspace: string, workspaceMode: 'private' | 'public') =>
      workspaceMode === 'private'
        ? `Changes only your connection. ${workspace} stays Private for everyone, so the models that can read it stay the same.`
        : `Changes only your connection: public models could then read public-safe channels in ${workspace}. You’ll confirm first.`,
    makePrivate: 'Make private',
    more: 'Privacy…',
    hostOnly: 'Only the host can change the workspace setting.',
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
