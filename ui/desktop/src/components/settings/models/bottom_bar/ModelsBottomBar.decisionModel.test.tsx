import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { switchModelCopy } from '../copy';

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
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([
    row('versa_azure', 'private'),
    row('ollama_decision', 'private'),
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
});
