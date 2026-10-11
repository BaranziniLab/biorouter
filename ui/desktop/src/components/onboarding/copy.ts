/**
 * The words of first run (codex simplicity redesign, R-34): the page's band and the setup panels a
 * provider row opens in place. Tests import these instead of retyping them.
 *
 * Help strings are InfoTip text: full sentences, at most two, about 160 characters. Labels and
 * buttons take no trailing period and no arrow. Toasts are sentence case with no exclamation mark.
 *
 * ⚠ Privacy copy is not here and never moves here: the tier notes (`providerOrdering.ts`), the
 * host-managed panel (`privacy/hostManagedModelCopy.ts`) and the non-private disclosure are their
 * own files, frozen by ruling.
 */

/** The first-run page's band (`ProviderGuard`). */
export const firstRunCopy = {
  /** The desktop's title: the same task, and the same words, as the `#/welcome` page. */
  title: 'Choose a provider',
  /** A browser served by `biorouter serve`, where the choice is the host's. */
  hostTitle: 'Get started',
  /** Into the app with no provider bound; the composer says why it cannot send. */
  skip: 'Explore Biorouter first',
  openCrew: 'Open Crew',
} as const;
