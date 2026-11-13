import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { ModelAndProviderProvider } from '../../../ModelAndProviderContext';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';

/**
 * W2-5. The model chip names a Llama Server model the way the model picker lists it.
 *
 * Measured: the picker listed "Qwen3.8 27B", and once it was chosen the chip under the composer
 * read `qwen3.8-27b`, because the chip only knew predefined-model aliases. The real
 * `ModelAndProviderProvider` is mounted here, so the catalog read, the name the context answers
 * with and the chip that draws it are tested as one path.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  getProviders: vi.fn(),
  refreshConfig: vi.fn(),
  llamacppStatus: vi.fn(),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  llamacppStatus: mocks.llamacppStatus,
  llamacppWarmup: vi.fn(),
  setConfigProvider: vi.fn(),
  updateAgentProvider: vi.fn(),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
  isUserActionRefusal: () => false,
}));
vi.mock('../../../../toasts', () => ({ toastError: vi.fn(), toastSuccess: vi.fn() }));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    getProviders: mocks.getProviders,
    refreshConfig: mocks.refreshConfig,
  }),
  usePrivacyTiersEnabled: () => true,
}));

vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

Object.defineProperty(window, 'appConfig', {
  writable: true,
  value: { get: () => undefined },
});

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

let selection = { model: 'qwen3.8-27b', provider: 'llamacpp' };

const providerEntry = (
  name: string,
  display: string,
  tier: 'private' | 'public',
  runsLocally = false
) => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: display, tier, runs_locally: runsLocally },
  affiliation: null,
  resolved_tier: tier,
});

const status = {
  data: {
    sidecar: { state: 'stopped', model: null, warmed: false, detail: null },
    catalog: [
      { name: 'qwen3.8-27b', display_name: 'Qwen3.8 27B', download_status: 'downloaded' },
      { name: 'gemma4-31b', display_name: 'Gemma 4 31B', download_status: 'not_downloaded' },
    ],
    system: {},
  },
};

function renderBar(props: {
  sessionId?: string;
  effectiveModel?: { provider: string; model: string };
}) {
  return render(
    <ModelAndProviderProvider>
      <ModelsBottomBar
        sessionId={props.sessionId ?? null}
        privacyTier={props.sessionId ? 'private' : undefined}
        effectiveModel={props.effectiveModel}
        dropdownRef={dropdownRef}
        setView={vi.fn()}
        alerts={[]}
      />
    </ModelAndProviderProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  selection = { model: 'qwen3.8-27b', provider: 'llamacpp' };
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_MODEL') return selection.model;
    if (key === 'BIOROUTER_PROVIDER') return selection.provider;
    return '';
  });
  mocks.getProviders.mockResolvedValue([
    providerEntry('llamacpp', 'Llama Server', 'private', true),
    providerEntry('versa_azure', 'Versa API Azure', 'private'),
  ]);
  mocks.refreshConfig.mockResolvedValue(undefined);
  mocks.llamacppStatus.mockResolvedValue(status);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'LONG', short: 'SHORT', acknowledged: true },
  });
});

describe('the model chip on a Llama Server model', () => {
  it('reads "Qwen3.8 27B", as the picker listed it, and keeps the id for hover and speech', async () => {
    renderBar({});
    const chip = await screen.findByTestId('model-chip');
    await waitFor(() => expect(chip).toHaveTextContent('Qwen3.8 27B'));
    expect(chip).not.toHaveTextContent('qwen3.8-27b');
    // The visible name leads the accessible name, and the id closes it in the tooltip's words.
    expect(chip).toHaveAccessibleName(/^Current model: Qwen3\.8 27B\b/);
    expect(chip.getAttribute('aria-label')).toMatch(/Model: qwen3\.8-27b$/);
  });

  it('names it the same way in the menu', async () => {
    renderBar({});
    const chip = await screen.findByTestId('model-chip');
    await waitFor(() => expect(chip).toHaveTextContent('Qwen3.8 27B'));
    fireEvent.pointerDown(chip, { button: 0, ctrlKey: false });
    const line = await screen.findByTestId('model-provider-line');
    const block = line.parentElement as HTMLElement;
    await waitFor(() => expect(block).toHaveTextContent('Qwen3.8 27B'));
    expect(block).not.toHaveTextContent('qwen3.8-27b');
    expect(line).toHaveTextContent('Llama Server');
  });

  it('names a chat bound to a local model while new chats start elsewhere', async () => {
    selection = { model: 'gpt-5.5-2026-04-24', provider: 'versa_azure' };
    renderBar({ sessionId: 's1', effectiveModel: { provider: 'llamacpp', model: 'gemma4-31b' } });
    const chip = await screen.findByTestId('model-chip');
    await waitFor(() => expect(chip).toHaveTextContent('Gemma 4 31B'));
    expect(chip.getAttribute('aria-label')).toMatch(/Model: gemma4-31b$/);
  });

  it('leaves every other model as it was: no catalog read, no id tail', async () => {
    selection = { model: 'gpt-5.5-2026-04-24', provider: 'versa_azure' };
    renderBar({});
    const chip = await screen.findByTestId('model-chip');
    await waitFor(() => expect(chip).toHaveTextContent('gpt-5.5'));
    expect(chip).toHaveAccessibleName(/^Current model: gpt-5\.5-2026-04-24\b/);
    expect(chip.getAttribute('aria-label')).not.toMatch(/Model: gpt/);
    expect(mocks.llamacppStatus).not.toHaveBeenCalled();
  });
});
