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
import type { DecisionInstallStatus } from '../../../../utils/sessionDecisions';
import { SwitchModelModal, groupProviderOptions } from './SwitchModelModal';
import { switchModelCopy } from '../copy';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
  install: vi.fn(),
  startInstall: vi.fn(),
}));

vi.mock('../../../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../../../utils/sessionDecisions')>()),
  fetchDecisionInstall: mocks.install,
  startDecisionInstall: mocks.startInstall,
}));

/** `GET /decisions/models/install` with Ollama running here, and tev1:0.8b installed or not. */
function installStatus(installed: boolean): { data: DecisionInstallStatus; missing: boolean } {
  return {
    data: {
      offers: [
        {
          model: 'tev1:0.8b',
          displayName: 'tev1:0.8b',
          approxSize: 'about 1.5 GB',
          note: 'small and fast',
          isDefault: true,
          installed,
        },
      ],
      ollama: {
        baseUrl: 'http://127.0.0.1:11434/',
        state: 'running',
        version: null,
        message: null,
      },
      job: null,
    },
    missing: false,
  };
}

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
  mocks.install.mockResolvedValue(installStatus(true));
});

async function openProviderMenu() {
  await screen.findByText('gpt-5.5-2026-04-24');
  fireEvent.keyDown(screen.getAllByRole('combobox')[0], { key: 'ArrowDown', code: 'ArrowDown' });
}

/** The provider menu, opened again once another provider is chosen. */
async function openProviderMenuAgain() {
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
    // Both carry the decision mark, the glyph every decision surface draws.
    for (const row of [ollama, baseten]) {
      expect(within(row).getByTestId('decision-provider-mark')).toHaveClass('lucide-gauge');
    }
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

  /**
   * Ollama's decision models with none on this computer: picking one would bind a
   * model that answers nothing, so the install is offered beside the list.
   */
  it('an empty decision group shows the install panel', async () => {
    mocks.getProviders.mockResolvedValue([
      ROWS[0],
      provider('ollama_decision', 'Ollama decision models', 'private'),
    ]);
    mocks.getProviderModels.mockImplementation(async (name: string) =>
      name === 'ollama_decision' ? [] : ['gpt-5.5-2026-04-24']
    );
    mocks.install.mockResolvedValue(installStatus(false));
    render(
      <SwitchModelModal sessionId="s1" privacyTier="private" onClose={vi.fn()} setView={vi.fn()} />
    );
    await openProviderMenu();
    fireEvent.click(await screen.findByRole('option', { name: /Ollama decision models/ }));
    const panel = await screen.findByTestId('decision-model-install');
    expect(panel).toHaveTextContent(
      'This chat is private, so only a decision model on this computer may read it, and none is installed.'
    );
    mocks.startInstall.mockResolvedValue({
      ...installStatus(false).data,
      job: {
        model: 'tev1:0.8b',
        state: 'installed',
        completedBytes: 1,
        totalBytes: 1,
        status: 'success',
        error: null,
      },
    });
    const calls = mocks.getProviderModels.mock.calls.length;
    fireEvent.click(within(panel).getByRole('button', { name: 'Install tev1:0.8b' }));
    expect(mocks.startInstall).toHaveBeenCalledWith('tev1:0.8b');
    expect(await screen.findByText('tev1:0.8b is installed.')).toBeInTheDocument();
    // The list is read again, with the model just installed in it.
    await waitFor(() => expect(mocks.getProviderModels.mock.calls.length).toBeGreaterThan(calls));
  });

  it('says nothing about installing when the decision model is there, or for Baseten', async () => {
    render(<SwitchModelModal sessionId="s1" onClose={vi.fn()} setView={vi.fn()} />);
    await openProviderMenu();
    fireEvent.click(await screen.findByRole('option', { name: /Ollama decision models/ }));
    expect(await screen.findByText('tev1:0.8b')).toBeInTheDocument();
    await waitFor(() => expect(mocks.install).toHaveBeenCalled());
    expect(screen.queryByTestId('decision-model-install')).toBeNull();
    mocks.install.mockClear();

    await openProviderMenuAgain();
    fireEvent.click(await screen.findByRole('option', { name: /Baseten decision models/ }));
    await screen.findByText('decision-models/joey-1');
    expect(screen.queryByTestId('decision-model-install')).toBeNull();
    expect(mocks.install).not.toHaveBeenCalled();
  });

  // R1-9: Ollama's row said "On this machine" and a sentence about the least
  // restricted affiliation; Baseten's said nothing. One plain line each now.
  it('says where each decision provider runs, in one plain line', async () => {
    mocks.getProviders.mockResolvedValue([
      ROWS[0],
      {
        ...provider('ollama_decision', 'Ollama decision models', 'private', ['tev1:0.8b']),
        affiliation: { kind: 'local', institutions: [] },
      },
      ROWS[2],
    ]);
    render(<SwitchModelModal sessionId="s1" onClose={vi.fn()} setView={vi.fn()} />);
    await openProviderMenu();
    fireEvent.click(await screen.findByRole('option', { name: /Ollama decision models/ }));
    expect(await screen.findByTestId('switch-model-decision-place')).toHaveTextContent(
      'Runs on this computer.'
    );
    expect(screen.queryByTestId('switch-model-affiliation')).toBeNull();
    expect(screen.queryByText(/least restricted affiliation/)).toBeNull();

    await openProviderMenuAgain();
    fireEvent.click(await screen.findByRole('option', { name: /Baseten decision models/ }));
    await screen.findByText('decision-models/joey-1');
    expect(screen.getByTestId('switch-model-decision-place')).toHaveTextContent('Runs on Baseten.');
    expect(screen.queryByTestId('switch-model-affiliation')).toBeNull();
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
