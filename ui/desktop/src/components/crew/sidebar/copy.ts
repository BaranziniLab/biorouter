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
    /** Shown, with no padlock, until the observer verifies this connection's privacy. */
    checking: 'Checking privacy…',
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
    accept: 'Accept',
    acceptLabel: (target: string) => `Accept invitation to ${target}`,
    /** `inviter` is `personLabel(…, 'inline')`. */
    acceptFromLabel: (inviter: string) => `Accept invitation from ${inviter}`,
  },

  waiting: {
    letIn: 'Let in…',
    letInLabel: (username: string) => `Let @${username} in`,
    approved: 'Approved',
    otherDevice: (username: string) =>
      `A device with a different code tried to join as @${username}.`,
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
    signedInAs: 'Signed in as',
    invite: (workspace: string) => `Invite people to ${workspace}…`,
    people: 'People…',
    privacy: 'Privacy…',
    access: 'Chats with access…',
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

  you: {
    /** A dev profile's badge. */
    devProfile: (name: string) => `Profile: ${name}`,
    editProfile: 'Edit profile…',
    keys: 'Keys and security…',
    copyUsername: 'Copy my username',
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
    makePublic: 'Make public…',
    makePrivate: 'Make private',
    more: 'Privacy…',
    hostOnly: 'Only the host can change the workspace setting.',
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
