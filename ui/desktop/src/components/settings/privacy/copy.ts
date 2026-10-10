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
   * ⚠ Wording kept from the paragraph it replaces (two clauses, see the comment at the row in
   * `PrivacyPanel.tsx`). Only its place changed: it is the row's InfoTip now.
   */
  help: 'Chats on private models stay private: a public model can’t read them and can’t call a private extension. A private model is one your institution hosts, or one that runs on this machine.',
} as const;
