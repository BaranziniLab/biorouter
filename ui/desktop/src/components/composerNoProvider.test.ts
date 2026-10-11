import { describe, expect, it } from 'vitest';
import {
  NO_MODEL_CHIP_LABEL,
  NO_MODEL_COMPOSER_ACTION,
  NO_MODEL_COMPOSER_HINT,
  hasNoModelConfigured,
} from './composerNoProvider';

/**
 * ⚠ **The whole point of this function is the `loading` arm.**
 * `currentProvider` is `null` until the config has been read, so the obvious
 * implementation — `!provider` — announces "no model yet" over every correctly
 * configured install for the first frames after launch, and disables Send
 * there. Nothing about the rendered composer would look wrong in a screenshot
 * taken a moment later, which is why this is pinned as arithmetic rather than
 * left to a component test.
 */
describe('hasNoModelConfigured', () => {
  it('is true only once the config has been read and named no provider', () => {
    expect(hasNoModelConfigured('ready', null)).toBe(true);
    expect(hasNoModelConfigured('ready', '')).toBe(true);
  });

  it('is false while the config is still loading, whatever the provider looks like', () => {
    expect(hasNoModelConfigured('loading', null)).toBe(false);
    expect(hasNoModelConfigured('loading', '')).toBe(false);
    expect(hasNoModelConfigured('loading', 'versa_azure')).toBe(false);
  });

  it('is false for a configured provider', () => {
    expect(hasNoModelConfigured('ready', 'versa_azure')).toBe(false);
  });

  /**
   * Every existing suite that mocks `useModelAndProvider` omits the status
   * field. Answering `false` there means those composers keep working exactly as
   * they did — and, more importantly, that an unknown status never becomes a
   * claim that there is no model.
   */
  it('says nothing at all when the status is unknown', () => {
    expect(hasNoModelConfigured(undefined, null)).toBe(false);
    expect(hasNoModelConfigured(undefined, 'versa_azure')).toBe(false);
  });
});

/**
 * One missing step gets one noun (FR-R2-9). The composer card shows the hint's link and the model
 * chip together, and both open the same catalog, so they cannot name the move two ways.
 */
describe('the no-model words', () => {
  it('names the next move the same way on the hint link and the chip', () => {
    expect(NO_MODEL_COMPOSER_ACTION).toBe(NO_MODEL_CHIP_LABEL);
    expect(NO_MODEL_CHIP_LABEL).toBe('Choose a model');
  });

  it('states the problem in the hint and keeps the remedy out of it', () => {
    expect(NO_MODEL_COMPOSER_HINT).toBe('No model selected');
    expect(NO_MODEL_COMPOSER_HINT).not.toContain(NO_MODEL_COMPOSER_ACTION);
  });
});
