import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { announceAppModelSelection } from '../../../../utils/sessionBindingSync';
import { defaultWorkerModelCopy } from '../copy';
import {
  DefaultWorkerModelRow,
  parseWorkerChoice,
  WORKER_MODEL_KEY,
  WORKER_PROVIDER_KEY,
  workerProviderUsable,
} from './DefaultWorkerModelRow';

/**
 * Settings › Models › Default model › Default worker model: the model a subagent runs on when the
 * agent names none, stored as `BIOROUTER_WORKER_PROVIDER` and `BIOROUTER_WORKER_MODEL`. It
 * replaced lead/worker mode, and "Same as the chat’s model" (both keys unset) is the default.
 */

const mocks = vi.hoisted(() => ({
  values: {} as Record<string, unknown>,
  providers: [] as unknown[],
  writes: [] as string[],
  upsert: vi.fn(),
  remove: vi.fn(),
  getProviderModels: vi.fn(),
}));

vi.mock('../../../ConfigContext', () => {
  const api = {
    read: async (key: string) => mocks.values[key] ?? null,
    getProviders: async () => mocks.providers,
    getProviderModels: mocks.getProviderModels,
    upsert: mocks.upsert,
    remove: mocks.remove,
  };
  return { useConfig: () => api, usePrivacyTiersEnabled: () => true };
});

const openai = {
  name: 'openai',
  is_configured: true,
  resolved_tier: 'public',
  metadata: { display_name: 'OpenAI', known_models: [{ name: 'gpt-4o-mini' }] },
};
const ollama = {
  name: 'ollama',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'local', institutions: [] },
  metadata: { display_name: 'Ollama', known_models: [{ name: 'qwen3.8:27b' }] },
};
const decision = {
  name: 'ollama_decision',
  is_configured: true,
  resolved_tier: 'private',
  metadata: { display_name: 'Ollama decision models', known_models: [{ name: 'tev1:0.8b' }] },
};
const unconfigured = {
  name: 'anthropic',
  is_configured: false,
  resolved_tier: 'public',
  metadata: { display_name: 'Anthropic', known_models: [{ name: 'claude-sonnet' }] },
};

const row = () =>
  screen.getByText(defaultWorkerModelCopy.row).closest('.biorouter-settings-row') as HTMLElement;
const change = () => screen.getByRole('button', { name: defaultWorkerModelCopy.changeName });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.values = { BIOROUTER_PROVIDER: 'openai', BIOROUTER_MODEL: 'gpt-4o' };
  mocks.providers = [openai, ollama, decision, unconfigured];
  mocks.writes = [];
  mocks.upsert.mockImplementation(async (key: string, value: unknown) => {
    mocks.writes.push(`upsert ${key}`);
    mocks.values[key] = value;
  });
  mocks.remove.mockImplementation(async (key: string) => {
    mocks.writes.push(`remove ${key}`);
    delete mocks.values[key];
  });
  mocks.getProviderModels.mockResolvedValue([]);
});

describe('the Default worker model row', () => {
  it('reads "Same as the chat’s model" while no default worker is set, with its help', async () => {
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent(defaultWorkerModelCopy.sameAsChat));
    expect(change()).toHaveTextContent(defaultWorkerModelCopy.change);
    expect(defaultWorkerModelCopy.help).toBe(
      'Subagents use this model unless the agent picks another. Choose a lighter model to save tokens.'
    );
    expect(
      screen.getByRole('button', { name: `About ${defaultWorkerModelCopy.row}` })
    ).toBeInTheDocument();
  });

  it('names the default worker by its display names, with the tier mark', async () => {
    mocks.values[WORKER_PROVIDER_KEY] = 'ollama';
    mocks.values[WORKER_MODEL_KEY] = 'qwen3.8:27b';
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent('qwen3.8:27b · Ollama'));
    expect(row().querySelector('[data-crew-model-tier="private"]')).not.toBeNull();
    expect(row()).not.toHaveTextContent(defaultWorkerModelCopy.sameAsChat);
    // Never the raw provider id where the app has a name for it.
    expect(row()).not.toHaveTextContent('qwen3.8:27b · ollama');
  });

  it('offers "Same as the chat’s model" first, then configured chat models only', async () => {
    const user = userEvent.setup();
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(change()).toBeEnabled());
    await user.click(change());
    const list = await screen.findByRole('listbox', { name: defaultWorkerModelCopy.listLabel });
    const options = within(list).getAllByRole('option');
    expect(options[0]).toHaveTextContent(defaultWorkerModelCopy.sameAsChat);
    expect(options[0]).toHaveAttribute('aria-selected', 'true');
    expect(within(list).getByRole('option', { name: /^gpt-4o-mini/ })).toBeInTheDocument();
    expect(within(list).getByRole('option', { name: /^qwen3\.8:27b/ })).toBeInTheDocument();
    // A decision model cannot carry out a subagent's task; an unconfigured provider cannot run.
    expect(within(list).queryByRole('option', { name: /^tev1/ })).toBeNull();
    expect(within(list).queryByRole('option', { name: /^claude-sonnet/ })).toBeNull();
  });

  it('saves a chosen model with the provider written last', async () => {
    const user = userEvent.setup();
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(change()).toBeEnabled());
    await user.click(change());
    await user.click(await screen.findByRole('option', { name: /^gpt-4o-mini/ }));
    await waitFor(() => expect(row()).toHaveTextContent('gpt-4o-mini · OpenAI'));
    expect(mocks.writes).toEqual([`upsert ${WORKER_MODEL_KEY}`, `upsert ${WORKER_PROVIDER_KEY}`]);
    expect(mocks.upsert).toHaveBeenCalledWith(WORKER_MODEL_KEY, 'gpt-4o-mini', false);
    expect(mocks.upsert).toHaveBeenCalledWith(WORKER_PROVIDER_KEY, 'openai', false);
  });

  it('clears the provider before it replaces a model, so no half-pair is ever read', async () => {
    mocks.values[WORKER_PROVIDER_KEY] = 'ollama';
    mocks.values[WORKER_MODEL_KEY] = 'qwen3.8:27b';
    const user = userEvent.setup();
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent('qwen3.8:27b · Ollama'));
    await user.click(change());
    await user.click(await screen.findByRole('option', { name: /^gpt-4o-mini/ }));
    await waitFor(() => expect(row()).toHaveTextContent('gpt-4o-mini · OpenAI'));
    expect(mocks.writes).toEqual([
      `remove ${WORKER_PROVIDER_KEY}`,
      `upsert ${WORKER_MODEL_KEY}`,
      `upsert ${WORKER_PROVIDER_KEY}`,
    ]);
  });

  it('removes both keys for "Same as the chat’s model", provider first', async () => {
    mocks.values[WORKER_PROVIDER_KEY] = 'ollama';
    mocks.values[WORKER_MODEL_KEY] = 'qwen3.8:27b';
    const user = userEvent.setup();
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent('qwen3.8:27b · Ollama'));
    await user.click(change());
    await user.click(
      await screen.findByRole('option', { name: defaultWorkerModelCopy.sameAsChat })
    );
    await waitFor(() => expect(row()).toHaveTextContent(defaultWorkerModelCopy.sameAsChat));
    expect(mocks.writes).toEqual([`remove ${WORKER_PROVIDER_KEY}`, `remove ${WORKER_MODEL_KEY}`]);
    expect(mocks.upsert).not.toHaveBeenCalled();
  });

  it('says a stored model whose provider is not set up is not the one subagents use', async () => {
    mocks.values[WORKER_PROVIDER_KEY] = 'anthropic';
    mocks.values[WORKER_MODEL_KEY] = 'claude-sonnet';
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent(/isn’t set up/));
    expect(row()).toHaveTextContent(defaultWorkerModelCopy.sameAsChat);
  });

  it('says a refused write, and what the daemon still has', async () => {
    mocks.upsert.mockRejectedValue(new Error('400 Bad Request'));
    const user = userEvent.setup();
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(change()).toBeEnabled());
    await user.click(change());
    await user.click(await screen.findByRole('option', { name: /^gpt-4o-mini/ }));
    await waitFor(() =>
      expect(row()).toHaveTextContent(defaultWorkerModelCopy.saveFailed('400 Bad Request'))
    );
    expect(row()).toHaveTextContent(defaultWorkerModelCopy.sameAsChat);
  });

  it('reads again when another window writes the default worker', async () => {
    render(<DefaultWorkerModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent(defaultWorkerModelCopy.sameAsChat));
    mocks.values[WORKER_PROVIDER_KEY] = 'openai';
    mocks.values[WORKER_MODEL_KEY] = 'gpt-4o-mini';
    act(() => announceAppModelSelection());
    await waitFor(() => expect(row()).toHaveTextContent('gpt-4o-mini · OpenAI'));
  });
});

describe('the default worker model as stored', () => {
  it('is set only when both halves are', () => {
    expect(parseWorkerChoice('openai', 'gpt-4o-mini')).toEqual({
      provider: 'openai',
      model: 'gpt-4o-mini',
    });
    expect(parseWorkerChoice('openai', '')).toBeNull();
    expect(parseWorkerChoice(null, 'gpt-4o-mini')).toBeNull();
    expect(parseWorkerChoice(' ', ' ')).toBeNull();
  });

  it('runs on a configured chat provider, never on a decision model', () => {
    const usable = (item: unknown) => workerProviderUsable(item as never);
    expect(usable(openai)).toBe(true);
    expect(usable(decision)).toBe(false);
    expect(usable(unconfigured)).toBe(false);
    expect(usable(undefined)).toBe(false);
  });
});
