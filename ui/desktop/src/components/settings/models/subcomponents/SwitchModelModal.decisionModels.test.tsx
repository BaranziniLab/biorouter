/**
 * Decision models in the model picker: the providers whose models answer with a
 * choice and its probabilities (`baseten_decision`, `ollama_decision`) are listed
 * under their own "Decision models" heading, apart from the chat models, with
 * their resolved tier as a word and, when private, the padlock. Choosing one binds
 * the chat like any model.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { SwitchModelModal, groupProviderOptions } from './SwitchModelModal';
import { switchModelCopy } from '../copy';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
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
    currentModel: null,
    currentProvider: 'versa_azure',
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

function provider(
  name: string,
  displayName: string,
  tier: 'public' | 'private',
  knownModels: string[] = []
): ProviderDetails {
  return {
    name,
    provider_type: 'Builtin',
    affiliation: null,
    resolved_tier: tier,
    unavailable_reason: null,
    is_configured: true,
    metadata: {
      config_keys: [],
      default_model: knownModels[0] ?? '',
      description: '',
      display_name: displayName,
      known_models: knownModels.map((model) => ({ name: model, context_limit: 16384 })),
      model_doc_link: '',
      name,
      tier,
      runs_locally: false,
    },
  } as unknown as ProviderDetails;
}

const ROWS = [
  provider('versa_azure', 'Versa API Azure', 'private'),
  provider('ollama_decision', 'Ollama decision models', 'private', ['tev1:0.8b']),
  provider('baseten_decision', 'Baseten decision models', 'public', ['decision-models/joey-1']),
  provider('openai', 'OpenAI', 'public'),
];

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviders.mockResolvedValue(ROWS);
  mocks.getProviderModels.mockResolvedValue(['gpt-5.5-2026-04-24']);
  mocks.read.mockResolvedValue('');
  mocks.changeModel.mockResolvedValue(true);
});

async function openProviderMenu() {
  await screen.findByText('gpt-5.5-2026-04-24');
  fireEvent.keyDown(screen.getAllByRole('combobox')[0], { key: 'ArrowDown', code: 'ArrowDown' });
}

describe('decision models in the model picker', () => {
  it('groups decision providers under their own heading, after the chat providers', () => {
    const options = [
      { value: 'versa_azure', label: 'Versa' },
      {
        value: 'ollama_decision',
        label: 'Ollama decision',
        decision: { tier: 'private' as const },
      },
      { value: 'openai', label: 'OpenAI' },
      { value: 'configure_providers', label: 'Use other provider' },
    ];
    expect(groupProviderOptions(options)).toEqual([
      { options: [options[0], options[2]] },
      { label: switchModelCopy.decisionGroup, options: [options[1]] },
      { options: [options[3]] },
    ]);
    // With no decision provider the menu stays one flat list.
    expect(groupProviderOptions([options[0], options[3]])).toEqual([options[0], options[3]]);
  });

  it('lists them under "Decision models" with their tier and what they do', async () => {
    render(<SwitchModelModal sessionId="s1" onClose={vi.fn()} setView={vi.fn()} />);
    await openProviderMenu();

    const heading = await screen.findByText(switchModelCopy.decisionGroup);
    const group = heading.parentElement!;
    const ollama = within(group).getByRole('option', { name: /Ollama decision models/ });
    const baseten = within(group).getByRole('option', { name: /Baseten decision models/ });
    expect(ollama).toHaveTextContent(
      `${switchModelCopy.decisionPrivate} · ${switchModelCopy.decisionDetail}`
    );
    expect(within(ollama).getByTestId('decision-provider-lock')).toBeInTheDocument();
    expect(baseten).toHaveTextContent(
      `${switchModelCopy.decisionPublic} · ${switchModelCopy.decisionDetail}`
    );
    expect(within(baseten).queryByTestId('decision-provider-lock')).toBeNull();
    // The chat providers are not in that group.
    expect(within(group).queryByRole('option', { name: /Versa API Azure/ })).toBeNull();
    expect(screen.getByRole('option', { name: /Versa API Azure/ })).toBeInTheDocument();
  });

  it('binds the chat to a decision model like any model', async () => {
    render(<SwitchModelModal sessionId="s1" onClose={vi.fn()} setView={vi.fn()} />);
    await openProviderMenu();
    fireEvent.click(await screen.findByRole('option', { name: /Ollama decision models/ }));
    expect(await screen.findByText('tev1:0.8b')).toBeInTheDocument();
    const confirm = screen.getByRole('button', { name: switchModelCopy.select });
    await waitFor(() => expect(confirm).toBeEnabled());
    fireEvent.click(confirm);
    await waitFor(() =>
      expect(mocks.changeModel).toHaveBeenCalledWith(
        's1',
        expect.objectContaining({ name: 'tev1:0.8b', provider: 'ollama_decision' }),
        expect.anything()
      )
    );
  });

  it('keeps a public decision model out of a private chat, before it is chosen', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="private" onClose={vi.fn()} setView={vi.fn()} />
    );
    await openProviderMenu();
    fireEvent.click(await screen.findByRole('option', { name: /Baseten decision models/ }));
    // Its model fills the field, and the switch is barred with the reason on screen.
    await screen.findByText('decision-models/joey-1');
    expect(screen.getByRole('button', { name: switchModelCopy.select })).toBeDisabled();
    expect(screen.getByText(/private chat, so only private models/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: switchModelCopy.select }));
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });
});
