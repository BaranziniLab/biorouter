import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { switchModelCopy } from '../copy';
import { MODEL_COPY } from '../../../bottom_menu/copy';
import { resetReasoningEffortForTests } from '../../../../store/reasoningEffort';

/** The model chip's menu names a decision model for what it is. */

const mocks = vi.hoisted(() => ({
  read: vi.fn(async (_key: string, _isSecret: boolean) => '' as unknown),
  getProviders: vi.fn(async () => [] as unknown[]),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'gpt-5.5-2026-04-24',
    currentProvider: 'versa_azure',
    getCurrentModelAndProviderForDisplay: async () => ({
      model: 'gpt-5.5-2026-04-24',
      provider: 'Versa API Azure',
    }),
    getCurrentModelDisplayName: async () => 'gpt-5.5-2026-04-24',
    getCurrentProviderDisplayName: async () => 'Versa API Azure',
  }),
}));
vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const row = (name: string, tier: 'public' | 'private') => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: name, tier, runs_locally: tier === 'private' },
  affiliation: null,
  resolved_tier: tier,
});

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  resetReasoningEffortForTests();
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([
    row('versa_azure', 'private'),
    row('ollama_decision', 'private'),
    row('baseten_decision', 'public'),
    row('openai', 'public'),
  ]);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'LONG', short: 'SHORT', acknowledged: true },
  });
});

async function openMenu() {
  await screen.findByRole('button', { name: /Current model:/ });
  fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
  await screen.findByRole('menuitem', { name: /Change model/ });
}

describe('the model chip of a decision chat', () => {
  it('says the chat runs on a decision model', async () => {
    render(
      <ModelsBottomBar
        sessionId="chat-1"
        privacyTier="private"
        effectiveModel={{ provider: 'ollama_decision', model: 'tev1:0.8b' }}
        setView={vi.fn()}
      />
    );
    await openMenu();
    expect(screen.getByTestId('model-decision-line')).toHaveTextContent(
      `${switchModelCopy.decisionModel}: answers with a choice and its probabilities, not text`
    );
  });

  it('says nothing of the kind for a chat model', async () => {
    render(<ModelsBottomBar sessionId="chat-2" privacyTier="private" setView={vi.fn()} />);
    await openMenu();
    expect(screen.queryByTestId('model-decision-line')).toBeNull();
  });

  /** B-3: the menu offers nothing that means nothing in a decision chat. */
  it('offers no Effort and no Lead and worker', async () => {
    render(
      <ModelsBottomBar
        sessionId="chat-1"
        privacyTier="private"
        reasoningScope="session:chat-1"
        effectiveModel={{ provider: 'ollama_decision', model: 'tev1:0.8b' }}
        setView={vi.fn()}
      />
    );
    await openMenu();
    expect(screen.queryByRole('group', { name: MODEL_COPY.effortGroup })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: MODEL_COPY.leadWorker })).toBeNull();
    expect(screen.getByLabelText(/Current model/).getAttribute('aria-label')).not.toMatch(/Effort/);
  });

  it('draws no files disclosure line for a public decision model', async () => {
    render(
      <ModelsBottomBar
        sessionId="chat-3"
        privacyTier="public"
        effectiveModel={{ provider: 'baseten_decision', model: 'joey-1' }}
        setView={vi.fn()}
      />
    );
    await openMenu();
    await screen.findByTestId('model-decision-line');
    // Let the disclosure read settle, so its absence is the rule and not a race.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByTestId('non-private-model-chip-note')).toBeNull();
  });

  // R1-7, R1-9: the line said "Ollama decision models · Private model · On this
  // machine" and was cut with an ellipsis. It now says where the model runs, in the
  // words every decision surface uses, and wraps.
  it('says where a decision model runs, in full, without repeating it as an affiliation', async () => {
    mocks.getProviders.mockResolvedValue([
      { ...row('ollama_decision', 'private'), affiliation: { kind: 'local', institutions: [] } },
      row('baseten_decision', 'public'),
    ]);
    const { unmount } = render(
      <ModelsBottomBar
        sessionId="chat-5"
        privacyTier="private"
        effectiveModel={{ provider: 'ollama_decision', model: 'tev1:0.8b' }}
        setView={vi.fn()}
      />
    );
    await openMenu();
    const line = await screen.findByTestId('model-provider-line');
    await screen.findByText(/Runs on this computer · Private model/);
    expect(line).toHaveTextContent(/^Runs on this computer · Private model$/);
    expect(line).not.toHaveTextContent('On this machine');
    expect(line.querySelector('.truncate')).toBeNull();
    unmount();

    render(
      <ModelsBottomBar
        sessionId="chat-6"
        privacyTier="public"
        effectiveModel={{ provider: 'baseten_decision', model: 'decision-models/joey-1' }}
        setView={vi.fn()}
      />
    );
    await openMenu();
    await screen.findByText(/Runs on Baseten · Public model/);
    expect(screen.getAllByText('joey-1')).toHaveLength(2); // the chip and the menu
    expect(screen.queryByText('decision-models/joey-1')).toBeNull();
  });

  it('keeps Effort, Lead and worker and the disclosure for a chat model', async () => {
    render(
      <ModelsBottomBar
        sessionId="chat-4"
        privacyTier="public"
        reasoningScope="session:chat-4"
        effectiveModel={{ provider: 'openai', model: 'gpt-5.5' }}
        setView={vi.fn()}
      />
    );
    await openMenu();
    expect(screen.getByRole('group', { name: MODEL_COPY.effortGroup })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: MODEL_COPY.leadWorker })).toBeInTheDocument();
    expect(await screen.findByTestId('non-private-model-chip-note')).toHaveTextContent('SHORT');
  });
});

describe('a public chat its default decision model made public', () => {
  it('keeps the dot of "joey-1 · Baseten" on the line of the word before it', async () => {
    render(
      <ModelsBottomBar
        sessionId="chat-5"
        privacyTier="public"
        privacyBasis={{
          source: 'default',
          label: '',
          model: 'joey-1',
          place: 'Baseten',
          summary: 'its default decision model is joey-1, a Baseten model',
        }}
        effectiveModel={{ provider: 'openai', model: 'gpt-5.5' }}
        setView={vi.fn()}
      />
    );
    await openMenu();
    const line = screen.getByTestId('chat-privacy-line');
    // Live QA 2026-11-12: at the menu's 288px this wrapped as "… is joey-1" / "· Baseten.".
    // The space before the dot does not break, so a wrap now falls after it.
    expect(line.textContent).toBe(
      'Public chat: its default decision model is joey-1\u00a0· Baseten.'
    );
    expect(line.textContent).not.toMatch(/ ·/);
  });
});
