import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails, ProviderTier } from '../../../../api';
import { CREW_MODEL_FIXED_TEXT } from '../../../ModelAndProviderContext';
import {
  PRIVATE_MODEL_IN_PUBLIC_CHAT_TIP,
  PUBLIC_MODEL_IN_PRIVATE_CHAT,
  SwitchModelModal,
} from './SwitchModelModal';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  changeModel: vi.fn(),
  crewState: null as string | null,
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
    read: mocks.read,
  }),
}));

vi.mock('../../../ModelAndProviderContext', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  useModelAndProvider: () => ({
    changeModel: mocks.changeModel,
    currentModel: null,
    currentProvider: 'anthropic',
  }),
}));

vi.mock('../../../crew/access/chatCrewAccess', () => ({
  useChatCrewAccessState: () => mocks.crewState,
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

function provider(name: string, tier: ProviderTier, displayName = name): ProviderDetails {
  return {
    name,
    is_configured: true,
    provider_type: 'Builtin',
    affiliation: null,
    metadata: {
      config_keys: [],
      default_model: '',
      description: '',
      display_name: displayName,
      known_models: [],
      model_doc_link: '',
      name,
      tier,
      runs_locally: false,
    },
  } as ProviderDetails;
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.crewState = null;
  mocks.getProviders.mockResolvedValue([
    provider('anthropic', 'public', 'Anthropic'),
    provider('versa_azure', 'private', 'Versa'),
  ]);
  mocks.getProviderModels.mockResolvedValue(['Claude Opus 4.8']);
  mocks.read.mockResolvedValue('');
  mocks.changeModel.mockResolvedValue(true);
});

// W2-PRV-15: neither the picker nor the chip said a Crew chat's model is fixed,
// and the refusal arrived as "... failed" plus "then try again".
describe('SwitchModelModal in a Crew chat', () => {
  it('uses the daemon’s own sentence', () => {
    const refusal = readFileSync(
      join(__dirname, '../../../../../../../crates/biorouter/src/crew/refusal.rs'),
      'utf8'
    );
    expect(refusal).toContain(`"${CREW_MODEL_FIXED_TEXT}"`);
  });

  it('says up front that the model is fixed, and offers no way to change it', async () => {
    mocks.crewState = 'active';
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="private"
        initialProvider="anthropic"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    const note = await screen.findByTestId('switch-model-crew-fixed');
    expect(note).toHaveTextContent(CREW_MODEL_FIXED_TEXT);
    const confirm = screen.getByRole('button', { name: 'Select model' });
    expect(confirm).toBeDisabled();
    expect(confirm.getAttribute('aria-describedby')).toContain(note.id);
    // react-select drops the combobox role from a disabled input, so read the
    // inputs themselves: none of them can be typed into or opened.
    const inputs = [...document.querySelectorAll('input:not([type="checkbox"])')];
    expect(inputs.length).toBeGreaterThan(0);
    for (const input of inputs) {
      expect(input).toBeDisabled();
    }
    expect(screen.queryByRole('combobox')).toBeNull();
    expect(screen.queryByTestId('switch-model-also-new-chats')).toBeNull();

    fireEvent.click(confirm);
    expect(mocks.changeModel).not.toHaveBeenCalled();
  });

  it('shows a refusal as its sentence, without "try again"', async () => {
    mocks.changeModel.mockImplementation(
      async (_session: string, _model: unknown, options?: { onRefusal?: (s: string) => void }) => {
        options?.onRefusal?.(CREW_MODEL_FIXED_TEXT);
        return false;
      }
    );
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="anthropic"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );

    await screen.findByText('Claude Opus 4.8');
    const confirm = screen.getByRole('button', { name: 'Select model' });
    await waitFor(() => expect(confirm).toBeEnabled());
    fireEvent.click(confirm);

    const error = await screen.findByTestId('switch-model-submit-error');
    expect(error).toHaveTextContent(CREW_MODEL_FIXED_TEXT);
    expect(error).not.toHaveTextContent(/try again/i);
  });

  it('is an ordinary picker for a chat with no Crew access', async () => {
    mocks.crewState = 'none';
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="anthropic"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );
    await screen.findByText('Claude Opus 4.8');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled());
    expect(screen.queryByTestId('switch-model-crew-fixed')).toBeNull();
  });
});

// W2-PRV-7: a public chat switched to a private model became private with its
// next message, and nothing said so before the switch.
describe('SwitchModelModal public to private', () => {
  it('says the chat becomes private before a private model is applied to a public chat', async () => {
    render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="versa_azure"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );
    const tip = await screen.findByTestId('switch-model-becomes-private');
    expect(tip).toHaveTextContent(PRIVATE_MODEL_IN_PUBLIC_CHAT_TIP);
    expect(PRIVATE_MODEL_IN_PUBLIC_CHAT_TIP).toMatch(/make it public from History/);
    // A tip, not a refusal.
    await screen.findByText('Claude Opus 4.8');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Select model' })).toBeEnabled());
  });

  it('says nothing for a public model, a private chat, or no chat', async () => {
    const { unmount } = render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="public"
        initialProvider="anthropic"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );
    await screen.findByText('Claude Opus 4.8');
    expect(screen.queryByTestId('switch-model-becomes-private')).toBeNull();
    unmount();

    const second = render(
      <SwitchModelModal
        sessionId="s1"
        privacyTier="private"
        initialProvider="versa_azure"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );
    await screen.findByText('Claude Opus 4.8');
    expect(screen.queryByTestId('switch-model-becomes-private')).toBeNull();
    second.unmount();

    render(
      <SwitchModelModal
        sessionId={null}
        initialProvider="versa_azure"
        onClose={vi.fn()}
        setView={vi.fn()}
      />
    );
    await screen.findByText('Claude Opus 4.8');
    expect(screen.queryByTestId('switch-model-becomes-private')).toBeNull();
  });

  it('names the way back on a barred row in a private chat', () => {
    expect(PUBLIC_MODEL_IN_PRIVATE_CHAT).toMatch(/private chat, so only private models/);
    expect(PUBLIC_MODEL_IN_PRIVATE_CHAT).toMatch(/Make it public from History/);
  });
});
