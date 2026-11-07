/**
 * Fixed words for the vault card (the owner's D1 ruling on AO-12). Nothing here
 * is the model's: the card's only values are the daemon's action and the main
 * process's own failure sentences.
 */
export const crewVaultStepCopy = {
  unlockTitle: "Unlock Crew's vault",
  initTitle: "Set up Crew's vault",
  /** What Unlock does, and where the passphrase goes. */
  unlockBody:
    "Crew keeps its keys in a vault on this computer, and it is locked. Unlock opens your computer's own passphrase window. Your passphrase goes from there to Crew, never into this chat.",
  /** What Set up does, and where the passphrase goes. */
  initBody:
    "Crew needs a vault on this computer for its keys. Set up opens your computer's own passphrase window, where you choose a passphrase and type it twice. It goes from there to Crew, never into this chat.",
  unlockButton: 'Unlock',
  initButton: 'Set up',
  notNow: 'Not now',
  /** While the passphrase window is open. */
  waiting: 'Finish in the passphrase window.',
  /** The person closed the passphrase window. */
  closedUnlock: 'You closed the passphrase window. The vault is still locked.',
  closedInit: 'You closed the passphrase window. Nothing was set up.',
  /** The daemon took the passphrase; the chat takes the card down and carries on. */
  unlocked: 'Unlocked. The chat carries on.',
  setUp: 'Set up and unlocked. The chat carries on.',
  /** A failure with no sentence of its own. */
  failedUnlock: "The vault didn't unlock. Try again.",
  failedInit: "The vault wasn't set up. Try again.",
  /** Where to do it when this app can't open the passphrase window (no desktop bridge). */
  unlockFallback:
    'Open Crew and unlock it under Keys, or run biorouter crew credentials unlock in a terminal. Never type your passphrase into the chat.',
  initFallback:
    'Open Crew and set it up under Keys, or run biorouter crew credentials init in a terminal. Never type your passphrase into the chat.',
} as const;
