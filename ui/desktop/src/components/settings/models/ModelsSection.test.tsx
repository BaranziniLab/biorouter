import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsSection from './ModelsSection';
import { defaultModelCopy, modelsDangerCopy } from './copy';
import { SETTINGS_SECTION_IDS } from '../settingsSections';

/**
 * The Models tab is three sections of rows (spec 3.13): Default model (a Model row with a
 * Change… action and a Providers row with Manage…), Local models, and the Danger zone. These
 * tests hold the anatomy and the deep-link ids; the dialogs and the inventory have their own.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  getProviders: vi.fn(),
  predefined: false,
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    getProviders: mocks.getProviders,
    remove: vi.fn(),
  }),
}));

vi.mock('../../ModelAndProviderContext', () => ({
  UNKNOWN_PROVIDER_TITLE: 'Unknown provider',
  UNKNOWN_PROVIDER_MSG: 'Unknown provider message',
  useModelAndProvider: () => ({
    getCurrentModelDisplayName: async () => 'gpt-5.6-sol',
    getCurrentProviderDisplayName: async () => 'Versa Azure',
    currentModel: 'gpt-5.6-sol',
    currentProvider: 'versa_azure',
  }),
}));

vi.mock('./subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: () => <div>SWITCH-MODEL-MODAL</div>,
}));

vi.mock('./LocalModelInventory', () => ({
  default: () => <section id="settings-local-models">LOCAL-MODELS</section>,
}));

vi.mock('./predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => mocks.predefined,
}));

describe('ModelsSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.predefined = false;
    mocks.read.mockResolvedValue('');
    mocks.getProviders.mockResolvedValue([]);
  });

  it('holds three sections whose ids the Settings deep links scroll to', async () => {
    const { container } = render(<ModelsSection setView={vi.fn()} />);
    await screen.findByText('gpt-5.6-sol · Versa Azure');

    expect(container.querySelector(`#${SETTINGS_SECTION_IDS.defaultModel}`)).not.toBeNull();
    expect(container.querySelector(`#${SETTINGS_SECTION_IDS.localModels}`)).not.toBeNull();
    expect(container.querySelector(`#${SETTINGS_SECTION_IDS.modelsDangerZone}`)).not.toBeNull();
    const dangerZone = container.querySelector(`#${SETTINGS_SECTION_IDS.modelsDangerZone}`);
    expect(
      within(dangerZone as HTMLElement).getByRole('heading', { name: modelsDangerCopy.section })
    ).toBeInTheDocument();
  });

  it('shows the model and provider as the Model row’s value, beside a Change… button', async () => {
    const user = userEvent.setup();
    render(<ModelsSection setView={vi.fn()} />);

    expect(await screen.findByText('gpt-5.6-sol · Versa Azure')).toBeInTheDocument();
    // "Change…" is what it says; the dialog it opens keeps its name "Switch models".
    const change = screen.getByRole('button', { name: defaultModelCopy.changeName });
    expect(change).toHaveTextContent(defaultModelCopy.change);

    await user.click(change);
    expect(screen.getByText('SWITCH-MODEL-MODAL')).toBeInTheDocument();
  });

  it('opens the provider catalog from the Providers row', async () => {
    const user = userEvent.setup();
    const setView = vi.fn();
    render(<ModelsSection setView={setView} />);
    await screen.findByText('gpt-5.6-sol · Versa Azure');

    const manage = screen.getByRole('button', { name: defaultModelCopy.manageName });
    expect(manage).toHaveTextContent(defaultModelCopy.manage);
    await user.click(manage);
    expect(setView).toHaveBeenCalledWith('ConfigureProviders');
  });

  it('has no Providers row when the deployment pins its models', async () => {
    mocks.predefined = true;
    render(<ModelsSection setView={vi.fn()} />);
    await screen.findByText('gpt-5.6-sol · Versa Azure');

    expect(
      screen.queryByRole('button', { name: defaultModelCopy.manageName })
    ).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: defaultModelCopy.changeName })).toBeInTheDocument();
  });

  it('puts the explanations in InfoTips and no description paragraph in a row', async () => {
    const { container } = render(<ModelsSection setView={vi.fn()} />);
    await screen.findByText('gpt-5.6-sol · Versa Azure');

    expect(
      screen.getByRole('button', { name: `About ${defaultModelCopy.section}` })
    ).toBeInTheDocument();
    expect(container.querySelector('.biorouter-settings-row p')).toBeNull();
  });
});
