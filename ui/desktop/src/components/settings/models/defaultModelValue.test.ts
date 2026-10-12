import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { ProviderDetails } from '../../../api';
import { joinModelValue, knownDefaultModelValue } from './defaultModelValue';

/**
 * The Model row's value, worked out from what the app already holds (V4-R7-1). It must say what
 * `getCurrentModelDisplayName` and `getCurrentProviderDisplayName` will say once the daemon has
 * answered, or the row would change its words a moment after it drew, and it must say nothing
 * rather than half a sentence when it cannot be sure.
 */

const listed = (name: string, display_name: string) =>
  ({ name, metadata: { display_name } }) as unknown as ProviderDetails;

const providers = [listed('versa_azure', 'Versa API Azure'), listed('anthropic', 'Anthropic')];

describe('knownDefaultModelValue', () => {
  let original: unknown;

  beforeEach(() => {
    original = (window as unknown as { appConfig?: unknown }).appConfig;
    (window as unknown as { appConfig: unknown }).appConfig = { get: () => undefined };
  });

  afterEach(() => {
    (window as unknown as { appConfig?: unknown }).appConfig = original;
  });

  it('is the model and the provider’s display name, joined with a middle dot', () => {
    expect(knownDefaultModelValue('gpt-5.5-2026-04-24', 'versa_azure', providers)).toBe(
      'gpt-5.5-2026-04-24 · Versa API Azure'
    );
  });

  it('is not known until there is a model and a provider in force', () => {
    expect(knownDefaultModelValue(null, 'versa_azure', providers)).toBeNull();
    expect(knownDefaultModelValue('gpt-5.5', null, providers)).toBeNull();
    expect(knownDefaultModelValue('', '', providers)).toBeNull();
  });

  it('is not known when the provider list is missing, empty or does not name the provider', () => {
    expect(knownDefaultModelValue('gpt-5.5', 'versa_azure', undefined)).toBeNull();
    expect(knownDefaultModelValue('gpt-5.5', 'versa_azure', [])).toBeNull();
    expect(knownDefaultModelValue('gpt-5.5', 'openai', providers)).toBeNull();
  });

  it('uses a pinned model’s alias and subtext, as the daemon-side getters do', () => {
    (window as unknown as { appConfig: unknown }).appConfig = {
      get: (key: string) =>
        key === 'BIOROUTER_PREDEFINED_MODELS'
          ? JSON.stringify([{ name: 'gpt-5.5', alias: 'GPT 5.5', subtext: 'UCSF Versa' }])
          : undefined,
    };
    // The pinned subtext wins over the list, so it is known even before the list is fetched.
    expect(knownDefaultModelValue('gpt-5.5', 'versa_azure', [])).toBe('GPT 5.5 · UCSF Versa');
  });
});

describe('joinModelValue', () => {
  it('drops an empty half instead of leaving a dangling dot', () => {
    expect(joinModelValue('Select Model', null)).toBe('Select Model');
    expect(joinModelValue('', 'Versa')).toBe('Versa');
    expect(joinModelValue('', null)).toBe('');
  });
});
