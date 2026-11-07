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
} as const;
