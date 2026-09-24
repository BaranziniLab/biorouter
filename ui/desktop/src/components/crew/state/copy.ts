/**
 * Strings the controller produces, and pinned strings more than one Crew area shows.
 *
 * Tests import these instead of retyping them. A string marked pinned is asserted by a regression
 * test or cited by acceptance evidence; change it only together with that test.
 *
 * The observation sentences are written for a person (live QA round 1, T-07): no daemon wording, no
 * "observation", "cursor" or "policy", and nothing about the draft unless the composer held
 * something. Every key other areas import is kept; only values change.
 */
export const crewStatusCopy = {
  /** Pinned: the verified status (the status row's `sr-only` text in the new layout). */
  verified: 'Connected · identity verified',
  /** Pinned. */
  checking: 'Checking connection',
  /** Pinned. */
  updatesUnavailable: 'Updates unavailable',
  connected: 'Connected',
  connecting: 'Connecting…',
  /** A verified workspace is being verified again after its live updates ended (re-observation). */
  updating: 'Updating…',
  /**
   * Live updates ended as a dropped connection would, and Crew is checking the connection and
   * picking the view up again over the bridge the daemon kept or dialled again. Ranked above
   * "Offline" and "Updates unavailable" (live QA round 2, Q2-01).
   */
  reconnecting: 'Reconnecting…',
  signInNeeded: 'Sign-in needed',
  cantVerify: 'Can’t verify server',
  notSetUp: 'Not set up on this server',
  notJoined: 'Not joined yet',
  offline: 'Offline',
  cantConnect: 'Can’t connect',
} as const;

export const crewObservationCopy = {
  /** Pinned fragment "unsent draft is retained". Said only when the composer holds something. */
  draftRetained: 'Your unsent draft is retained.',
  /**
   * Said only when something was cleared. `crewSurfaces.test.ts` pins its opening words ("Access or
   * privacy changed").
   */
  draftCleared: 'Access or privacy changed, so your unsent draft was cleared.',
  /**
   * Pinned fragment "privacy or selected channel access changed". Reported only when a material
   * change (privacy, institution, or the channels the draft was written for) cleared a non-empty
   * draft; a workspace policy epoch moving on its own never says it.
   */
  scopeChanged:
    'Workspace privacy or selected channel access changed, so your unsent draft was cleared.',
  channelAccessLost: 'You no longer have access to that channel, so it was closed.',
  /** `channel` is `#name`. */
  channelAccessLostNamed: (channel: string) =>
    `You no longer have access to ${channel}, so it was closed.`,
  /** Added to a lost-channel sentence only when the composer held something. */
  draftDiscarded: 'Your unsent draft for it was cleared.',
  wrongConnection: 'Crew sent updates for a different workspace, so they weren’t shown.',
  repeatedlyEnded: 'Live updates keep stopping. Check that Crew is running, then retry.',
  observationFailed: 'Live updates stopped.',
  historyFailed: 'Earlier messages couldn’t be loaded.',
  connectionsRefreshFailed: 'Crew couldn’t load your saved workspaces.',

  // Plain words for a terminal observation frame, by its code. The daemon's own sentence ("Room
  // observation ended…") is never shown. `workspace` is the workspace's display name.
  /** Updates stopped and retrying by itself did not bring them back (or it could not retry). */
  updatesStopped: (workspace: string) => `Live updates for ${workspace} stopped.`,
  /** `channel_access_changed`, before a fresh view says which channel. */
  channelAccessChanged: (channel: string) => `You no longer have access to ${channel}.`,
  /** `access_denied`, `forbidden`, `privacy_denied`, `principal_revoked`. */
  accessChanged: (workspace: string) => `Your access to ${workspace} changed.`,
  /** `unauthorized` / `unknown_device`: the workspace does not know this computer (yet). */
  unknownComputer: (workspace: string) => `${workspace} doesn’t recognize this computer yet.`,
  /**
   * `unauthorized` / `unknown_device` on a connection verified in this app session: the computer
   * was known and no longer is, so "yet" and Retry would invite waiting (Q2-18).
   */
  removedHere: (workspace: string) =>
    `${workspace} doesn’t recognize this computer any more. If you didn’t expect that, ask the host.`,
  /** `principal_revoked`: the person was removed from the workspace (Q2-18). */
  noLongerMember: (workspace: string) => `You’re no longer a member of ${workspace}.`,
  /** `human_authority_required`. */
  notConfirmed: (workspace: string) =>
    `Live updates for ${workspace} stopped because Crew couldn’t confirm the request came from you.`,
  /** `observer_capacity_reached`. */
  tooManyViews: (workspace: string) =>
    `Too many Crew windows are showing live updates for ${workspace}. Close one, then retry.`,
  /** `response_too_large`. */
  updateTooLarge: (workspace: string) => `An update from ${workspace} was too large to show.`,
} as const;

export const crewActionCopy = {
  actionFallback: 'Crew could not complete that action.',
  sendPrivacyUnverified: 'Refresh the workspace to verify connection privacy before sending.',
  grantPrivacyUnverified:
    'Refresh the workspace to verify connection privacy before granting agent access.',
  sendTransferRecordKept:
    'Message posted. Local transfer metadata could not be cleared; forget the completed record in saved transfers.',
  unknownOutcomeGate:
    'Inspect the previous task conversations and remote effects, then acknowledge the inspection before starting another task.',
  restartNeedsInspection: 'Confirm that you inspected the previous task before starting a new one.',
} as const;
