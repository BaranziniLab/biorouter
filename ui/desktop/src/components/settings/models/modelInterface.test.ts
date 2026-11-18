import { describe, expect, it, vi } from 'vitest';
import {
  fetchModelsForProviders,
  modelSupportedInputMimeTypes,
  modelSupportsVision,
  resolveKnownModelInfo,
} from './modelInterface';
import type { ProviderDetails, ProviderMetadata } from '../../../api';

const metadata: ProviderMetadata = {
  name: 'versa_azure',
  display_name: 'Versa API Azure',
  description: 'Test provider',
  default_model: 'gpt-5.5-2026-04-24',
  known_models: [
    { name: 'gpt-5.5', context_limit: 1_050_000, supports_vision: true },
    { name: 'gpt-5.3-codex', context_limit: 400_000 },
    { name: 'text-only', context_limit: 128_000 },
  ],
  model_doc_link: '',
  config_keys: [],
  allows_unlisted_models: true,
};

const claudeMetadata: ProviderMetadata = {
  name: 'claude_test',
  display_name: 'Claude Test',
  description: 'Test provider',
  default_model: 'claude-sonnet-4-6',
  known_models: [
    { name: 'claude-sonnet-4-6', context_limit: 1_000_000, supports_vision: true },
    {
      name: 'claude-opus-4-6',
      context_limit: 1_000_000,
      supports_vision: true,
      supported_input_mime_types: [
        'image/png',
        'image/jpeg',
        'image/jpg',
        'image/gif',
        'image/webp',
      ],
    },
    { name: 'claude-sonnet-4-20250514', context_limit: 200_000, supports_vision: true },
  ],
  model_doc_link: '',
  config_keys: [],
  allows_unlisted_models: true,
};

describe('model vision resolution', () => {
  it('matches exact known model metadata', () => {
    expect(modelSupportsVision(metadata, 'gpt-5.5')).toBe(true);
    expect(modelSupportsVision(metadata, 'text-only')).toBe(false);
  });

  it('keeps Copilot Sonnet 5.5 attachments through retained Sonnet 5 family metadata', () => {
    const copilotMetadata: ProviderMetadata = {
      ...metadata,
      name: 'github_copilot',
      known_models: [{ name: 'claude-sonnet-5', context_limit: 128_000, supports_vision: true }],
    };
    expect(resolveKnownModelInfo(copilotMetadata, 'claude-sonnet-5.5')?.name).toBe(
      'claude-sonnet-5'
    );
    expect(modelSupportsVision(copilotMetadata, 'claude-sonnet-5.5')).toBe(true);
    expect(modelSupportedInputMimeTypes(copilotMetadata, 'claude-sonnet-5.5')).toEqual([
      'image/png',
      'image/jpeg',
      'image/jpg',
      'image/gif',
      'image/webp',
    ]);
  });

  it('matches dated deployments to their vision-capable family', () => {
    expect(modelSupportsVision(metadata, 'gpt-5.5-2026-04-24')).toBe(true);
    expect(resolveKnownModelInfo(metadata, 'gpt-5.5-2026-04-24')?.name).toBe('gpt-5.5');
  });

  it('does not infer vision for codex variants', () => {
    expect(modelSupportsVision(metadata, 'gpt-5.3-codex-2026-01-01')).toBe(false);
  });

  it.each([
    ['anthropic/claude-sonnet-4.6', 'claude-sonnet-4-6'],
    ['anthropic/claude-sonnet-4-6', 'claude-sonnet-4-6'],
    ['databricks-claude-sonnet-4-6', 'claude-sonnet-4-6'],
    ['us.anthropic.claude-sonnet-4-6', 'claude-sonnet-4-6'],
    ['us.anthropic.claude-opus-4-6-v1', 'claude-opus-4-6'],
    ['us.anthropic.claude-opus-4-6-v1:0', 'claude-opus-4-6'],
    ['claude-sonnet-4@20250514', 'claude-sonnet-4-20250514'],
  ])('matches Claude alias %s to known model %s', (modelName, expectedKnownModel) => {
    expect(resolveKnownModelInfo(claudeMetadata, modelName)?.name).toBe(expectedKnownModel);
    expect(modelSupportsVision(claudeMetadata, modelName)).toBe(true);
  });

  it('does not overmatch similarly named Claude models', () => {
    expect(resolveKnownModelInfo(claudeMetadata, 'claude-sonnet-4-60')).toBeUndefined();
    expect(modelSupportsVision(claudeMetadata, 'claude-sonnet-4-60')).toBe(false);
  });

  it('resolves structured input MIME types through provider aliases', () => {
    expect(modelSupportedInputMimeTypes(claudeMetadata, 'us.anthropic.claude-opus-4-6-v1')).toEqual(
      ['image/png', 'image/jpeg', 'image/jpg', 'image/gif', 'image/webp']
    );
  });
});

describe('provider catalogue defaults', () => {
  function provider(
    models: string[],
    defaultModel = 'recommended',
    name = 'example'
  ): ProviderDetails {
    return {
      name,
      provider_type: 'Builtin',
      is_configured: true,
      metadata: {
        ...metadata,
        name,
        default_model: defaultModel,
        known_models: models.map((name) => ({ name, context_limit: 128_000 })),
      },
    } as ProviderDetails;
  }

  it('puts an available declared default first and preserves the remaining catalogue order', async () => {
    const getModels = vi.fn();
    const row = provider(['older', 'recommended', 'fast']);
    const [result] = await fetchModelsForProviders([row], getModels);

    expect(result.models).toEqual(['recommended', 'older', 'fast']);
    expect(row.metadata.known_models.map(({ name }) => name)).toEqual([
      'older',
      'recommended',
      'fast',
    ]);
    expect(getModels).not.toHaveBeenCalled();
  });

  it('keeps the available catalogue when its declared default is not listed', async () => {
    const [result] = await fetchModelsForProviders(
      [provider(['available', 'fast'], 'unavailable')],
      vi.fn()
    );

    expect(result.models).toEqual(['available', 'fast']);
  });

  it('preserves a live account list default ahead of the static metadata fallback', async () => {
    const getModels = vi.fn().mockResolvedValue(['account-default', 'recommended', 'fast']);
    const [result] = await fetchModelsForProviders([provider([])], getModels);

    expect(result.models).toEqual(['account-default', 'recommended', 'fast']);
    expect(getModels).toHaveBeenCalledWith('example');
  });

  it('uses the Copilot account catalogue and keeps its server default order without merging static IDs', async () => {
    const getModels = vi.fn().mockResolvedValue(['account-default', 'claude-sonnet-5.5']);
    const row = provider(
      ['gpt-5.3-codex', 'grok-4.7', 'claude-opus-4.8', 'gpt-5.6-terra'],
      'gpt-5.3-codex',
      'github_copilot'
    );
    const [result] = await fetchModelsForProviders([row], getModels);

    expect(getModels).toHaveBeenCalledWith('github_copilot');
    expect(result.models).toEqual(['account-default', 'claude-sonnet-5.5']);
    expect(row.metadata.default_model).toBe('gpt-5.3-codex');
    expect(row.metadata.known_models.map(({ name }) => name)).toEqual([
      'gpt-5.3-codex',
      'grok-4.7',
      'claude-opus-4.8',
      'gpt-5.6-terra',
    ]);
  });

  it('keeps an authoritative empty Copilot account catalogue empty for Auto-only accounts', async () => {
    const getModels = vi.fn().mockResolvedValue([]);
    const [result] = await fetchModelsForProviders(
      [
        provider(
          ['gpt-5.3-codex', 'grok-4.7', 'claude-opus-4.8', 'gpt-5.6-terra'],
          'gpt-5.3-codex',
          'github_copilot'
        ),
      ],
      getModels
    );

    expect(getModels).toHaveBeenCalledWith('github_copilot');
    expect(result.models).toEqual([]);
    expect(result.error).toBeNull();
  });

  it('uses only the declared Copilot LTS fallback when account discovery is unavailable', async () => {
    const getModels = vi.fn().mockRejectedValue(new Error('synthetic catalogue unavailable'));
    const [result] = await fetchModelsForProviders(
      [
        provider(
          ['gpt-5.3-codex', 'grok-4.7', 'claude-opus-4.8', 'gpt-5.6-terra'],
          'gpt-5.3-codex',
          'github_copilot'
        ),
      ],
      getModels
    );

    expect(getModels).toHaveBeenCalledWith('github_copilot');
    expect(result.models).toEqual(['gpt-5.3-codex']);
    expect(result.error).toBeNull();
  });

  it('prefers the Codex account catalogue and its default over the bundled fallback', async () => {
    const getModels = vi.fn().mockResolvedValue(['account-default', 'account-other']);
    const [result] = await fetchModelsForProviders(
      [provider(['bundled', 'bundled-other'], 'bundled', 'codex')],
      getModels
    );

    expect(result.models).toEqual(['account-default', 'account-other']);
    expect(getModels).toHaveBeenCalledWith('codex');
  });

  it.each([
    ['unavailable', () => Promise.reject(new Error('CLI unavailable'))],
    ['empty', () => Promise.resolve([])],
  ])('uses the declared Codex fallback when its account catalogue is %s', async (_, response) => {
    const getModels = vi.fn().mockImplementation(response);
    const [result] = await fetchModelsForProviders(
      [provider(['older', 'bundled'], 'bundled', 'codex')],
      getModels
    );

    expect(getModels).toHaveBeenCalledWith('codex');
    expect(result.models).toEqual(['bundled', 'older']);
    expect(result.error).toBeNull();
  });
});
