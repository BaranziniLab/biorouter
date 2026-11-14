/**
 * Fixed words for the vault card (the owner's D1 ruling on AO-12). Nothing here
 * is the model's: the card's only values are the daemon's action and the main
 * process's own failure sentences.
 *
 * The card keeps one line in view, where the passphrase goes, and folds what the vault is and
 * what the button opens behind the title's info icon (owner, 2026-11-13, design.md 9.16).
 */
export const crewVaultStepCopy = {
  unlockTitle: "Unlock Crew's vault",
  initTitle: "Set up Crew's vault",
  /** Where the passphrase goes: the card's one line in view, for Unlock and Set up alike. */
  passphraseGoes:
    "Your passphrase goes from your computer's own passphrase window to Crew, never into this chat.",
  /** Behind the title's info icon: "About the vault". */
  aboutLabel: 'the vault',
  /** What Unlock does. */
  unlockAbout:
    "Crew keeps its keys in a vault on this computer, and it is locked. Unlock opens your computer's own passphrase window.",
  /** What Set up does. */
  initAbout:
    "Crew needs a vault on this computer for its keys. Set up opens your computer's own passphrase window, where you choose a passphrase and type it twice.",
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
  /**
   * Where this app can't open the passphrase window (no desktop bridge): the warning stays in
   * view, and where to do it instead sits under a disclosure.
   */
  fallbackWarning: 'Never type your passphrase into the chat.',
  unlockHowLabel: 'How do I unlock it?',
  unlockHow:
    'Open Crew and unlock it under Keys, or run biorouter crew credentials unlock in a terminal.',
  initHowLabel: 'How do I set it up?',
  initHow:
    'Open Crew and set it up under Keys, or run biorouter crew credentials init in a terminal.',
} as const;
