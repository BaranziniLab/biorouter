/**
 * The Privacy switch row's words (Settings > App > Privacy, spec §3.13).
 *
 * Only the row lives here. The DR-17 disclosure above it is served by the daemon, and the
 * disable confirmation's sentences are locked in `PrivacyPanel.tsx` (vocabulary rule 9): their
 * words and placement do not move for a style change.
 */
export const privacyRowCopy = {
  /** Also the switch's accessible name; tests and the privacy-off note find it by this. */
  label: 'Privacy tiers',
  /**
   * ⚠ The two clauses of the paragraph it replaces (see the comment at the row in
   * `PrivacyPanel.tsx`), tightened to the InfoTip's two-sentence, ~160-character limit (owner,
   * 2026-11-13). It is the row's InfoTip.
   */
  help: 'Chats on private models stay private: public models can’t read them or call private extensions. A private model runs at your institution or on this machine.',
} as const;

/** The enforcement-off strip's words that are not the composer note's shared ones. */
export const privacyOffStripCopy = {
  /** The disclosure under the strip's state line that holds the consequence. */
  whatThisMeans: 'What this means',
} as const;
