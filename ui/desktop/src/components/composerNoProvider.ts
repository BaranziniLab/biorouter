import type { ModelConfigStatus } from './ModelAndProviderContext';

/**
 * The composer's "no model selected" state: the honest other half of letting a
 * user into the app before they have configured a provider.
 *
 * ⚠ **The decision is here, not at either call site.** The chip and the send
 * guard must agree exactly: a chip that says "Choose a model" over a composer
 * that still lets you press Send produces the daemon's own error toast, written
 * for a developer, at the moment a first-run user is least equipped to read it.
 * That was the whole reason the first-run wall had no way past it.
 */

/** What the model chip reads when nothing is configured. */
export const NO_MODEL_CHIP_LABEL = 'Choose a model';

/**
 * The one line inside the composer card, and the link on the end of it.
 *
 * ⚠ **The link is the chip's own label, not a second spelling of it.** Both sit in the same card
 * and both open the same catalog, and one missing step used to read "Choose a provider" in the
 * hint and "Choose a model" on the chip: two nouns for one next move on the first screen after
 * leaving first run (FR-R2-9). Deriving it makes the drift impossible rather than merely tested.
 */
export const NO_MODEL_COMPOSER_HINT = 'No model selected';
export const NO_MODEL_COMPOSER_ACTION = NO_MODEL_CHIP_LABEL;

/**
 * Is there genuinely no model bound?
 *
 * ⚠ **`status === 'ready'` is load-bearing, and it is the whole function.**
 * `currentProvider` starts `null` and stays `null` until the config has been
 * read, so a check on the provider alone announces "no model yet" over every
 * perfectly configured install for the first frames after launch — and worse,
 * would disable Send there. `ModelConfigStatus`'s own doc comment says exactly
 * this; the type exists because the mistake is the natural one to make.
 *
 * ⚠ Also total over `undefined`, which is what every existing suite that mocks
 * `useModelAndProvider` without the field supplies. The direction is deliberate:
 * an unknown status means "say nothing and block nothing", never "claim there is
 * no model".
 */
export function hasNoModelConfigured(
  status: ModelConfigStatus | undefined,
  provider: string | null | undefined
): boolean {
  return status === 'ready' && !provider;
}
