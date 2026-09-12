import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { announceAppModelSelection } from '../../../../utils/sessionBindingSync';

/**
 * The chip's lead/worker statement has to follow the app-wide selection, not just
 * its value — and it has to be TRUE.
 *
 * # Two defects, one file
 *
 * **#279.** Measured live on 2026-09-12, two windows of one instance: Settings →
 * Models → Lead/worker, Save. Saving rewrites `BIOROUTER_MODEL` to the worker and
 * announces it, so BOTH windows' chips moved — but only the window that saved
 * said anything about a pair. The other kept the "no pair configured" it had read
 * at mount and drew a bare name, with nothing to suggest a lead existed. The
 * reads behind the label are asserted as ONE refresh, because the failure this
 * guards is a partial one.
 *
 * **D7, 2026-09-12.** What the refreshed label SAID was wrong. Measured against
 * `origin/main` with lead `claude_code / claude-opus-5`, worker `versa_azure /
 * gpt-4.1-mini-2025-04-14` and `BIOROUTER_LEAD_TURNS: 3`, Home's chip read
 * `Current model: gpt-4.1-mini-2025-04-14 (worker) (Public model)` — while
 * `LeadWorkerProvider::get_active_provider` sends the first three turns of every
 * new chat to the lead. `BIOROUTER_MODEL` **is** the worker while a pair is on, so
 * the old role (that key compared against `BIOROUTER_LEAD_MODEL`) could only ever
 * read `worker`.
 *
 * So the assertions here changed with the truth, and #279's mechanism did not:
 * the label still moves on another window's announcement, and still moves because
 * all four keys are re-read together. `leadWorkerLabel.ts` holds the rule and its
 * arithmetic is tested beside it.
 */
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

// The app-wide selection this window believes in. After a lead/worker save it
// is the WORKER — that is what the daemon writes to `BIOROUTER_MODEL`.
const selection = { model: 'gpt-4.1-mini-2025-04-14', provider: 'versa_azure' };
/** The other half of the measured pair. */
const LEAD = 'claude-opus-5';

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: selection.model,
    currentProvider: selection.provider,
    getCurrentModelAndProviderForDisplay: async () => ({
      model: selection.model,
      provider: 'Versa API Azure',
    }),
    getCurrentModelDisplayName: async () => selection.model,
    getCurrentProviderDisplayName: async () => 'Versa API Azure',
  }),
}));

vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

const providerEntry = (name: string, display: string, tier: 'private' | 'public') => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: display, tier, runs_locally: false },
  affiliation: null,
  resolved_tier: tier,
});

/** What the daemon answers for the keys the label is derived from. */
function daemonHolds(
  leadModel: string,
  activeModel: string,
  extra: { leadProvider?: string; leadTurns?: unknown } = {}
) {
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_LEAD_MODEL') return leadModel;
    if (key === 'BIOROUTER_LEAD_PROVIDER') return extra.leadProvider ?? 'claude_code';
    if (key === 'BIOROUTER_MODEL') return activeModel;
    if (key === 'BIOROUTER_LEAD_TURNS') return extra.leadTurns ?? 3;
    return '';
  });
}

/**
 * The handover line lives in the chip's dropdown, and Radix's trigger opens on
 * `pointerdown`, not on `click` — the gesture
 * `ModelsBottomBar.browserSurface.test.tsx` uses on this chip.
 */
async function openChipMenu() {
  await screen.findByRole('button', { name: /Current model:/ });
  fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
  await screen.findByRole('menuitem', { name: /Change model/ });
}

function renderBar(sessionId: string | null = null) {
  return render(
    <ModelsBottomBar
      sessionId={sessionId}
      privacyTier={undefined}
      dropdownRef={dropdownRef}
      setView={vi.fn()}
      alerts={[]}
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([
    providerEntry('versa_azure', 'Versa API Azure', 'private'),
    providerEntry('claude_code', 'Claude Code', 'public'),
  ]);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: {
      title_template: '{provider} is not hosted by your institution.',
      long: 'LONG',
      short: 'SHORT',
      acknowledged: true,
    },
  });
  // No pair configured when this window mounts — the state every window that
  // did not open the modal is in.
  daemonHolds('', selection.model);
});

describe('a lead/worker pair saved in another window', () => {
  it('names the lead, as the lead, once the selection change is announced', async () => {
    renderBar();
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14/));
    expect(trigger).not.toHaveAccessibleName(/lead/);

    // The other window saved the pair: the daemon now holds a lead, and
    // `BIOROUTER_MODEL` names the worker.
    daemonHolds(LEAD, selection.model);
    announceAppModelSelection();

    // Home's next message opens a new chat, and a new chat's first turns are the
    // lead's — so this is the model that gets it.
    await waitFor(() => expect(trigger).toHaveAccessibleName(/claude-opus-5 \(lead\)/));
    expect(trigger).not.toHaveAccessibleName(/worker/);
  });

  it('drops the statement again when the pair is turned off elsewhere', async () => {
    daemonHolds(LEAD, selection.model);
    renderBar();
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/\(lead\)/));

    daemonHolds('', selection.model);
    announceAppModelSelection();

    await waitFor(() => expect(trigger).not.toHaveAccessibleName(/lead/));
    expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14/);
  });

  /**
   * D7's other half. The role was derived by comparing `BIOROUTER_MODEL` against
   * `BIOROUTER_LEAD_MODEL`, and `BIOROUTER_MODEL` IS the worker, so inside a chat
   * the chip asserted `(worker)` over the whole of every chat — including the
   * opening turns the lead serves. The turn count that would settle it is daemon
   * state this component is never served, so the claim is dropped rather than
   * flipped.
   */
  it('claims no role inside a chat, where the live half is not knowable', async () => {
    daemonHolds(LEAD, selection.model);
    renderBar('20260610_28');
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14/));
    expect(trigger).not.toHaveAccessibleName(/\(worker\)/);
    expect(trigger).not.toHaveAccessibleName(/\(lead\)/);
  });

  /**
   * What the chip cannot say, the dropdown can — and it is the same sentence on
   * both surfaces, because the handover is true of both.
   */
  it('states the handover in the dropdown, on Home and in a chat alike', async () => {
    daemonHolds(LEAD, selection.model, { leadTurns: 2 });
    const { unmount } = renderBar();
    await openChipMenu();
    await waitFor(() =>
      expect(screen.getByTestId('lead-worker-handover-note')).toHaveTextContent(
        'Lead/worker mode. claude-opus-5 answers the first 2 turns of a chat; gpt-4.1-mini-2025-04-14 takes the rest.'
      )
    );
    unmount();

    renderBar('20260610_28');
    await openChipMenu();
    await waitFor(() =>
      expect(screen.getByTestId('lead-worker-handover-note')).toHaveTextContent(
        /claude-opus-5 answers the first 2 turns/
      )
    );
  });

  it('says nothing about a pair when none is configured', async () => {
    renderBar();
    await openChipMenu();
    expect(screen.queryByTestId('lead-worker-handover-note')).toBeNull();
  });

  /**
   * The badges beside the name are read off ONE catalog row, for whichever
   * provider the chip is naming. Naming the lead while still resolving the
   * worker's provider put the worker's Private padlock on a public lead — the
   * cross-provider pairing `ModelsBottomBar`'s own comments warn about, and the
   * demotion case the tier exists to catch.
   */
  it("takes the lead PROVIDER's tier, not the worker's", async () => {
    daemonHolds(LEAD, selection.model);
    renderBar();
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/claude-opus-5 \(lead\)/));
    // `claude_code` is Public; `versa_azure` — the worker's provider, and what
    // the app-wide selection names — is Private.
    await waitFor(() => expect(trigger).toHaveAccessibleName(/Public model/));
    expect(trigger).not.toHaveAccessibleName(/Private model/);
  });

  /**
   * A chat with its own binding runs the single provider its row names, so no
   * half of a globally configured pair is in play: no role, no handover line.
   */
  it('says nothing about a pair for a chat with its own binding', async () => {
    daemonHolds(LEAD, selection.model);
    render(
      <ModelsBottomBar
        sessionId="20260610_28"
        privacyTier={undefined}
        effectiveModel={{ provider: 'versa_azure', model: 'gpt-5.2-2025-12-11' }}
        dropdownRef={dropdownRef}
        setView={vi.fn()}
        alerts={[]}
      />
    );
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/gpt-5\.2-2025-12-11/));
    expect(trigger).not.toHaveAccessibleName(/lead|worker/);
    await openChipMenu();
    expect(screen.queryByTestId('lead-worker-handover-note')).toBeNull();
  });
});
