import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { announceAppModelSelection } from '../../../../utils/sessionBindingSync';

/**
 * The chip's lead/worker ROLE has to follow the app-wide selection, not just
 * its value.
 *
 * Measured live on 2026-09-12, two windows of one instance: Settings → Models →
 * Lead/worker, lead `gpt-5.5-2026-04-24`, worker `gpt-4.1-mini-2025-04-14`,
 * Save. Saving rewrites `BIOROUTER_MODEL` to the WORKER and announces it, so
 * BOTH windows' chips moved to `gpt-4.1-mini-2025-04-14` — but only the window
 * that saved said `(worker)`. The other kept the `isLeadWorkerActive: false` it
 * had read at mount and drew a bare name, with nothing to say a lead was
 * configured at all.
 *
 * The three reads behind the label are asserted as ONE refresh here, because
 * the failure this guards is a partial one: `isLeadWorkerActive` fresh and
 * `BIOROUTER_MODEL` stale puts `(lead)` on the worker.
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

vi.mock('../../../BaseChat', () => ({ useCurrentModelInfo: () => null }));
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

/** What the daemon answers for the two keys the role is derived from. */
function daemonHolds(leadModel: string, activeModel: string) {
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_LEAD_MODEL') return leadModel;
    if (key === 'BIOROUTER_MODEL') return activeModel;
    return '';
  });
}

function renderBar() {
  return render(
    <ModelsBottomBar
      sessionId={null}
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
  it('labels the chip (worker) once the selection change is announced', async () => {
    renderBar();
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14/));
    expect(trigger).not.toHaveAccessibleName(/worker/);

    // The other window saved the pair: the daemon now holds a lead, and
    // `BIOROUTER_MODEL` names the worker.
    daemonHolds('gpt-5.5-2026-04-24', selection.model);
    announceAppModelSelection();

    await waitFor(() =>
      expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14 \(worker\)/)
    );
  });

  it('labels it (lead) when the selection names the lead half', async () => {
    renderBar();
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14/));

    // A pair whose lead IS the selected model: the role must read `lead`, which
    // is only derivable from BOTH keys being re-read together.
    daemonHolds(selection.model, selection.model);
    announceAppModelSelection();

    await waitFor(() => expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14 \(lead\)/));
    expect(trigger).not.toHaveAccessibleName(/worker/);
  });

  it('drops the role again when the pair is turned off elsewhere', async () => {
    daemonHolds('gpt-5.5-2026-04-24', selection.model);
    renderBar();
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/\(worker\)/));

    daemonHolds('', selection.model);
    announceAppModelSelection();

    await waitFor(() => expect(trigger).not.toHaveAccessibleName(/worker/));
    expect(trigger).toHaveAccessibleName(/gpt-4\.1-mini-2025-04-14/);
  });
});
