/**
 * Fixed words for a Crew step card (the owner's D1 ruling on AO-12). Card copy is
 * this fixed text plus the daemon's own values, each passed through
 * `sanitizeDisplayText`; nothing the model wrote ever reaches a card.
 */
export const crewStepCopy = {
  /** A step this app does not know how to draw: a newer daemon's. */
  unknown:
    'This chat needs a step only you can take in Crew. Open Crew to finish it. Never type a password, code or passphrase into the chat.',
  /** Said under every step: the card acts as you and only you can finish it. */
  actsAsYou: 'This acts as you. Only you can finish it.',
  /**
   * The one line a step draws where nothing waits on it: a saved or imported transcript, or a
   * chat whose turn has ended. It names nothing from the card, since the card's words there are
   * whatever the file says.
   */
  notWaiting: 'A Crew step waited here. It is not waiting any more, so there is nothing to do.',
  /**
   * What a step says once the daemon ended it while this window watched (WI-17), by how. A step
   * is never answered as done: the chat learns that from the daemon, so these never claim it.
   */
  ended: {
    answered: 'This step is over. The chat carries on.',
    dismissed: 'Skipped. The chat carries on without this step.',
    timed_out: 'This step timed out before it was taken. The chat carries on without it.',
    withdrawn: 'This step no longer waits for you. The chat carries on.',
  },
  /**
   * A changed host key's notice read back from a transcript: fixed words with no host and no
   * fingerprint, since there they would be whatever the file says.
   */
  changedKeyNotice:
    "A server's host key changed here, so Crew did not connect. Nothing connects to it until the server's IT team confirms the change.",
} as const;
