import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import type { ConfigData } from '../../../types/config';

// #50 — Settings → App named a stale provider ("current settings for ollama"
// while the active provider was versa_azure). ConfigContext's `config` is only
// refetched when a write goes THROUGH that context; switching model/provider
// writes via `setConfigProvider` on the API instead, so the cached copy keeps
// naming whatever was configured when the app started. The live provider is
// the one ModelAndProviderContext tracks.

const configSnapshot: ConfigData = {
  BIOROUTER_PROVIDER: 'ollama', // stale: what the app booted with
  BIOROUTER_MODEL: 'qwen3.8:27b',
  OLLAMA_HOST: 'localhost',
};

const modelAndProvider = {
  currentProvider: null as string | null,
  refreshCurrentModelAndProvider: vi.fn(async () => {}),
};

const refreshConfig = vi.fn(async () => {});

/** The providers the daemon lists. `ollama` is deliberately absent, so a provider it cannot name
 * falls back to its id. */
let providerList: Array<{ name: string; metadata: { display_name: string } }> = [];
const getProviders = vi.fn(async () => providerList);

/** Set to make the next `refreshConfig()` reject, as a failed config read does. */
let refreshConfigRejection: Error | null = null;

// Deliberately a plain function rather than the spy itself. A vitest spy
// attaches its own handler to a promise it returns (that is how it records
// settled results), which marks the rejection as handled — so a spy here would
// swallow the very unhandled rejection these tests exist to detect, and they
// would pass against the broken code. The spy is still called, so the
// call-count assertions are unaffected.
//
// Defined once, at module scope: the real `refreshConfig` is a `useCallback`
// with no dependencies, so it keeps one identity for the provider's lifetime.
// Handing back a fresh closure per render would instead re-fire the mount
// effect on every render, which is a property of the mock and not of the app.
const refreshConfigStub = () => {
  void refreshConfig();
  return refreshConfigRejection ? Promise.reject(refreshConfigRejection) : Promise.resolve();
};

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    config: configSnapshot,
    upsert: vi.fn(),
    refreshConfig: refreshConfigStub,
    getProviders,
  }),
}));

vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => modelAndProvider,
}));

import ConfigSettings from './ConfigSettings';
import { configCopy } from './copy';

/** The Configuration row, whose value is the provider the settings belong to. */
const configRow = () => screen.getByTestId('config-settings-row');

beforeEach(() => {
  vi.clearAllMocks();
  modelAndProvider.currentProvider = 'versa_azure';
  refreshConfigRejection = null;
  providerList = [
    { name: 'versa_azure', metadata: { display_name: 'Versa API Azure' } },
    { name: 'databricks', metadata: { display_name: 'Databricks' } },
  ];
});

describe('ConfigSettings provider label (#50)', () => {
  it('names the live provider, not the one cached in the config context', async () => {
    render(<ConfigSettings />);

    expect(await within(configRow()).findByText('Versa API Azure')).toBeInTheDocument();
    expect(screen.queryByText('ollama')).not.toBeInTheDocument();
    // The sentence that used to sit under the header is gone: the row's value says it.
    expect(screen.queryByText(/current settings for/)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/Edit your Biorouter configuration settings/)
    ).not.toBeInTheDocument();
  });

  it('updates the value when the active provider changes', async () => {
    const { rerender } = render(<ConfigSettings />);
    await within(configRow()).findByText('Versa API Azure');

    modelAndProvider.currentProvider = 'databricks';
    rerender(<ConfigSettings />);

    expect(within(configRow()).getByText('Databricks')).toBeInTheDocument();
    expect(within(configRow()).queryByText('Versa API Azure')).not.toBeInTheDocument();
  });

  // V4-12: the row said `versa_azure` in the mono face where the Models tab says "Versa API Azure".
  it('shows the provider by name in the sans face, never by id', async () => {
    render(<ConfigSettings />);

    const value = await within(configRow()).findByText('Versa API Azure');
    expect(value).not.toHaveClass('font-mono');
    expect(within(configRow()).queryByText('versa_azure')).not.toBeInTheDocument();
  });

  it('shows no value until the provider list has answered, not the raw id', async () => {
    render(<ConfigSettings />);

    // Synchronously after the first paint the lookup is still pending.
    expect(within(configRow()).queryByText('versa_azure')).not.toBeInTheDocument();
    expect(within(configRow()).queryByText('Versa API Azure')).not.toBeInTheDocument();
    // Then the name arrives (and the update is awaited, not left to land after the test).
    expect(await within(configRow()).findByText('Versa API Azure')).toBeInTheDocument();
  });

  it('falls back to the id for a provider the daemon does not list', async () => {
    modelAndProvider.currentProvider = 'ollama';
    render(<ConfigSettings />);

    expect(await within(configRow()).findByText('ollama')).toBeInTheDocument();
  });

  it('falls back to the id when the provider list cannot be read', async () => {
    getProviders.mockRejectedValueOnce(new Error('daemon unreachable'));
    render(<ConfigSettings />);

    expect(await within(configRow()).findByText('versa_azure')).toBeInTheDocument();
  });

  it('is one row: the word Configuration, the provider, an Edit… button and a tip', async () => {
    render(<ConfigSettings />);

    const row = configRow();
    await within(row).findByText('Versa API Azure');
    expect(within(row).getByText(configCopy.row)).toBeInTheDocument();
    const edit = within(row).getByRole('button', { name: configCopy.editName });
    expect(edit).toHaveTextContent(configCopy.edit);
    // Edit… is the 28px secondary rung, and the tip explains the row without a paragraph.
    expect(edit).toHaveAccessibleDescription(configCopy.help);
    expect(
      within(row).getByRole('button', { name: `About ${configCopy.row}` })
    ).toBeInTheDocument();
  });

  it('re-reads the active provider when the section mounts', async () => {
    render(<ConfigSettings />);

    await waitFor(() =>
      expect(modelAndProvider.refreshCurrentModelAndProvider).toHaveBeenCalledTimes(1)
    );
  });

  // #52 — this page renders BIOROUTER_PROVIDER/BIOROUTER_MODEL as editable
  // fields straight out of the cached config. A model switch writes them
  // through the API, so opening Settings after one showed the pre-switch
  // values in the editor beside a correct live label.
  it('re-reads the cached config when the section opens', async () => {
    render(<ConfigSettings />);

    await waitFor(() => expect(refreshConfig).toHaveBeenCalledTimes(1));
  });

  it('falls back to the config value while the live provider is still loading', async () => {
    modelAndProvider.currentProvider = null;
    render(<ConfigSettings />);

    expect(await within(configRow()).findByText('ollama')).toBeInTheDocument();
  });

  // `refreshConfig` used to be incapable of rejecting: it called the non-throwing
  // `readAllConfig` and swallowed a failed read as an empty config. Making the
  // read reject (so it can no longer erase the cache) changed that contract, and
  // this mount effect discards the promise — so a daemon that cannot serve the
  // config turns opening Settings into an unhandled rejection. The page has
  // nothing to do about it either way: the cached snapshot it renders is still
  // there, which is the whole point of the read no longer erasing it.
  it('does not leave an unhandled rejection when the config re-read fails', async () => {
    const unhandled: unknown[] = [];
    const capture = (reason: unknown) => unhandled.push(reason);
    process.on('unhandledRejection', capture);

    try {
      refreshConfigRejection = new Error('daemon unreachable');

      render(<ConfigSettings />);

      await waitFor(() => expect(refreshConfig).toHaveBeenCalledTimes(1));
      // Node decides a rejection is unhandled at the end of a turn, not at the
      // point of rejection — give it a few.
      await new Promise((resolve) => setTimeout(resolve, 0));
      await new Promise((resolve) => setTimeout(resolve, 0));
      await new Promise((resolve) => setImmediate(resolve));

      expect(unhandled.map(String)).toEqual([]);
    } finally {
      process.off('unhandledRejection', capture);
    }
  });

  it('still renders the cached config when the re-read fails', async () => {
    refreshConfigRejection = new Error('daemon unreachable');

    render(<ConfigSettings />);

    await waitFor(() => expect(refreshConfig).toHaveBeenCalledTimes(1));
    expect(await within(configRow()).findByText('Versa API Azure')).toBeInTheDocument();
  });
});
