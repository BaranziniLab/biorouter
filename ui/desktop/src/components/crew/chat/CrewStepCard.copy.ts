/**
 * Fixed words for a Crew step card (the owner's D1 ruling on AO-12). Card copy is
 * this fixed text plus the daemon's own values, each passed through
 * `sanitizeDisplayText`; nothing the model wrote ever reaches a card.
 *
 * A card shows one line a person needs and folds the rest behind an info icon (owner,
 * 2026-11-13, design.md 9.16). A warning about typing into the chat always stays in view.
 */
export const crewStepCopy = {
  /** A step this app does not know how to draw (a newer daemon's): its title. */
  unknownTitle: 'A Crew step needs you.',
  /** Behind the unknown step's info icon: "About this step". */
  unknownHelpLabel: 'this step',
  unknownHelp: 'This chat needs a step only you can take in Crew. Open Crew to finish it.',
  /** The unknown step's one line, always in view. */
  unknownWarning: 'Never type a password, code or passphrase into the chat.',
  /** Said under every step: the card acts as you and only you can finish it. */
  actsAsYou: 'This acts as you. Only you can finish it.',
  /**
   * The one line a step draws where nothing waits on it: a saved or imported transcript, or a
   * chat whose turn has ended. It names nothing from the card, since the card's words there are
   * whatever the file says.
   */
  notWaiting: 'This Crew step is over.',
  /**
   * What a step says once the daemon ended it while this window watched (WI-17), by how. A step
   * is never answered as done: the chat learns that from the daemon, so these never claim it.
   */
  ended: {
    answered: 'This step is over. The chat carries on.',
    dismissed: 'Skipped. The chat carries on.',
    timed_out: 'This step timed out. The chat carries on.',
    withdrawn: 'This step no longer waits for you. The chat carries on.',
  },
  /**
   * A changed host key's notice read back from a transcript: fixed words with no host and no
   * fingerprint, since there they would be whatever the file says. The refusal stays in view;
   * what it means for the server waits behind its info icon.
   */
  changedKeyNotice: "A server's host key changed here, so Crew did not connect.",
  changedKeyNoticeHelpLabel: 'the changed key',
  changedKeyNoticeHelp: "Nothing connects to it until the server's IT team confirms the change.",
} as const;
