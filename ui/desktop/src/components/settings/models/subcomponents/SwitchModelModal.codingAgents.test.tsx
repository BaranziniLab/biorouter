import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { SwitchModelModal, configureProvidersReturn } from './SwitchModelModal';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  upsert: vi.fn(),
  changeModel: vi.fn(),
  status: vi.fn(),
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
    read: mocks.read,
    upsert: mocks.upsert,
  }),
}));

vi.mock('../../../ModelAndProviderContext', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  useModelAndProvider: () => ({
    changeModel: mocks.changeModel,
    currentModel: 'gpt-5.5-2026-04-24',
    currentProvider: 'versa_azure',
  }),
}));

vi.mock('../../../onboarding/codingAgentControls', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  codingAgentStatusOnce: mocks.status,
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

function row(
  name: string,
  displayName: string,
  configured: boolean,
  tier: 'public' | 'private'
): ProviderDetails {
  return {
    name,
    is_configured: configured,
    unavailable_reason: null,
    provider_type: 'Builtin',
    affiliation: null,
    metadata: {
      config_keys: [],
      default_model: '',
      description: '',
      display_name: displayName,
      known_models: [{ name: `${name}-model`, context_limit: 1000 }],
      model_doc_link: '',
      name,
      tier,
      runs_locally: false,
    },
  } as unknown as ProviderDetails;
}

function agent(providerId: string, state: string) {
  return {
    kind: providerId,
    providerId,
    displayName: providerId,
    path: `/opt/homebrew/bin/${providerId}`,
    version: '1.0',
    auth: { state, plan: null, account: null },
    loginCommand: `${providerId} login`,
    installHint: '',
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviders.mockResolvedValue([
    row('versa_azure', 'Versa API Azure', true, 'private'),
    row('codex', 'Codex', false, 'public'),
    row('claude_code', 'Claude Code', false, 'public'),
  ]);
  mocks.getProviderModels.mockResolvedValue([]);
  mocks.read.mockResolvedValue('');
  mocks.upsert.mockResolvedValue(undefined);
  mocks.changeModel.mockResolvedValue(true);
  mocks.status.mockResolvedValue({
    agents: [agent('codex', 'signed_in_subscription'), agent('claude_code', 'signed_out')],
  });
});

async function openProviderMenu() {
  const combo = (await screen.findAllByRole('combobox'))[0];
  fireEvent.keyDown(combo, { key: 'ArrowDown', code: 'ArrowDown' });
}

// W2-PRV-5: a signed-in Codex showed "Ready" in the catalog but was missing from
// a chat's picker, because the daemon reports it unconfigured until "Use Codex"
// saves its command key.
describe('SwitchModelModal and signed-in coding agents', () => {
  it('lists a signed-in agent that was never connected, and not a signed-out one', async () => {
    render(
      <SwitchModelModal sessionId="s1" privacyTier="public" onClose={vi.fn()} setView={vi.fn()} />
    );
    await waitFor(() => expect(mocks.status).toHaveBeenCalledTimes(1));
    await openProviderMenu();

    expect(await screen.findByRole('option', { name: 'Codex' })).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'Claude Code' })).toBeNull();
    // "Use other provider" stays last.
    const options = screen.getAllByRole('option').map((o) => o.textContent);
    expect(options[options.length - 1]).toBe('Use other provider');
  });

  it('saves the agent’s command key before switching the chat to it', async () => {
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="codex"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );
    await waitFor(() => expect(mocks.status).toHaveBeenCalledTimes(1));
    const confirm = screen.getByRole('button', { name: 'Select model' });
    await waitFor(() => expect(confirm).toBeEnabled());
    fireEvent.click(confirm);

    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
    expect(mocks.upsert).toHaveBeenCalledWith('CODEX_COMMAND', 'codex', false);
    expect(mocks.upsert.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.changeModel.mock.invocationCallOrder[0]
    );
    expect(mocks.changeModel).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ provider: 'codex', name: 'codex-model' }),
      expect.objectContaining({ alsoForNewChats: false })
    );
    // Never the app-wide provider: that is changeModel's to write, by scope.
    expect(mocks.upsert).not.toHaveBeenCalledWith('BIOROUTER_PROVIDER', expect.anything(), false);
  });

  it('asks nothing of the CLIs when every agent is already set up', async () => {
    mocks.getProviders.mockResolvedValue([
      row('versa_azure', 'Versa API Azure', true, 'private'),
      row('codex', 'Codex', true, 'public'),
    ]);
    render(
      <SwitchModelModal sessionId="s1" privacyTier="public" onClose={vi.fn()} setView={vi.fn()} />
    );
    await screen.findAllByRole('combobox');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(mocks.status).not.toHaveBeenCalled();
  });
});

describe('configureProvidersReturn', () => {
  const original = window.location.hash;
  afterEach(() => {
    window.location.hash = original;
  });

  it('carries the chat, its tier and the screen to come back to', () => {
    window.location.hash = '#/pair?resumeSessionId=s-9';
    expect(configureProvidersReturn('s-9', 'private')).toEqual({
      returnTo: '/pair?resumeSessionId=s-9',
      resumeSessionId: 's-9',
      privacyTier: 'private',
    });
  });

  it('carries only the way back when there is no chat', () => {
    window.location.hash = '#/';
    expect(configureProvidersReturn(null, undefined)).toEqual({ returnTo: '/' });
  });
});
