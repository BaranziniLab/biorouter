import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../api';
import type Model from '../models/modelInterface';
import ProviderCatalog from './ProviderCatalog';
import { __resetHeldChatModelsForTests, heldChatModel } from '../models/pendingChatModel';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';

/**
 * T3-SH-2. "Use other provider" from a chat not sent yet opened this catalog
 * with nothing to say which chat it came from, so its model step wrote the model
 * every new chat starts on (`BIOROUTER_PROVIDER: claude_code`, measured). It now
 * holds the pick for that chat, by its tab, exactly as the chat's own picker
 * does.
 */
const mocks = vi.hoisted(() => ({
  modalProps: null as Record<string, unknown> | null,
  fetchCodingAgentStatus: vi.fn(),
  upsert: vi.fn(),
  read: vi.fn(),
  getPrivacyDisclosure: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
}));
vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: async () => ({ provider: 'versa_azure', model: 'm' }),
  }),
}));
// The model step itself is `SwitchModelModal`'s suite's business; this suite
// asks what the catalog hands it.
vi.mock('../models/subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: (props: Record<string, unknown>) => {
    mocks.modalProps = props;
    return <div data-testid="switch-model-modal" />;
  },
}));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../../onboarding/codingAgentStatus', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  fetchCodingAgentStatus: mocks.fetchCodingAgentStatus,
}));
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    upsert: mocks.upsert,
    read: mocks.read,
    getProviders: vi.fn(async () => []),
  }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('../../InAppTerminalDock', () => ({
  default: () => <div data-testid="in-app-terminal-dock" />,
}));
vi.mock('../../../utils/ollamaDetection', () => ({
  checkOllamaStatus: vi.fn(async () => ({ isRunning: false, host: 'http://127.0.0.1:11434' })),
  getOllamaModels: vi.fn(async () => []),
  hasModel: vi.fn(async () => false),
  pullOllamaModel: vi.fn(async () => false),
  deleteOllamaModel: vi.fn(async () => false),
  pollForOllama: vi.fn(() => () => {}),
  getOllamaDownloadUrl: () => 'https://ollama.com/download',
  getPreferredModel: () => 'gpt-oss:20b',
}));

const codex = {
  name: 'codex',
  is_configured: false,
  provider_type: 'Builtin',
  resolved_tier: null,
  unavailable_reason: null,
  metadata: {
    config_keys: [],
    default_model: '',
    description: '',
    display_name: 'Codex',
    known_models: [],
    model_doc_link: '',
    name: 'codex',
    tier: 'public',
    runs_locally: false,
    institutions: [],
  },
} as unknown as ProviderDetails;

const clickTab = (key: 'local' | 'institutional' | 'commercial') => {
  const trigger = screen.getByTestId(`catalog-tab-${key}`);
  fireEvent.mouseDown(trigger);
  fireEvent.click(trigger);
};

async function openModelStep(props: {
  chatSessionId?: string | null;
  heldChatTabId?: string | null;
}) {
  render(
    <ProviderCatalog
      providers={[codex]}
      mode="settings"
      configuredProvider={null}
      refreshProviders={vi.fn()}
      {...props}
    />
  );
  clickTab('commercial');
  fireEvent.click(await screen.findByTestId('provider-row-toggle-codex'));
  fireEvent.click(await screen.findByTestId('coding-agent-connect-codex'));
  await screen.findByTestId('switch-model-modal');
  return mocks.modalProps as {
    sessionId: string | null;
    onChooseForUnsentChat?: (model: Model) => void;
    unsentChatTabId?: string;
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.modalProps = null;
  __resetHeldChatModelsForTests();
  __resetDisclosureStoreForTests();
  mocks.read.mockResolvedValue(null);
  mocks.upsert.mockResolvedValue(undefined);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'L', short: 'S', acknowledged: true },
  });
  mocks.fetchCodingAgentStatus.mockResolvedValue({
    agents: [
      {
        kind: 'codex',
        providerId: 'codex',
        displayName: 'Codex',
        path: null,
        version: null,
        auth: { state: 'signed_in_subscription' },
        loginCommand: 'codex auth login',
        installHint: 'install codex',
      },
    ],
  });
});

afterEach(() => {
  __resetHeldChatModelsForTests();
});

describe('the model step of a catalog opened from a chat not sent yet', () => {
  it('holds the pick for that chat instead of offering the new-chats scope', async () => {
    const modal = await openModelStep({ heldChatTabId: 'tab-3' });

    expect(modal.sessionId).toBeNull();
    expect(modal.onChooseForUnsentChat).toBeTypeOf('function');
    // A second "Use other provider" from here still names the same chat.
    expect(modal.unsentChatTabId).toBe('tab-3');

    const pick: Model = { name: 'gpt-6-sol', provider: 'codex' };
    act(() => modal.onChooseForUnsentChat?.(pick));
    expect(heldChatModel('tab-3')).toBe(pick);
    expect(heldChatModel('tab-4')).toBeNull();
  });

  it('opened from a started chat, switches that chat and holds nothing', async () => {
    const modal = await openModelStep({ chatSessionId: 's-7', heldChatTabId: 'tab-3' });
    expect(modal.sessionId).toBe('s-7');
    expect(modal.onChooseForUnsentChat).toBeUndefined();
    expect(modal.unsentChatTabId).toBeUndefined();
  });

  it('opened from Settings, keeps the new-chats scope', async () => {
    const modal = await openModelStep({});
    expect(modal.sessionId).toBeNull();
    expect(modal.onChooseForUnsentChat).toBeUndefined();
  });
});
