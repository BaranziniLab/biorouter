import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  BROWSER_KEEPS_CHAT_MODEL,
  BROWSER_PUBLIC_DEFAULT_HIDES_PRIVATE,
  SWITCH_SCOPE_NEW_CHATS,
  SwitchModelModal,
  USE_FOR_THIS_CHAT_LABEL,
} from './SwitchModelModal';
import { BROWSER_SURFACE_MARKER } from '../../../../utils/surface';
import {
  loadProviderSetupAccess,
  resetProviderSetupAccessForTests,
} from '../../../../utils/providerSetupAccess';

/**
 * SD-1 at the choke point: `ModelsBottomBar`, `ModelSettingsButtons`,
 * `ProviderGrid` and `ProviderGuard` all open this one dialog, and its confirm
 * button reaches `changeModel` → `POST /config/set_provider`, which a
 * browser-served daemon refuses with a 409. Guarding only the four entry points
 * would leave the fifth one somebody adds next year unguarded.
 */

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
  getProviderSetupAccess: vi.fn(),
  /** The host's current default, which the private-to-public warning compares against. */
  currentProvider: 'openai',
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getProviderSetupAccess: mocks.getProviderSetupAccess,
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
    currentProvider: mocks.currentProvider,
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

// A stub that reports its own `isDisabled`, so the inert-input half of the
// requirement is observable. The real react-select renders no attribute a
// jsdom query can reach for a disabled *container*.
vi.mock('../../../ui/Select', () => ({
  Select: ({
    value,
    placeholder,
    isDisabled,
  }: {
    value?: { value?: string } | null;
    placeholder?: string;
    isDisabled?: boolean;
  }) => (
    <div
      data-testid={placeholder?.startsWith('Provider') ? 'provider-select' : 'model-select'}
      data-disabled={isDisabled ? 'true' : 'false'}
    >
      {value?.value || placeholder}
    </div>
  ),
}));

function renderModal() {
  return render(
    <SwitchModelModal
      sessionId="s1"
      onClose={vi.fn()}
      setView={vi.fn()}
      initialProvider="openai"
      initialModel="gpt-4o"
    />
  );
}

describe('SwitchModelModal on a browser-served surface', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([
      {
        name: 'openai',
        is_configured: true,
        provider_type: 'Commercial',
        affiliation: null,
        metadata: {
          name: 'openai',
          display_name: 'OpenAI',
          default_model: 'gpt-4o',
          known_models: [{ name: 'gpt-4o' }],
          config_keys: [],
          tier: 'public',
        },
      },
    ]);
    mocks.getProviderModels.mockResolvedValue(['gpt-4o']);
    mocks.read.mockResolvedValue('');
    mocks.changeModel.mockResolvedValue(true);
    mocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: false } });
    mocks.currentProvider = 'openai';
    resetProviderSetupAccessForTests();
  });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
    resetProviderSetupAccessForTests();
  });

  /**
   * The daemon refused this browser (or has not answered): the host owns the choice, as SD-1
   * had it for every browser.
   *
   * ⚠ **Fails against today's code**, where "Select model" is enabled the
   * moment the dialog opens (`isValid` starts `true`) and there is no note to
   * find. Today a browser user clicks it, `changeModel` fires, and the only
   * thing they see is the agent-facing 409 rendered as an error toast.
   */
  it('disables the confirm and explains why, before anything is clicked', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderModal();

    const note = await screen.findByTestId('host-managed-model-note');
    expect(note.textContent).toMatch(/biorouter configure/);
    expect(note.textContent).toMatch(/private conversation/i);

    expect(screen.getByRole('button', { name: 'Select model' })).toBeDisabled();
  });

  /**
   * The disabled attribute is a claim about the button; this is a claim about
   * the WRITE. `handleSubmit` carries its own guard for a keyboard submit or a
   * call site that renders its own confirm, and without asserting the call
   * count a rewrite could drop that guard and still pass the test above.
   *
   * ⚠ Fails against today's code: the click lands, `changeModel` is called
   * once, and the daemon answers 409.
   */
  it('never reaches changeModel even if the confirm is clicked', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderModal();

    await screen.findByTestId('host-managed-model-note');
    fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

    await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });

  /**
   * The pickers themselves, not only the confirm. A dialog whose selects still
   * respond while its one action is dead reads as a broken form rather than as
   * a decision made elsewhere.
   *
   * ⚠ Fails against today's code, which passes `isDisabled` only for
   * `loadingModels` — so both stubs report `false` once the catalog settles.
   */
  it('makes the provider and model pickers inert', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderModal();

    await screen.findByTestId('host-managed-model-note');
    await waitFor(() => {
      expect(screen.getByTestId('provider-select')).toHaveAttribute('data-disabled', 'true');
      expect(screen.getByTestId('model-select')).toHaveAttribute('data-disabled', 'true');
    });
  });

  /**
   * ⚠ **The control.** Passes before and after; it exists to catch a helper
   * that answers `browser` too readily. A false positive here does not merely
   * add a note — it takes the model picker away from every desktop user, which
   * is a far worse outcome than the 409 this change replaces.
   */
  it('leaves the desktop dialog fully usable', async () => {
    renderModal();

    await waitFor(() =>
      expect(screen.getByTestId('provider-select')).toHaveAttribute('data-disabled', 'false')
    );
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();

    const confirm = screen.getByRole('button', { name: 'Select model' });
    expect(confirm).toBeEnabled();
    fireEvent.click(confirm);
    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
  });

  /**
   * SD-18. A daemon that admits the served page takes `/config/set_provider`, so the dialog
   * works, but only for the default new chats start on: `/agent/update_provider` stays refused
   * in a browser, where an open chat keeps the model it started on. So the chat's half of the
   * choice is drawn unavailable with its reason, and the confirm changes the default.
   */
  describe('when the daemon allows provider setup', () => {
    beforeEach(async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      mocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: true } });
      await loadProviderSetupAccess();
    });

    it('changes the default for new chats and never this chat', async () => {
      renderModal();

      await waitFor(() =>
        expect(screen.getByTestId('provider-select')).toHaveAttribute('data-disabled', 'false')
      );
      expect(screen.queryByTestId('host-managed-model-note')).toBeNull();
      expect(screen.getByText(SWITCH_SCOPE_NEW_CHATS)).toBeInTheDocument();
      // The desktop's "Also use for new chats" box would say this chat changes; it does not.
      expect(screen.queryByTestId('switch-model-also-new-chats')).toBeNull();

      const confirm = screen.getByRole('button', { name: 'Select model' });
      expect(confirm).toBeEnabled();
      fireEvent.click(confirm);

      await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
      const [sessionId, model] = mocks.changeModel.mock.calls[0];
      expect(sessionId).toBeNull();
      expect(model).toMatchObject({ name: 'gpt-4o', provider: 'openai' });
    });

    it('shows the chat-scope switch disabled, with its reason, before anything is clicked', async () => {
      renderModal();

      const scope = await screen.findByTestId('switch-model-browser-chat-scope');
      const chatSwitch = screen.getByRole('checkbox', { name: USE_FOR_THIS_CHAT_LABEL });
      expect(chatSwitch).toBeDisabled();
      expect(chatSwitch).not.toBeChecked();
      expect(scope).toHaveTextContent(BROWSER_KEEPS_CHAT_MODEL);
      expect(chatSwitch).toHaveAccessibleDescription(BROWSER_KEEPS_CHAT_MODEL);
    });

    // The page states the host's default as its capability, so moving the default from a
    // private model to a public one hides private chats from it. Said before the switch.
    it('says before the switch that a public default hides private chats here', async () => {
      mocks.currentProvider = 'versa_azure';
      mocks.getProviders.mockResolvedValue([
        ...(await mocks.getProviders()),
        {
          name: 'versa_azure',
          is_configured: true,
          provider_type: 'Institutional',
          affiliation: null,
          metadata: {
            name: 'versa_azure',
            display_name: 'Versa (Azure)',
            default_model: 'gpt-5.5',
            known_models: [{ name: 'gpt-5.5' }],
            config_keys: [],
            tier: 'private',
          },
        },
      ]);
      renderModal();

      expect(await screen.findByTestId('switch-model-browser-hides-private')).toHaveTextContent(
        BROWSER_PUBLIC_DEFAULT_HIDES_PRIVATE
      );
    });

    it('says nothing about reach when the default stays public', async () => {
      renderModal();
      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled()
      );
      expect(screen.queryByTestId('switch-model-browser-hides-private')).toBeNull();
    });

    // An unsent chat starts on the default anyway, so a pick is not held for it (the held pick
    // would be bound with `/agent/update_provider` when the chat starts, which a browser never
    // sends), and no "keeps its model" line is drawn for a chat that has none yet.
    it('gives an unsent chat the default it will start on, holding no pick', async () => {
      const choose = vi.fn();
      render(
        <SwitchModelModal
          sessionId={null}
          onClose={vi.fn()}
          setView={vi.fn()}
          initialProvider="openai"
          initialModel="gpt-4o"
          onChooseForUnsentChat={choose}
        />
      );

      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled()
      );
      expect(screen.queryByTestId('switch-model-browser-chat-scope')).toBeNull();
      fireEvent.click(screen.getByRole('button', { name: 'Select model' }));

      await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
      expect(mocks.changeModel.mock.calls[0][0]).toBeNull();
      expect(choose).not.toHaveBeenCalled();
    });
  });
});
