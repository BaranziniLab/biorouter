import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { announceAppModelSelection } from '../../../../utils/sessionBindingSync';
import { crewTaskModelCopy } from '../copy';
import { CrewTaskModelRow } from './CrewTaskModelRow';

/**
 * Settings › Models › Default model › Crew tasks (Crew design OW-11): the model Ask my agent and
 * `@agent` start a Crew task on, stored as `CREW_TASK_MODEL`, never Claude Code or Codex.
 */

const mocks = vi.hoisted(() => ({
  values: {} as Record<string, unknown>,
  providers: [] as unknown[],
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

const versa = {
  name: 'versa_azure',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
  metadata: { display_name: 'Versa', known_models: [{ name: 'gpt-5.5' }] },
};
const ollama = {
  name: 'ollama',
  is_configured: true,
  resolved_tier: 'private',
  affiliation: { kind: 'local', institutions: [] },
  metadata: { display_name: 'Ollama', known_models: [{ name: 'qwen3.8:27b' }] },
};
const claudeCode = {
  name: 'claude_code',
  is_configured: true,
  resolved_tier: 'public',
  metadata: { display_name: 'Claude Code', known_models: [{ name: 'opus' }] },
};
const codex = {
  name: 'codex',
  is_configured: true,
  resolved_tier: 'public',
  metadata: { display_name: 'Codex', known_models: [{ name: 'gpt-6-sol' }] },
};

const row = () =>
  screen.getByText(crewTaskModelCopy.row).closest('.biorouter-settings-row') as HTMLElement;
const change = () => screen.getByRole('button', { name: crewTaskModelCopy.changeName });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.values = { BIOROUTER_PROVIDER: 'versa_azure', BIOROUTER_MODEL: 'gpt-5.5' };
  mocks.providers = [versa, ollama, claudeCode, codex];
  mocks.upsert.mockImplementation(async (key: string, value: unknown) => {
    mocks.values[key] = value;
  });
  mocks.remove.mockImplementation(async (key: string) => {
    delete mocks.values[key];
  });
  mocks.getProviderModels.mockResolvedValue([]);
});

describe('the Crew tasks row (OW-11)', () => {
  it('reads "Same as new chats" while Crew tasks have no model of their own', async () => {
    render(<CrewTaskModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent(crewTaskModelCopy.sameAsNewChats));
    expect(change()).toHaveTextContent(crewTaskModelCopy.change);
    expect(
      screen.getByRole('button', { name: `About ${crewTaskModelCopy.row}` })
    ).toBeInTheDocument();
  });

  it('names its own model with the tier mark', async () => {
    mocks.values.CREW_TASK_MODEL = { provider: 'ollama', model: 'qwen3.8:27b' };
    render(<CrewTaskModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent('qwen3.8:27b · Ollama'));
    expect(row().querySelector('[data-crew-model-tier="private"]')).not.toBeNull();
    expect(row()).not.toHaveTextContent(crewTaskModelCopy.sameAsNewChats);
  });

  it('offers "Same as new chats" first, then every model but Claude Code’s and Codex’s', async () => {
    const user = userEvent.setup();
    render(<CrewTaskModelRow />);
    await waitFor(() => expect(change()).toBeEnabled());
    await user.click(change());
    const list = await screen.findByRole('listbox', { name: crewTaskModelCopy.listLabel });
    const options = within(list).getAllByRole('option');
    expect(options[0]).toHaveTextContent(crewTaskModelCopy.sameAsNewChats);
    expect(options[0]).toHaveAttribute('aria-selected', 'true');
    expect(within(list).getByRole('option', { name: /^gpt-5\.5/ })).toBeInTheDocument();
    expect(within(list).getByRole('option', { name: /^qwen3\.8:27b/ })).toBeInTheDocument();
    expect(within(list).queryByRole('option', { name: /^opus/ })).toBeNull();
    expect(within(list).queryByRole('option', { name: /^gpt-6-sol/ })).toBeNull();
    expect(within(list).queryByText('Claude Code')).toBeNull();
    expect(within(list).queryByText('Codex')).toBeNull();
  });

  it('saves a chosen model as {provider, model}, and says it', async () => {
    const user = userEvent.setup();
    render(<CrewTaskModelRow />);
    await waitFor(() => expect(change()).toBeEnabled());
    await user.click(change());
    await user.click(await screen.findByRole('option', { name: /^qwen3\.8:27b/ }));
    expect(mocks.upsert).toHaveBeenCalledWith(
      'CREW_TASK_MODEL',
      { provider: 'ollama', model: 'qwen3.8:27b' },
      false
    );
    await waitFor(() => expect(row()).toHaveTextContent('qwen3.8:27b · Ollama'));
  });

  it('removes the key for "Same as new chats"', async () => {
    mocks.values.CREW_TASK_MODEL = { provider: 'ollama', model: 'qwen3.8:27b' };
    const user = userEvent.setup();
    render(<CrewTaskModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent('qwen3.8:27b · Ollama'));
    await user.click(change());
    await user.click(await screen.findByRole('option', { name: crewTaskModelCopy.sameAsNewChats }));
    expect(mocks.remove).toHaveBeenCalledWith('CREW_TASK_MODEL', false);
    expect(mocks.upsert).not.toHaveBeenCalled();
    await waitFor(() => expect(row()).toHaveTextContent(crewTaskModelCopy.sameAsNewChats));
  });

  it('says a stored model that Crew cannot use is not the one tasks start on', async () => {
    mocks.values.CREW_TASK_MODEL = { provider: 'claude_code', model: 'opus' };
    render(<CrewTaskModelRow />);
    await waitFor(() =>
      expect(row()).toHaveTextContent(crewTaskModelCopy.unavailable('opus · Claude Code'))
    );
    expect(row()).toHaveTextContent(crewTaskModelCopy.sameAsNewChats);
  });

  it('says a refused write, and what the daemon still has', async () => {
    mocks.upsert.mockRejectedValue(new Error('409 Conflict'));
    const user = userEvent.setup();
    render(<CrewTaskModelRow />);
    await waitFor(() => expect(change()).toBeEnabled());
    await user.click(change());
    await user.click(await screen.findByRole('option', { name: /^gpt-5\.5/ }));
    await waitFor(() =>
      expect(row()).toHaveTextContent(crewTaskModelCopy.saveFailed('409 Conflict'))
    );
    expect(row()).toHaveTextContent(crewTaskModelCopy.sameAsNewChats);
  });

  it('reads again when another window writes the model', async () => {
    render(<CrewTaskModelRow />);
    await waitFor(() => expect(row()).toHaveTextContent(crewTaskModelCopy.sameAsNewChats));
    mocks.values.CREW_TASK_MODEL = { provider: 'versa_azure', model: 'gpt-5.5' };
    act(() => announceAppModelSelection());
    await waitFor(() => expect(row()).toHaveTextContent('gpt-5.5 · Versa'));
  });
});
