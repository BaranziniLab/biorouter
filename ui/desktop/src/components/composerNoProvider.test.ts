import { describe, expect, it } from 'vitest';
import { hasNoModelConfigured } from './composerNoProvider';

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
