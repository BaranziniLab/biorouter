import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { SwitchModelModal } from './SwitchModelModal';

const OPUS_55 = 'us.anthropic.claude-opus-5-5';
const OPUS_48 = 'us.anthropic.claude-opus-4-8';
const MODELS = [
  OPUS_55,
  'us.anthropic.claude-sonnet-5-5',
  'us.anthropic.claude-haiku-5-5',
  OPUS_48,
];

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
  currentProvider: 'anthropic' as string,
  currentModel: null as string | null,
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
    read: mocks.read,
  }),
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    changeModel: mocks.changeModel,
    currentModel: mocks.currentModel,
    currentProvider: mocks.currentProvider,
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

function versaProvider(models = MODELS): ProviderDetails {
  return {
    name: 'versa_bedrock',
    provider_type: 'Builtin',
    is_configured: true,
    affiliation: null,
    resolved_tier: 'private',
    unavailable_reason: null,
    metadata: {
      name: 'versa_bedrock',
      display_name: 'Versa API Bedrock',
      description: '',
      default_model: OPUS_55,
      known_models: models.map((name) => ({ name, context_limit: 1_000_000 })),
      config_keys: [],
      model_doc_link: '',
      tier: 'private',
      runs_locally: false,
      allows_unlisted_models: false,
    },
  };
}

function openPicker(provider = 'versa_bedrock') {
  render(
    <SwitchModelModal
      sessionId={null}
      initialProvider={provider}
      onClose={vi.fn()}
      setView={vi.fn()}
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.currentProvider = 'anthropic';
  mocks.currentModel = null;
  mocks.getProviders.mockResolvedValue([versaProvider()]);
  mocks.getProviderModels.mockResolvedValue([OPUS_48]);
  mocks.read.mockResolvedValue('');
  mocks.changeModel.mockResolvedValue(true);
});

describe('SwitchModelModal provider defaults', () => {
  it('uses the Codex account default instead of the bundled default', async () => {
    const codex = versaProvider(['gpt-6-astra', 'gpt-6-sol']);
    codex.name = 'codex';
    codex.metadata.name = 'codex';
    codex.metadata.display_name = 'Codex';
    codex.metadata.default_model = 'gpt-6-astra';
    codex.metadata.tier = 'public';
    codex.resolved_tier = 'public';
    mocks.getProviders.mockResolvedValue([codex]);
    mocks.getProviderModels.mockResolvedValue(['gpt-6-sol']);
    openPicker('codex');

    await waitFor(() => expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled());
    expect(screen.getByText('GPT-6 Sol')).toBeInTheDocument();
    expect(mocks.getProviderModels).toHaveBeenCalledWith('codex');
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

    await waitFor(() =>
      expect(mocks.changeModel).toHaveBeenCalledWith(
        null,
        expect.objectContaining({ provider: 'codex', name: 'gpt-6-sol' }),
        expect.any(Object)
      )
    );
  });

  it('selects the declared default when it is available later in the provider catalogue', async () => {
    mocks.getProviders.mockResolvedValue([
      versaProvider([OPUS_48, ...MODELS.filter((name) => name !== OPUS_48)]),
    ]);
    openPicker();

    await waitFor(() => expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled());
    expect(screen.getByText('Claude Opus 5.5')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

    await waitFor(() =>
      expect(mocks.changeModel).toHaveBeenCalledWith(
        null,
        expect.objectContaining({ provider: 'versa_bedrock', name: OPUS_55 }),
        expect.any(Object)
      )
    );
  });

  it('starts fresh Versa setup on Opus 5.5 and offers all 5.5 models before 4.8', async () => {
    openPicker();

    await screen.findByText('Claude Opus 5.5');
    fireEvent.keyDown(screen.getAllByRole('combobox')[1], {
      key: 'ArrowDown',
      code: 'ArrowDown',
    });
    const options = await screen.findAllByRole('option');
    expect(options).toHaveLength(4);
    for (const [index, label] of [
      'Claude Opus 5.5',
      'Claude Sonnet 5.5',
      'Claude Haiku 5.5',
      'Claude Opus 4.8',
    ].entries()) {
      expect(options[index]).toHaveTextContent(label);
      expect(options[index]).toHaveTextContent(MODELS[index]);
    }
    expect(mocks.getProviderModels).not.toHaveBeenCalled();
    fireEvent.keyDown(screen.getAllByRole('combobox')[1], { key: 'Escape' });
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

    await waitFor(() =>
      expect(mocks.changeModel).toHaveBeenCalledWith(
        null,
        expect.objectContaining({ provider: 'versa_bedrock', name: OPUS_55 }),
        expect.any(Object)
      )
    );
  });

  it('preserves an explicitly selected Opus 4.8 when the same provider is reopened', async () => {
    mocks.currentProvider = 'versa_bedrock';
    mocks.currentModel = OPUS_48;
    openPicker();

    await screen.findByText('Claude Opus 4.8');
    await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

    await waitFor(() =>
      expect(mocks.changeModel).toHaveBeenCalledWith(
        null,
        expect.objectContaining({ provider: 'versa_bedrock', name: OPUS_48 }),
        expect.any(Object)
      )
    );
  });
});

function copilotProvider(): ProviderDetails {
  const provider = versaProvider(['gpt-5.3-codex', 'grok-4.7', 'claude-opus-4.8', 'gpt-5.6-terra']);
  provider.name = 'github_copilot';
  provider.metadata.name = 'github_copilot';
  provider.metadata.display_name = 'GitHub Copilot';
  provider.metadata.default_model = 'gpt-5.3-codex';
  provider.metadata.allows_unlisted_models = true;
  provider.metadata.tier = 'public';
  provider.resolved_tier = 'public';
  return provider;
}

describe('SwitchModelModal Copilot account defaults', () => {
  it('selects the eligible account daily model instead of the static LTS fallback', async () => {
    mocks.getProviders.mockResolvedValue([copilotProvider()]);
    mocks.getProviderModels.mockResolvedValue(['claude-sonnet-5.5', 'gpt-5.3-codex']);
    openPicker('github_copilot');

    await screen.findByText('Claude Sonnet 5.5');
    expect(mocks.getProviderModels).toHaveBeenCalledWith('github_copilot');
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));
    await waitFor(() =>
      expect(mocks.changeModel).toHaveBeenCalledWith(
        null,
        expect.objectContaining({ provider: 'github_copilot', name: 'claude-sonnet-5.5' }),
        expect.any(Object)
      )
    );
  });

  it.each(['claude-opus-4.8', 'gpt-5.6-terra'])(
    'preserves an explicit saved %s even when account discovery no longer suggests it',
    async (savedModel) => {
      mocks.currentProvider = 'github_copilot';
      mocks.currentModel = savedModel;
      mocks.getProviders.mockResolvedValue([copilotProvider()]);
      mocks.getProviderModels.mockResolvedValue(['claude-sonnet-5.5', 'gpt-5.3-codex']);
      openPicker('github_copilot');

      await waitFor(() => expect(mocks.getProviderModels).toHaveBeenCalledWith('github_copilot'));
      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled()
      );
      fireEvent.click(screen.getByRole('button', { name: 'Select model' }));
      await waitFor(() =>
        expect(mocks.changeModel).toHaveBeenCalledWith(
          null,
          expect.objectContaining({ provider: 'github_copilot', name: savedModel }),
          expect.any(Object)
        )
      );
    }
  );

  it('offers only the LTS fallback when discovery rejects despite broad capability metadata', async () => {
    mocks.getProviders.mockResolvedValue([copilotProvider()]);
    mocks.getProviderModels.mockRejectedValue(new Error('synthetic catalogue unavailable'));
    openPicker('github_copilot');

    await waitFor(() => expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled());
    fireEvent.keyDown(screen.getAllByRole('combobox')[1], { key: 'ArrowDown' });
    const options = await screen.findAllByRole('option');
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveTextContent('gpt-5.3-codex');
    expect(options[1]).toHaveTextContent('Enter a model not listed');
    expect(options.map((option) => option.textContent).join(' ')).not.toContain('gpt-5.6-terra');
    expect(options.map((option) => option.textContent).join(' ')).not.toContain('claude-opus-4.8');
    fireEvent.keyDown(screen.getAllByRole('combobox')[1], { key: 'Escape' });
  });

  it('does not select a named LTS model for a successful empty Auto-only account catalogue', async () => {
    mocks.getProviders.mockResolvedValue([copilotProvider()]);
    mocks.getProviderModels.mockResolvedValue([]);
    openPicker('github_copilot');

    await waitFor(() => expect(mocks.getProviderModels).toHaveBeenCalledWith('github_copilot'));
    expect(screen.getByRole('button', { name: 'Select model' })).toBeDisabled();
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });
});
