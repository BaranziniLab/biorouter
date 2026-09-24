/**
 * Strings the controller produces, and pinned strings more than one Crew area shows.
 *
 * Tests import these instead of retyping them. A string marked pinned is asserted by a regression
 * test or cited by acceptance evidence; change it only together with that test. The controller's
 * sentences still carry the wording they had before the redesign: the copy deck's rewording of them
 * ("Error strings" in the ui-redesign spec) has not been adopted, and adopting it means editing
 * this file together with the tests that pin them.
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
  signInNeeded: 'Sign-in needed',
  cantVerify: 'Can’t verify server',
  notSetUp: 'Not set up on this server',
  notJoined: 'Not joined yet',
  offline: 'Offline',
  cantConnect: 'Can’t connect',
} as const;

export const crewObservationCopy = {
  /** Pinned fragment "unsent draft is retained". */
  draftRetained:
    'Your unsent draft is retained for this channel. Retry to verify access before sending.',
  draftCleared:
    'Access or privacy changed, so the unsent draft and attachments were cleared. Retry to verify access.',
  /** Pinned fragment "privacy or selected channel access changed". */
  scopeChanged:
    'Workspace privacy or selected channel access changed while reconnecting. The unsent draft and attachments were cleared; review the current policy before composing again.',
  channelAccessLost:
    'Access to the selected channel changed. Its messages and unsent composer content have been cleared; choose an authorized channel to continue.',
  wrongConnection:
    'The daemon returned a different Crew connection. Retry to verify the workspace.',
  repeatedlyEnded: 'The daemon repeatedly ended Crew observation. Retry after checking the daemon.',
  observationFailed: 'Crew observation failed.',
  historyFailed: 'Earlier messages could not be loaded.',
  connectionsRefreshFailed: 'Saved Crew connections could not be refreshed.',
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
