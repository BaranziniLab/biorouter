import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';

/**
 * D6 of the 2026-09-12 model-controls run, and the most dangerous of that batch.
 *
 * The "Change model" dialog opened from inside a chat is headed *"Select a
 * provider and model for this chat."* and pre-filled the APP-WIDE selection, so
 * pressing "Select model" without touching anything moved the chat to a model
 * the user never chose. Model identity decides the privacy tier, so this is
 * correctness, not cosmetics.
 *
 * Measured live against `origin/main` (dev GUI, sandboxed config, 2026-09-12):
 * chat `20260610_28` — session row `provider_name = versa_azure`,
 * `model_config_json.model_name = gpt-5.2-2025-12-11`, `privacy_tier = private`
 * — with `BIOROUTER_MODEL = gpt-5.5-2026-04-24` and
 * `BIOROUTER_PROVIDER = versa_azure`. The composer chip read
 * `gpt-5.2-2025-12-11`; the dialog opened beneath it read
 * `Select a provider and model for this chat.` over **`gpt-5.5-2026-04-24`**.
 *
 * The dialog is stubbed here and its props are the assertion. That is
 * deliberate: what the dialog OPENS ON is a property of the hand-off, and the
 * real modal's two selects load asynchronously from a provider catalog whose
 * absence would make a passing test say nothing about which pair was passed.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(async (_key: string, _isSecret: boolean) => '' as unknown),
  getProviders: vi.fn(async () => [] as unknown[]),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  switchModelProps: [] as Record<string, unknown>[],
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

/** The app-wide selection — what the chip used to state everywhere. */
const SELECTION = { model: 'gpt-5.5-2026-04-24', provider: 'versa_azure' };
/** What chat `20260610_28`'s own row named, and what it really runs on. */
const CHAT_BINDING = { model: 'gpt-5.2-2025-12-11', provider: 'versa_azure' };

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: SELECTION.model,
    currentProvider: SELECTION.provider,
    getCurrentModelAndProviderForDisplay: async () => ({
      model: SELECTION.model,
      provider: 'Versa API Azure',
    }),
    getCurrentModelDisplayName: async () => SELECTION.model,
    getCurrentProviderDisplayName: async () => 'Versa API Azure',
  }),
}));

vi.mock('../subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: (props: Record<string, unknown>) => {
    mocks.switchModelProps.push(props);
    return <div data-testid="switch-model-modal" />;
  },
}));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.switchModelProps = [];
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([
    {
      name: 'versa_azure',
      is_configured: true,
      provider_type: 'Builtin',
      metadata: {
        name: 'versa_azure',
        display_name: 'Versa API Azure',
        tier: 'private',
        runs_locally: false,
      },
      affiliation: null,
      resolved_tier: 'private',
    },
  ]);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'LONG', short: 'SHORT', acknowledged: true },
  });
  // No lead/worker pair configured, so nothing else competes for the chip.
  mocks.read.mockImplementation(async (key: string) =>
    key === 'BIOROUTER_MODEL' ? SELECTION.model : ''
  );
});

/**
 * `ModelsBottomBar` renders the dialog only from its own dropdown, and Radix's
 * trigger opens on `pointerdown`, not on `click` — the same gesture
 * `ModelsBottomBar.browserSurface.test.tsx` uses on this chip.
 */
async function openChangeModel() {
  await screen.findByRole('button', { name: /Current model:/ });
  fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
  fireEvent.click(await screen.findByRole('menuitem', { name: /Change model/ }));
  await screen.findByTestId('switch-model-modal');
  return mocks.switchModelProps[mocks.switchModelProps.length - 1];
}

describe('the Change model dialog opened from a chat', () => {
  it("opens on the chat's own binding, not the app-wide selection", async () => {
    render(
      <ModelsBottomBar
        sessionId="20260610_28"
        privacyTier="private"
        effectiveModel={CHAT_BINDING}
        dropdownRef={dropdownRef}
        setView={vi.fn()}
        alerts={[]}
      />
    );
    // The chip already states the binding; the dialog has to agree with it.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Current model:/ })).toHaveAccessibleName(
        /gpt-5\.2-2025-12-11/
      )
    );

    const props = await openChangeModel();
    expect(props.sessionId).toBe('20260610_28');
    expect(props.initialProvider).toBe(CHAT_BINDING.provider);
    expect(props.initialModel).toBe(CHAT_BINDING.model);
    // Without this the case above passes for a dialog handed a hard-coded pair.
    expect(props.initialModel).not.toBe(SELECTION.model);
  });

  /**
   * A chat whose binding agrees with the selection, or which has none of its own,
   * arrives with `effectiveModel` unset (`usePinnedModel` sets it only on a
   * difference). Passing `undefined` through is correct there — the modal's own
   * fallback resolves to the same pair — and asserting it stops the fix from
   * being written as a hard-coded value.
   */
  it('passes nothing through for a chat that runs the selection', async () => {
    render(
      <ModelsBottomBar
        sessionId="20260610_29"
        privacyTier="public"
        dropdownRef={dropdownRef}
        setView={vi.fn()}
        alerts={[]}
      />
    );
    const props = await openChangeModel();
    expect(props.sessionId).toBe('20260610_29');
    expect(props.initialProvider).toBeUndefined();
    expect(props.initialModel).toBeUndefined();
  });
});
