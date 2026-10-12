import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { renderToString } from 'react-dom/server';
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
  /** What the app already holds: the provider list and the model in force. */
  providersList: [] as unknown[],
  currentModel: 'gpt-5.6-sol' as string | null,
  currentProvider: 'versa_azure' as string | null,
  /** The daemon's two reads, with identities that hold still across renders. */
  getCurrentModelDisplayName: vi.fn(),
  getCurrentProviderDisplayName: vi.fn(),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    getProviders: mocks.getProviders,
    providersList: mocks.providersList,
    remove: vi.fn(),
  }),
}));

vi.mock('../../ModelAndProviderContext', () => ({
  UNKNOWN_PROVIDER_TITLE: 'Unknown provider',
  UNKNOWN_PROVIDER_MSG: 'Unknown provider message',
  useModelAndProvider: () => ({
    getCurrentModelDisplayName: mocks.getCurrentModelDisplayName,
    getCurrentProviderDisplayName: mocks.getCurrentProviderDisplayName,
    currentModel: mocks.currentModel,
    currentProvider: mocks.currentProvider,
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
  getModelDisplayName: (name: string) => name,
  getProviderDisplayName: () => '',
}));

describe('ModelsSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.predefined = false;
    mocks.providersList = [];
    mocks.currentModel = 'gpt-5.6-sol';
    mocks.currentProvider = 'versa_azure';
    mocks.read.mockResolvedValue('');
    mocks.getProviders.mockResolvedValue([]);
    mocks.getCurrentModelDisplayName.mockResolvedValue('gpt-5.6-sol');
    mocks.getCurrentProviderDisplayName.mockResolvedValue('Versa Azure');
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

  describe('the Model row on a tab switch (V4-R7-1, principle 9)', () => {
    const provider = (name: string, display: string) => ({
      name,
      metadata: { display_name: display },
    });
    const skeletons = (container: HTMLElement) =>
      container.querySelectorAll('[data-slot="skeleton"]');
    /** A daemon that has not answered yet, so only what the app already holds can be on screen. */
    const daemonSilent = () => {
      mocks.getCurrentModelDisplayName.mockReturnValue(new Promise(() => {}));
      mocks.getCurrentProviderDisplayName.mockReturnValue(new Promise(() => {}));
    };

    it('paints the model and provider in the first render, with no skeleton, before the daemon answers', () => {
      mocks.providersList = [provider('versa_azure', 'Versa Azure')];
      daemonSilent();
      const { container } = render(<ModelsSection setView={vi.fn()} />);

      // Read synchronously: nothing has awaited, so this is the first committed frame.
      expect(screen.getByText('gpt-5.6-sol · Versa Azure')).toBeInTheDocument();
      expect(skeletons(container)).toHaveLength(0);
    });

    it('has the value in the render itself, not set by an effect after the first frame', () => {
      // `render` flushes effects inside `act`, so it cannot tell a value drawn in the first frame
      // from one an effect wrote right after it. A server render runs no effects: what it returns
      // is exactly what the first frame holds.
      mocks.providersList = [provider('versa_azure', 'Versa Azure')];
      daemonSilent();
      const html = renderToString(<ModelsSection setView={vi.fn()} />);

      expect(html).toContain('gpt-5.6-sol · Versa Azure');
      expect(html).not.toContain('data-slot="skeleton"');

      mocks.providersList = [];
      expect(renderToString(<ModelsSection setView={vi.fn()} />)).toContain('data-slot="skeleton"');
    });

    it('keeps the same value on screen when the daemon then answers with the same words', async () => {
      mocks.providersList = [provider('versa_azure', 'Versa Azure')];
      const { container } = render(<ModelsSection setView={vi.fn()} />);
      const value = screen.getByText('gpt-5.6-sol · Versa Azure');

      await vi.waitFor(() => expect(mocks.getCurrentProviderDisplayName).toHaveBeenCalled());
      await Promise.resolve();
      // The same node, not a skeleton and then a new one.
      expect(screen.getByText('gpt-5.6-sol · Versa Azure')).toBe(value);
      expect(skeletons(container)).toHaveLength(0);
    });

    it('waits on a skeleton only when the app cannot say yet, then shows the daemon’s answer', async () => {
      mocks.providersList = [];
      const { container } = render(<ModelsSection setView={vi.fn()} />);

      expect(skeletons(container)).toHaveLength(1);
      expect(await screen.findByText('gpt-5.6-sol · Versa Azure')).toBeInTheDocument();
      expect(skeletons(container)).toHaveLength(0);
    });

    it('waits on a skeleton for a provider the list does not know, not half a sentence', () => {
      mocks.providersList = [provider('anthropic', 'Anthropic')];
      daemonSilent();
      const { container } = render(<ModelsSection setView={vi.fn()} />);

      expect(skeletons(container)).toHaveLength(1);
      expect(screen.queryByText(/gpt-5.6-sol/)).not.toBeInTheDocument();
    });

    it('changes the words at once when the model in force changes, never the old model beside the new', () => {
      mocks.providersList = [
        provider('versa_azure', 'Versa Azure'),
        provider('anthropic', 'Anthropic'),
      ];
      daemonSilent();
      const { rerender, container } = render(<ModelsSection setView={vi.fn()} />);
      expect(screen.getByText('gpt-5.6-sol · Versa Azure')).toBeInTheDocument();

      mocks.currentModel = 'claude-opus-5';
      mocks.currentProvider = 'anthropic';
      rerender(<ModelsSection setView={vi.fn()} />);

      expect(screen.getByText('claude-opus-5 · Anthropic')).toBeInTheDocument();
      expect(screen.queryByText(/gpt-5.6-sol/)).not.toBeInTheDocument();
      expect(skeletons(container)).toHaveLength(0);
    });
  });
});
