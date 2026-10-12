/**
 * One coral button per tab with rows open on their own (vision FR-R5-2, principle 7).
 *
 * Since FR-R4-2 a row opens without closing another, and every first-run panel ends in its primary,
 * so two open panels drew two coral buttons in one view: "Download and start" beside "Use Ollama"
 * on Local, "Use Claude Code" beside "Use Codex" on Public. The panel the person turned to last
 * (opened, or moved focus into) holds the tab's one coral button; the others draw the same action
 * as secondary, still enabled.
 *
 * Coral is read as the `default` variant's fill class: the variant is the whole decision, and
 * jsdom paints nothing.
 */
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails, ProviderTier } from '../../../api';
import ProviderCatalog from './ProviderCatalog';
import type { CodingAgentAuth, CodingAgentAvailability } from '../../onboarding/codingAgentStatus';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';
import { resetLlamaServerStoreForTests } from '../models/llamaServerStore';

const mocks = vi.hoisted(() => ({
  getPrivacyDisclosure: vi.fn(),
  fetchCodingAgentStatus: vi.fn(),
  llamacppStatus: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  llamacppStatus: mocks.llamacppStatus,
  // The key box asks which providers a key can be detected for; its list is its own suite's.
  getDetectableProviders: vi.fn(async () => ({ data: { providers: [] } })),
}));
vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: async () => ({ provider: 'anthropic', model: 'm' }),
  }),
}));
vi.mock('../models/subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: () => <div data-testid="switch-model-modal" />,
}));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../../onboarding/codingAgentStatus', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  fetchCodingAgentStatus: mocks.fetchCodingAgentStatus,
}));
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ upsert: vi.fn(), read: vi.fn(async () => null), getProviders: vi.fn() }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('../../InAppTerminalDock', () => ({
  default: () => <div data-testid="in-app-terminal-dock" />,
}));
// Ollama is running and has its model, so its panel offers "Use Ollama" (never a real request).
vi.mock('../../../utils/ollamaDetection', () => ({
  checkOllamaStatus: vi.fn(async () => ({ isRunning: true, host: 'http://127.0.0.1:11434' })),
  getOllamaModels: vi.fn(async () => []),
  hasModel: vi.fn(async () => true),
  pullOllamaModel: vi.fn(async () => false),
  deleteOllamaModel: vi.fn(async () => false),
  pollForOllama: vi.fn(() => () => {}),
  getOllamaDownloadUrl: () => 'https://ollama.com/download',
  getPreferredModel: () => 'gpt-oss:20b',
}));

function provider(
  name: string,
  display: string,
  backend: { tier?: ProviderTier; runs_locally?: boolean; institutions?: { id: string }[] } = {},
  isConfigured = false
): ProviderDetails {
  return {
    name,
    is_configured: isConfigured,
    provider_type: 'Builtin',
    resolved_tier: null,
    unavailable_reason: null,
    metadata: {
      config_keys: [],
      default_model: '',
      description: '',
      display_name: display,
      known_models: [],
      model_doc_link: '',
      name,
      tier: backend.tier ?? 'public',
      runs_locally: backend.runs_locally ?? false,
      institutions: backend.institutions ?? [],
    },
  } as unknown as ProviderDetails;
}

const LOCAL = { tier: 'private' as ProviderTier, runs_locally: true };
const UCSF = {
  tier: 'private' as ProviderTier,
  institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
};

const signedIn = (kind: 'claude_code' | 'codex'): CodingAgentAvailability => ({
  kind,
  providerId: kind,
  displayName: kind === 'codex' ? 'Codex' : 'Claude Code',
  path: null,
  version: null,
  auth: { state: 'signed_in_subscription' } as CodingAgentAuth,
  loginCommand: `${kind} auth login`,
  installHint: `install ${kind}`,
});

const llamaStatus = {
  data: {
    sidecar: { state: 'stopped', warmed: false, build: 'test', detail: null, model: null },
    catalog: [
      {
        name: 'gemma4',
        display_name: 'Gemma 4 E4B',
        family: 'Gemma 4',
        ollama_name: 'gemma4:latest',
        official_url: '',
        hf_spec: 'google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0',
        download_size: '9.6 GB',
        description: 'Laptop default',
        min_gpu_memory_gib: 16,
        recommended_gpu_memory_gib: 16,
        context_limit: 131072,
        active_params_b: 4,
        speed_hint: 'Fast',
        is_default: true,
        downloaded: false,
        download_status: 'not_downloaded',
        download_source: 'none',
        fallback_downloaded: false,
        fallback_download_status: 'not_downloaded',
        model_path: null,
        suitable: true,
        suitability_status: 'suitable',
        suitability_message: 'Recommended for this machine.',
      },
    ],
    system: {
      os: 'macos',
      total_memory_gib: 64,
      accelerator_memory_gib: 64,
      accelerator_memory_kind: 'apple_unified',
      default_context_size: 131072,
      model_cache_dir: '/tmp/models',
      model_cache_layout: 'test',
    },
  },
};

/** Radix's TabsTrigger activates on mousedown, not on a synthetic click. */
const clickTab = (key: 'local' | 'institutional' | 'commercial') => {
  const trigger = screen.getByTestId(`catalog-tab-${key}`);
  fireEvent.mouseDown(trigger);
  fireEvent.click(trigger);
};

const isCoral = (button: HTMLElement) => button.classList.contains('bg-background-accent');
/** Every coral-filled button on the page: the rule is at most one. */
const coralButtons = () =>
  Array.from(document.querySelectorAll<HTMLElement>('button.bg-background-accent'));
const toggle = (name: string) => fireEvent.click(screen.getByTestId(`provider-row-toggle-${name}`));
const panel = (name: string) => screen.getByTestId(`provider-row-panel-${name}`);

beforeEach(() => {
  vi.clearAllMocks();
  resetLlamaServerStoreForTests();
  __resetDisclosureStoreForTests();
  mocks.llamacppStatus.mockResolvedValue(llamaStatus);
  mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [] });
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'L', short: 'S', acknowledged: true },
  });
});

afterEach(() => {
  resetLlamaServerStoreForTests();
});

describe('ProviderCatalog: one coral button per tab (FR-R5-2)', () => {
  it('gives Local one: the panel opened last holds it, and closing it hands it back', async () => {
    render(
      <ProviderCatalog
        providers={[
          provider('llamacpp', 'Llama Server', LOCAL),
          provider('ollama', 'Ollama', LOCAL),
        ]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    // Llama Server opens by default and holds the tab's primary.
    const start = await within(panel('llamacpp')).findByTestId('llamacpp-start');
    expect(isCoral(start)).toBe(true);

    // Opening Ollama keeps Llama Server open (FR-R4-2) and hands Ollama the coral button.
    toggle('ollama');
    const useOllama = await within(panel('ollama')).findByRole('button', { name: 'Use Ollama' });
    expect(isCoral(useOllama)).toBe(true);
    expect(isCoral(within(panel('llamacpp')).getByTestId('llamacpp-start'))).toBe(false);
    expect(within(panel('llamacpp')).getByTestId('llamacpp-start')).toBeEnabled();
    expect(coralButtons()).toEqual([useOllama]);

    // Closing Ollama hands it back to the panel still open.
    toggle('ollama');
    expect(coralButtons()).toEqual([within(panel('llamacpp')).getByTestId('llamacpp-start')]);
  });

  it('gives Public one between two ready agents, and focus moves it to the panel in use', async () => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({
      agents: [signedIn('claude_code'), signedIn('codex')],
    });
    render(
      <ProviderCatalog
        providers={[provider('claude_code', 'Claude Code'), provider('codex', 'Codex')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('commercial');
    await screen.findByTestId('provider-row-toggle-claude_code');

    toggle('claude_code');
    const useClaude = screen.getByTestId('coding-agent-connect-claude_code');
    expect(isCoral(useClaude)).toBe(true);

    toggle('codex');
    const useCodex = screen.getByTestId('coding-agent-connect-codex');
    expect(coralButtons()).toEqual([useCodex]);
    expect(useClaude).toBeEnabled();

    // Focus moving into Claude Code's panel is the person turning back to it.
    act(() => useClaude.focus());
    expect(coralButtons()).toEqual([useClaude]);

    // Closing the panel that holds it hands it to the one still open.
    toggle('claude_code');
    expect(coralButtons()).toEqual([useCodex]);
  });

  it('shares Public’s one between the key box and an open agent on first run', async () => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [signedIn('claude_code')] });
    render(
      <ProviderCatalog
        providers={[provider('claude_code', 'Claude Code'), provider('anthropic', 'Anthropic')]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    clickTab('commercial');
    await screen.findByTestId('provider-row-toggle-claude_code');
    toggle('claude_code');
    const useClaude = screen.getByTestId('coding-agent-connect-claude_code');
    const detect = screen.getByTestId('commercial-detect');
    expect(coralButtons()).toEqual([useClaude]);

    // Typing a key is working in the key box: its Detect takes the coral button.
    const keyField = screen.getByLabelText('API key');
    act(() => keyField.focus());
    fireEvent.change(keyField, { target: { value: 'sk-test' } });
    expect(coralButtons()).toEqual([detect]);

    // And back to the agent, with the key still typed.
    act(() => useClaude.focus());
    expect(coralButtons()).toEqual([useClaude]);
    expect(detect).toBeEnabled();
  });

  it('gives Institutional one between two Versa forms with their keys in', () => {
    render(
      <ProviderCatalog
        providers={[
          provider('versa_azure', 'Versa API Azure', UCSF),
          provider('versa_bedrock', 'Versa API Bedrock', UCSF),
        ]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    clickTab('institutional');
    toggle('versa_bedrock');
    toggle('versa_azure');

    const azureKey = within(panel('versa_azure')).getByLabelText('API key');
    act(() => azureKey.focus());
    fireEvent.change(azureKey, { target: { value: 'azure-test' } });
    const azureSave = within(panel('versa_azure')).getByTestId('institutional-connect');
    expect(coralButtons()).toEqual([azureSave]);

    const bedrockId = within(panel('versa_bedrock')).getByLabelText('Access key ID');
    act(() => bedrockId.focus());
    fireEvent.change(bedrockId, { target: { value: 'id-test' } });
    fireEvent.change(within(panel('versa_bedrock')).getByLabelText('Secret access key'), {
      target: { value: 'secret-test' },
    });
    const bedrockSave = within(panel('versa_bedrock')).getByTestId('institutional-connect');
    expect(coralButtons()).toEqual([bedrockSave]);
    expect(azureSave).toBeEnabled();
  });

  it('holds each tab’s one on its own: a panel opened on Public leaves Local’s coral', async () => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [signedIn('claude_code')] });
    render(
      <ProviderCatalog
        providers={[
          provider('llamacpp', 'Llama Server', LOCAL),
          provider('claude_code', 'Claude Code'),
        ]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    // A signed-in agent opens the catalog on Public once the probe answers (`defaultCatalogTab`).
    await screen.findByTestId('provider-row-toggle-claude_code');
    clickTab('local');
    expect(coralButtons()).toEqual([await screen.findByTestId('llamacpp-start')]);

    clickTab('commercial');
    toggle('claude_code');
    expect(coralButtons()).toEqual([screen.getByTestId('coding-agent-connect-claude_code')]);

    clickTab('local');
    expect(coralButtons()).toEqual([await screen.findByTestId('llamacpp-start')]);
  });
});
