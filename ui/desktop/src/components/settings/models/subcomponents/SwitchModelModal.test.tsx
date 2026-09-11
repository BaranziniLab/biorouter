import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ALSO_FOR_NEW_CHATS_HINT,
  ALSO_FOR_NEW_CHATS_LABEL,
  SWITCH_SCOPE_NEW_CHATS,
  SWITCH_SCOPE_THIS_CHAT,
  SwitchModelModal,
} from './SwitchModelModal';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
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
    currentModel: 'claude-old',
    currentProvider: 'anthropic',
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

vi.mock('../../../ui/Select', () => ({
  Select: ({ value, placeholder }: { value?: { value?: string } | null; placeholder?: string }) => (
    <div data-testid={placeholder?.startsWith('Provider') ? 'provider-select' : 'model-select'}>
      {value?.value || placeholder}
    </div>
  ),
}));

describe('SwitchModelModal onboarding initialization', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([
      {
        name: 'openai',
        is_configured: true,
        provider_type: 'Commercial',
        metadata: {
          name: 'openai',
          display_name: 'OpenAI',
          default_model: 'gpt-4o',
          known_models: [{ name: 'gpt-4o' }],
          allows_unlisted_models: false,
          config_keys: [],
        },
      },
    ]);
    mocks.getProviderModels.mockResolvedValue(['gpt-4o']);
    mocks.read.mockResolvedValue('');
    mocks.changeModel.mockResolvedValue(true);
  });

  it('refreshes the provider catalog when onboarding supplies a detected provider', async () => {
    render(
      <SwitchModelModal
        sessionId={null}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="openai"
        initialModel="gpt-4o"
      />
    );

    await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
    expect(mocks.getProviders).toHaveBeenNthCalledWith(1, true);
    await waitFor(() => {
      expect(document.querySelector('[data-testid="provider-select"]')).toHaveTextContent('openai');
      expect(document.querySelector('[data-testid="model-select"]')).toHaveTextContent('gpt-4o');
    });
  });
});

/**
 * What the dialog does while a bind is in flight, and what it says when one
 * fails.
 *
 * Reported as *"the Select Model button actually froze without telling me
 * why"* against Versa API Azure. The button had not frozen: `changeModel` was
 * awaiting the daemon, the button carried no pending state to show it, and on
 * failure `handleSubmit` returned leaving the dialog byte-for-byte as it was.
 * Every report the user got lived in a toast in the opposite corner of the
 * screen — including `TypeError: Failed to fetch`, which is what the underlying
 * fault (a runaway catalogue poll exhausting the renderer's socket pool)
 * produced. A dialog that cannot say "working" or "that failed" is
 * indistinguishable from a dead one.
 */
describe('SwitchModelModal switch feedback', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([
      {
        name: 'versa_azure',
        is_configured: true,
        provider_type: 'Institutional',
        metadata: {
          name: 'versa_azure',
          display_name: 'Versa API Azure',
          default_model: 'gpt-5.5-2026-04-24',
          known_models: [{ name: 'gpt-5.5-2026-04-24' }],
          allows_unlisted_models: false,
          config_keys: [],
        },
      },
    ]);
    mocks.getProviderModels.mockResolvedValue(['gpt-5.5-2026-04-24']);
    mocks.read.mockResolvedValue('');
  });

  const renderModal = (onClose = vi.fn()) =>
    render(
      <SwitchModelModal
        sessionId="s-1"
        onClose={onClose}
        setView={vi.fn()}
        initialProvider="versa_azure"
        initialModel="gpt-5.5-2026-04-24"
      />
    );

  const clickSelect = async () => {
    const button = await waitFor(() => {
      const found = screen.getAllByRole('button').find((el) => el.textContent === 'Select model');
      if (!found) throw new Error('Select model button not rendered');
      return found;
    });
    fireEvent.click(button);
    return button;
  };

  it('shows the switch is running, and refuses a second one while it is', async () => {
    let release: (ok: boolean) => void = () => {};
    mocks.changeModel.mockImplementation(
      () =>
        new Promise<boolean>((resolve) => {
          release = resolve;
        })
    );

    renderModal();
    const button = await clickSelect();

    // The label is the whole point: a bind crosses the network twice and the
    // user must be able to tell "working" from "ignored me".
    await waitFor(() => expect(button).toHaveTextContent('Switching'));
    expect(button).toBeDisabled();

    // A second click must not start a second bind — two racing binds write two
    // providers through `/config/set_provider` and the loser wins.
    fireEvent.click(button);
    expect(mocks.changeModel).toHaveBeenCalledTimes(1);

    await act(async () => {
      release(true);
    });
  });

  it('says so in the dialog when the switch did not happen', async () => {
    mocks.changeModel.mockResolvedValue(false);
    const onClose = vi.fn();

    renderModal(onClose);
    await clickSelect();

    await waitFor(() =>
      expect(screen.getByTestId('switch-model-submit-error')).toHaveTextContent(
        'The model was not switched'
      )
    );
    // A refused switch leaves the dialog open — closing it would hide the one
    // control that can retry.
    expect(onClose).not.toHaveBeenCalled();
    // And the button must come back, or the dialog is a dead end.
    await waitFor(() =>
      expect(
        screen.getAllByRole('button').find((el) => el.textContent === 'Select model')
      ).toBeEnabled()
    );
  });

  /**
   * ⚠ `handleSubmit` is wired straight to `onClick`, so nothing holds the
   * promise it returns. Before this, a throw anywhere inside it became an
   * unhandled rejection and the dialog did not move — the exact silent freeze
   * that was reported.
   */
  it('reports a thrown failure instead of leaving an unhandled rejection', async () => {
    mocks.changeModel.mockRejectedValue(new Error('Failed to fetch'));

    const unhandled: unknown[] = [];
    const onUnhandled = (event: PromiseRejectionEvent) => {
      unhandled.push(event.reason);
      event.preventDefault();
    };
    window.addEventListener('unhandledrejection', onUnhandled);

    try {
      renderModal();
      await clickSelect();

      await waitFor(() =>
        expect(screen.getByTestId('switch-model-submit-error')).toHaveTextContent('Failed to fetch')
      );
      await act(async () => {
        await Promise.resolve();
      });
    } finally {
      window.removeEventListener('unhandledrejection', onUnhandled);
    }

    expect(unhandled).toEqual([]);
  });
});

/**
 * F3 / `docs/security/privacy-tiers.md` §14.3 P4 — the dialog says what a
 * switch changes, before the user commits.
 *
 * Until 2026-09-11 a switch made from a chat's composer also rewrote the model
 * every new chat starts on, in every window, with nothing on screen to say so:
 * provider QA F bound Claude Code in one chat for one check, and the next chat
 * it opened came up public. The coupling is now an explicit, unticked box, and
 * the dialog opened with no chat says plainly that it is the app-wide choice.
 */
describe('SwitchModelModal — what the switch changes', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([
      {
        name: 'versa_azure',
        is_configured: true,
        provider_type: 'Institutional',
        metadata: {
          name: 'versa_azure',
          display_name: 'Versa API Azure',
          default_model: 'gpt-5.5-2026-04-24',
          known_models: [{ name: 'gpt-5.5-2026-04-24' }],
          allows_unlisted_models: false,
          config_keys: [],
        },
      },
    ]);
    mocks.getProviderModels.mockResolvedValue(['gpt-5.5-2026-04-24']);
    mocks.read.mockResolvedValue('');
    mocks.changeModel.mockResolvedValue(true);
  });

  const renderModal = (sessionId: string | null) =>
    render(
      <SwitchModelModal
        sessionId={sessionId}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="versa_azure"
        initialModel="gpt-5.5-2026-04-24"
      />
    );

  const confirm = () =>
    waitFor(() => {
      const found = screen.getAllByRole('button').find((el) => el.textContent === 'Select model');
      if (!found) throw new Error('Select model button not rendered');
      return found;
    });

  /** Let the provider list land, and the model list after it, inside act. */
  const settle = async () => {
    await waitFor(() =>
      expect(screen.getByTestId('provider-select')).toHaveTextContent('versa_azure')
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  it('from a chat, says it switches this chat and offers new chats as an unticked box', async () => {
    renderModal('s-1');

    expect(screen.getByText(SWITCH_SCOPE_THIS_CHAT)).toBeInTheDocument();
    const box = screen.getByRole('checkbox', { name: new RegExp(ALSO_FOR_NEW_CHATS_LABEL) });
    expect(box).not.toBeChecked();
    expect(screen.getByText(ALSO_FOR_NEW_CHATS_HINT)).toBeInTheDocument();
    await settle();
  });

  /**
   * ⚠ Found by driving the running app, not by a test. `Checkbox` draws its
   * square beside an `sr-only` input; with the label as a SIBLING (`htmlFor`)
   * only the words toggled it, and a click on the square — the thing a person
   * aims at — did nothing at all.
   */
  it('ticks when the square itself is clicked, not only its words', async () => {
    renderModal('s-1');
    const box = screen.getByRole('checkbox', { name: new RegExp(ALSO_FOR_NEW_CHATS_LABEL) });
    // The square: Checkbox's own 24px target, which holds the hidden input.
    fireEvent.click(box.parentElement as HTMLElement);
    expect(box).toBeChecked();
    await settle();
  });

  it('leaves new chats alone unless the box is ticked', async () => {
    renderModal('s-1');
    fireEvent.click(await confirm());

    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
    expect(mocks.changeModel).toHaveBeenCalledWith(
      's-1',
      expect.objectContaining({ name: 'gpt-5.5-2026-04-24', provider: 'versa_azure' }),
      { alsoForNewChats: false }
    );
  });

  it('carries a ticked box through to the switch', async () => {
    renderModal('s-1');
    fireEvent.click(screen.getByRole('checkbox', { name: new RegExp(ALSO_FOR_NEW_CHATS_LABEL) }));
    fireEvent.click(await confirm());

    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
    expect(mocks.changeModel).toHaveBeenCalledWith(
      's-1',
      expect.objectContaining({ name: 'gpt-5.5-2026-04-24' }),
      { alsoForNewChats: true }
    );
  });

  /**
   * With no chat — Home, a chat not started, Settings → Models, onboarding —
   * the only thing a switch can change is the model new chats start on, so
   * there is no box to offer, and the description says how far it reaches.
   */
  it('with no chat, says it sets the model new chats start on in every window', async () => {
    renderModal(null);

    expect(screen.getByText(SWITCH_SCOPE_NEW_CHATS)).toBeInTheDocument();
    expect(screen.queryByRole('checkbox')).toBeNull();

    await settle();
    fireEvent.click(await confirm());
    await waitFor(() => expect(mocks.changeModel).toHaveBeenCalledTimes(1));
    expect(mocks.changeModel).toHaveBeenCalledWith(
      null,
      expect.objectContaining({ name: 'gpt-5.5-2026-04-24' })
    );
  });
});
