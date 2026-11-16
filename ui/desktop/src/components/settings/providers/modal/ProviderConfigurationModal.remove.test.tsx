import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import ProviderConfigurationModal from './ProviderConfiguationModal';
import { providerSetupCopy } from '../copy';

/**
 * W1-35: Remove clears what Biorouter saved for a built-in provider, but a key in the environment
 * the daemon was started in keeps it configured. The row then still read "Configured" and nothing
 * said why, so Remove looked like it did nothing.
 */
const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  remove: vi.fn(),
  getProviders: vi.fn(),
  onClose: vi.fn(),
}));
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    upsert: vi.fn(),
    remove: mocks.remove,
    getProviders: mocks.getProviders,
  }),
}));
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: async () => ({ provider: 'versa_azure', model: 'gpt-5.6-sol' }),
  }),
}));

function anthropic(isConfigured: boolean): ProviderDetails {
  return {
    name: 'anthropic',
    is_configured: isConfigured,
    provider_type: 'Builtin',
    metadata: {
      name: 'anthropic',
      display_name: 'Anthropic',
      description: '',
      default_model: 'claude-fable-5',
      known_models: [],
      model_doc_link: '',
      config_keys: [
        { name: 'ANTHROPIC_API_KEY', required: true, secret: true, oauth_flow: false },
        {
          name: 'ANTHROPIC_HOST',
          required: false,
          secret: false,
          oauth_flow: false,
          default: 'https://api.anthropic.com',
        },
      ],
    },
  } as ProviderDetails;
}

/** What `/config/read` answers once Biorouter's own copy is gone: only the environment's. */
function environmentHolds(keys: readonly string[]) {
  mocks.read.mockImplementation(async (key: string, secret: boolean) =>
    keys.includes(key) ? (secret ? { masked_value: 'sk-a...wxyz' } : 'value') : null
  );
}

async function removeAnthropic() {
  render(<ProviderConfigurationModal provider={anthropic(true)} onClose={mocks.onClose} />);
  fireEvent.click(await screen.findByRole('button', { name: providerSetupCopy.remove }));
  fireEvent.click(await screen.findByRole('button', { name: providerSetupCopy.delete }));
  await waitFor(() => expect(mocks.remove).toHaveBeenCalledTimes(2));
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.remove.mockResolvedValue(undefined);
});

describe('removing a built-in provider', () => {
  it('a provider still configured from the environment says so after removal', async () => {
    environmentHolds(['ANTHROPIC_API_KEY']);
    mocks.getProviders.mockResolvedValue([anthropic(true)]);

    await removeAnthropic();

    const dialog = await screen.findByRole('dialog');
    const line = await within(dialog).findByTestId('provider-still-set-from-environment');
    // One neutral line: it names the variable and where it is removed.
    expect(line).toHaveTextContent('ANTHROPIC_API_KEY');
    expect(line).toHaveTextContent(/outside Biorouter/);
    expect(line.className).not.toMatch(/danger|warning/);
    expect(within(dialog).getByText(providerSetupCopy.stillSetUp('Anthropic'))).toBeVisible();
    // Biorouter's own copy was still removed, and the provider list was asked again.
    expect(mocks.remove).toHaveBeenCalledWith('ANTHROPIC_API_KEY', true);
    expect(mocks.getProviders).toHaveBeenCalledWith(true);
    // It stays open until the person has read it.
    expect(mocks.onClose).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole('button', { name: providerSetupCopy.ok }));
    expect(mocks.onClose).toHaveBeenCalledTimes(1);
  });

  it('closes as before when nothing in the environment keeps the provider set up', async () => {
    environmentHolds([]);
    mocks.getProviders.mockResolvedValue([anthropic(false)]);

    await removeAnthropic();

    await waitFor(() => expect(mocks.onClose).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('provider-still-set-from-environment')).toBeNull();
  });
});
