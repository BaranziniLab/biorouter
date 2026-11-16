import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SwitchModelModal } from './SwitchModelModal';
import { switchModelCopy } from '../copy';

/**
 * The Switch models dialog's anatomy (codex simplicity redesign, spec 3.13): a 480px ModalShell
 * with no title icon, labelled "Provider" and "Model" fields, a real radio group where a
 * deployment pins its models, `Note`s for the crew and submit messages, and the "Also use for
 * new chats" explanation behind an InfoTip. The logic the dialog guards (privacy pre-flight,
 * unavailable providers, switch feedback) has its own files; this one holds the shape.
 */

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
  predefined: false,
  predefinedModels: null as null | { id: string; name: string; alias?: string; provider: string }[],
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
    read: mocks.read,
  }),
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  CREW_MODEL_FIXED_TEXT: 'A chat with Crew access keeps its model.',
  useModelAndProvider: () => ({
    changeModel: mocks.changeModel,
    currentModel: 'gpt-5.6-sol',
    currentProvider: 'versa_azure',
  }),
}));

vi.mock('../../../crew/access/chatCrewAccess', () => ({
  useChatCrewAccessState: () => 'none',
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () =>
    mocks.predefinedModels ?? [
      {
        id: 'a',
        name: 'model-a',
        alias: 'Model A (recommended)',
        provider: 'Versa',
        subtext: 'Fast',
      },
      { id: 'b', name: 'model-b', alias: 'Model B', provider: 'Versa' },
    ],
  shouldShowPredefinedModels: () => mocks.predefined,
}));

const provider = {
  name: 'versa_azure',
  is_configured: true,
  provider_type: 'Institutional',
  metadata: {
    name: 'versa_azure',
    display_name: 'Versa Azure',
    default_model: 'gpt-5.6-sol',
    known_models: [{ name: 'gpt-5.6-sol' }],
    allows_unlisted_models: false,
    config_keys: [],
  },
};

describe('SwitchModelModal anatomy', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.predefined = false;
    mocks.predefinedModels = null;
    mocks.getProviders.mockResolvedValue([provider]);
    mocks.getProviderModels.mockResolvedValue(['gpt-5.6-sol']);
    mocks.read.mockResolvedValue('');
    mocks.changeModel.mockResolvedValue(true);
  });

  it('is a 480px form dialog, titled "Switch models", with no title icon', async () => {
    render(
      <SwitchModelModal
        sessionId={null}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="versa_azure"
        initialModel="gpt-5.6-sol"
      />
    );
    const dialog = await screen.findByRole('dialog', { name: switchModelCopy.title });
    expect(dialog).toHaveAttribute('data-size', 'md');
    expect(
      within(dialog).getByRole('heading', { name: switchModelCopy.title })
    ).toBeInTheDocument();
    expect(dialog.querySelector('[data-slot="dialog-title"] svg')).toBeNull();
  });

  it('gives the focus back to the control that opened it, as the shared dialog does for every one', async () => {
    const user = userEvent.setup();
    function Opener() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Change…
          </button>
          {open ? (
            <SwitchModelModal
              sessionId={null}
              onClose={() => setOpen(false)}
              setView={vi.fn()}
              initialProvider="versa_azure"
              initialModel="gpt-5.6-sol"
            />
          ) : null}
        </>
      );
    }
    render(<Opener />);
    const opener = screen.getByRole('button', { name: 'Change…' });
    await user.click(opener);
    await screen.findByRole('dialog', { name: switchModelCopy.title });

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(opener).toHaveFocus();
  });

  it('names its two selects "Provider" and "Model"', async () => {
    render(
      <SwitchModelModal
        sessionId={null}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="versa_azure"
        initialModel="gpt-5.6-sol"
      />
    );
    expect(
      await screen.findByRole('combobox', { name: switchModelCopy.provider })
    ).toBeInTheDocument();
    expect(
      await screen.findByRole('combobox', { name: switchModelCopy.model })
    ).toBeInTheDocument();
  });

  it('explains "Also use for new chats" in an InfoTip, not a line under it', async () => {
    render(
      <SwitchModelModal
        sessionId="s-1"
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="versa_azure"
        initialModel="gpt-5.6-sol"
      />
    );
    const box = await screen.findByRole('checkbox', { name: 'Also use for new chats' });
    expect(box).not.toBeChecked();
    expect(
      screen.getByRole('button', { name: 'About Also use for new chats' })
    ).toBeInTheDocument();
    // The InfoTip is the label's sibling, so a click on it cannot tick the box.
    expect(box.closest('label')?.querySelector('.br-info-tip')).toBeNull();
  });

  describe('where a deployment pins its models', () => {
    beforeEach(() => {
      mocks.predefined = true;
    });

    it('is a labelled radio group, and picking a row enables the confirm', async () => {
      const onModelSelected = vi.fn();
      render(
        <SwitchModelModal
          sessionId={null}
          onClose={vi.fn()}
          setView={vi.fn()}
          onModelSelected={onModelSelected}
        />
      );
      const group = await screen.findByRole('radiogroup', { name: switchModelCopy.model });
      const radios = within(group).getAllByRole('radio');
      expect(radios).toHaveLength(2);
      expect(radios[0]).not.toBeChecked();
      expect(screen.getByText('Fast')).toBeInTheDocument();

      const confirm = screen.getByRole('button', { name: switchModelCopy.select });
      fireEvent.click(radios[1]);
      expect(radios[1]).toBeChecked();
      await waitFor(() => expect(confirm).toBeEnabled());
    });

    it('names a model with no alias by its display name, with its id under it', async () => {
      // T1 F2: the name the chip, Settings and every other surface show, never the raw id alone.
      mocks.predefinedModels = [
        { id: 'h', name: 'us.anthropic.claude-haiku-5-5', provider: 'versa_bedrock' },
        { id: 'b', name: 'model-b', alias: 'Model B', provider: 'versa_bedrock' },
      ];
      render(<SwitchModelModal sessionId={null} onClose={vi.fn()} setView={vi.fn()} />);
      const group = await screen.findByRole('radiogroup', { name: switchModelCopy.model });
      const [haiku, aliased] = within(group)
        .getAllByRole('radio')
        .map((radio) => radio.closest('label')!);
      expect(within(haiku).getByText('Claude Haiku 5.5')).toHaveClass('text-label');
      expect(within(haiku).getByTestId('predefined-model-id')).toHaveTextContent(
        'us.anthropic.claude-haiku-5-5'
      );
      expect(within(haiku).getByTestId('predefined-model-id')).toHaveClass(
        'font-mono',
        'text-code'
      );
      // An alias is the deployment's own name for it; its id sits under it the same way.
      expect(within(aliased).getByText('Model B')).toBeInTheDocument();
      expect(within(aliased).getByTestId('predefined-model-id')).toHaveTextContent('model-b');
    });

    it('marks a recommended alias with a quiet word, not a badge', async () => {
      render(<SwitchModelModal sessionId={null} onClose={vi.fn()} setView={vi.fn()} />);
      await screen.findByRole('radiogroup', { name: switchModelCopy.model });
      expect(screen.getByText(switchModelCopy.recommended)).toBeInTheDocument();
    });
  });
});
