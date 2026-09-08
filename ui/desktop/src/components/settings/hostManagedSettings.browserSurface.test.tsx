import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ModelSettingsButtons from './models/subcomponents/ModelSettingsButtons';
import ResetProviderSection from './reset_provider/ResetProviderSection';
import { LeadWorkerSettings } from './models/subcomponents/LeadWorkerSettings';
import ConfigSettings from './config/ConfigSettings';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';

/**
 * SD-1 across the remaining Settings surfaces that write a capability config
 * key. Each of them 409s on a browser-served daemon today, and each does it
 * through a different verb:
 *
 * - `ModelSettingsButtons` → the switch-model dialog → `/config/set_provider`.
 * - `ResetProviderSection` → `/config/remove` on `BIOROUTER_PROVIDER`. ⚠ Easy
 *   to miss: a delete is guarded by the SAME predicate as a write, because a
 *   delete is not the absence of a write — it is a write of the key's default.
 * - `LeadWorkerSettings` → `/config/upsert` on three capability keys, and
 *   `/config/remove` on two of them when the mode is switched off.
 * - `ConfigSettings` → `/config/upsert` on whichever key the row names, which
 *   is why that one is guarded per key rather than per page.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
  remove: vi.fn(),
  refreshConfig: vi.fn(),
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  config: {} as Record<string, unknown>,
  currentProvider: 'openai' as string | null,
  refreshCurrentModelAndProvider: vi.fn(),
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    upsert: mocks.upsert,
    remove: mocks.remove,
    refreshConfig: mocks.refreshConfig,
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
    config: mocks.config,
  }),
}));

vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'gpt-5.5',
    currentProvider: mocks.currentProvider,
    refreshCurrentModelAndProvider: mocks.refreshCurrentModelAndProvider,
  }),
}));

vi.mock('./models/subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: () => <div>SWITCH-MODEL-MODAL</div>,
}));

vi.mock('./models/predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

vi.mock('./models/modelInterface', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  fetchModelsForProviders: async () => [],
}));

const browser = () => {
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.read.mockResolvedValue('');
  mocks.upsert.mockResolvedValue(undefined);
  mocks.remove.mockResolvedValue(undefined);
  mocks.refreshConfig.mockResolvedValue(undefined);
  mocks.getProviders.mockResolvedValue([]);
  mocks.getProviderModels.mockResolvedValue([]);
  mocks.currentProvider = 'openai';
  mocks.config = {};
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('Settings > Models — Switch models', () => {
  /**
   * ⚠ Fails against today's code: the button carries no `disabled` and there is
   * no note to find, so a browser user opens the dialog and meets the 409.
   */
  it('is disabled with a reason on a browser surface', async () => {
    browser();
    render(<ModelSettingsButtons setView={vi.fn()} />);

    expect(screen.getByRole('button', { name: 'Switch models' })).toBeDisabled();
    expect((await screen.findByTestId('host-managed-model-note')).textContent).toMatch(
      /biorouter configure/
    );
  });

  /**
   * ⚠ **Deliberately still enabled.** Storing a provider's API key is not a
   * capability write and does not 409, so disabling this too would remove a
   * capability the browser actually has. The block belongs on the step that is
   * genuinely refused, and this asserts the change did not over-reach.
   */
  it('leaves "Configure providers" reachable, because keys still save', () => {
    browser();
    render(<ModelSettingsButtons setView={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Configure providers' })).toBeEnabled();
  });

  /** The control: passes before and after. */
  it('is untouched in the desktop application', () => {
    render(<ModelSettingsButtons setView={vi.fn()} />);
    const button = screen.getByRole('button', { name: 'Switch models' });
    expect(button).toBeEnabled();
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();

    fireEvent.click(button);
    expect(screen.getByText('SWITCH-MODEL-MODAL')).toBeInTheDocument();
  });
});

describe('Settings > Models — Reset provider and model', () => {
  /**
   * ⚠ Fails against today's code, which offers the button on every surface.
   * Worth its own case rather than folding into the picker's: this one reaches
   * `/config/remove`, and it is the verb an implementation guarding only
   * *writes* would leave open — while succeeding here would strand the browser
   * session with no provider at all and no way to choose another.
   */
  it('is disabled with a reason on a browser surface', async () => {
    browser();
    render(<ResetProviderSection setView={vi.fn()} />);

    expect(screen.getByRole('button', { name: /Reset provider and model/ })).toBeDisabled();
    expect(await screen.findByTestId('host-managed-model-note')).toBeInTheDocument();
  });

  /** The control: passes before and after. */
  it('is untouched in the desktop application', () => {
    render(<ResetProviderSection setView={vi.fn()} />);
    expect(screen.getByRole('button', { name: /Reset provider and model/ })).toBeEnabled();
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();
  });
});

describe('Settings > Models — Lead/Worker', () => {
  /**
   * ⚠ Fails against today's code: Save is enabled whenever the mode is off
   * (the existing `disabled` only covers the enabled-but-incomplete case), so a
   * browser user can click it and fire five refused config writes at once.
   */
  it('disables Save with a reason on a browser surface', async () => {
    browser();
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);

    const save = await screen.findByRole('button', { name: 'Save Settings' });
    expect(save).toBeDisabled();
    expect(screen.getByTestId('host-managed-model-note')).toBeInTheDocument();
  });

  /**
   * The write, not the attribute. `handleSave` carries its own guard, and
   * without this a rewrite could drop it and still pass the test above.
   */
  it('performs no config write if Save is clicked anyway', async () => {
    browser();
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Save Settings' }));
    await waitFor(() => expect(mocks.read).toHaveBeenCalled());
    expect(mocks.upsert).not.toHaveBeenCalled();
    expect(mocks.remove).not.toHaveBeenCalled();
  });

  /**
   * The control. ⚠ Note it asserts the write REACHES the config, not merely
   * that the button is enabled — the desktop path has to keep working, and
   * "enabled" alone would still pass if `handleSave` returned early for
   * everyone.
   */
  it('still saves in the desktop application', async () => {
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);

    const save = await screen.findByRole('button', { name: 'Save Settings' });
    expect(save).toBeEnabled();
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();

    fireEvent.click(save);
    // Mode is off (no lead model configured), so Save takes the removal branch.
    await waitFor(() => expect(mocks.remove).toHaveBeenCalledWith('BIOROUTER_LEAD_MODEL', false));
  });
});

describe('Settings > Configuration editor', () => {
  async function openEditor() {
    render(<ConfigSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /Edit Configuration/ }));
  }

  /**
   * ⚠ **The row-level test, and the reason `isHostManagedConfigKey` exists.**
   *
   * This editor renders every non-secret key, and only five of them 409. Fails
   * against today's code (no row is ever disabled) — but it would ALSO fail a
   * lazy fix that disabled the whole page, because `BIOROUTER_MODEL` and
   * `OLLAMA_TIMEOUT` here must stay editable: neither is a capability key, so
   * both still save from a browser.
   */
  it('freezes only the keys the daemon actually refuses', async () => {
    browser();
    // `ollama`, so the editor's own provider-prefix filter keeps the two
    // `OLLAMA_*` rows on screen — one a capability key, one not, which is the
    // pair that separates a per-key guard from a per-page one.
    mocks.currentProvider = 'ollama';
    mocks.config = {
      BIOROUTER_PROVIDER: 'ollama',
      OLLAMA_HOST: 'http://localhost:11434',
      BIOROUTER_MODEL: 'gpt-5.5',
      OLLAMA_TIMEOUT: '600',
    };
    await openEditor();

    for (const frozen of ['BIOROUTER_PROVIDER', 'OLLAMA_HOST']) {
      const note = await screen.findByTestId(`host-managed-config-${frozen}`);
      const row = note.closest('.grid');
      expect(row).not.toBeNull();
      expect(within(row as HTMLElement).getByRole('textbox')).toBeDisabled();
      expect(within(row as HTMLElement).getByRole('button')).toBeDisabled();
    }

    for (const editable of ['BIOROUTER_MODEL', 'OLLAMA_TIMEOUT']) {
      expect(screen.queryByTestId(`host-managed-config-${editable}`)).toBeNull();
    }
    const stillEditable = screen.getByDisplayValue('gpt-5.5');
    expect(stillEditable).toBeEnabled();
    fireEvent.change(stillEditable, { target: { value: 'gpt-5.6' } });
    expect(stillEditable).toBeEnabled();
  });

  /** The control: passes before and after. */
  it('leaves every row editable in the desktop application', async () => {
    mocks.currentProvider = 'ollama';
    mocks.config = { BIOROUTER_PROVIDER: 'ollama', OLLAMA_HOST: 'http://localhost:11434' };
    await openEditor();

    expect(await screen.findByDisplayValue('http://localhost:11434')).toBeEnabled();
    expect(screen.getByDisplayValue('ollama')).toBeEnabled();
    expect(screen.queryByTestId('host-managed-config-BIOROUTER_PROVIDER')).toBeNull();
    expect(screen.queryByTestId('host-managed-config-OLLAMA_HOST')).toBeNull();
  });
});
