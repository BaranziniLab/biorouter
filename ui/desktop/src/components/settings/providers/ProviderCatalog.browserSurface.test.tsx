import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../api';
import ProviderCatalog from './ProviderCatalog';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';

/**
 * SD-1 on Settings > Providers.
 *
 * ⚠ This page is a *partial* block, and the test asserts the partiality rather
 * than only the note. Saving a provider's API key is not a capability write and
 * still works from a browser; the step after it — choosing which model becomes
 * the default, in `SwitchModelModal` — is the one that 409s. Saying so at the
 * top of the page means the user learns it before pasting a secret rather than
 * after.
 */

const mocks = vi.hoisted(() => ({
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  fetchCodingAgentStatus: vi.fn(),
  upsert: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../../onboarding/codingAgentStatus', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  fetchCodingAgentStatus: mocks.fetchCodingAgentStatus,
}));
// `useCodingAgents` writes the provider selection through `useConfig`, so the
// catalog now needs the config context that `ProviderGrid` never touched.
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    upsert: mocks.upsert,
    read: vi.fn(async () => null),
    getProviders: vi.fn(async () => []),
  }),
  usePrivacyTiersEnabled: () => true,
}));

function provider(name: string): ProviderDetails {
  return {
    name,
    is_configured: true,
    provider_type: 'Builtin',
    metadata: {
      config_keys: [],
      default_model: '',
      description: '',
      display_name: name,
      known_models: [],
      model_doc_link: '',
      name,
      tier: 'public',
      runs_locally: false,
    },
  } as ProviderDetails;
}

describe('ProviderCatalog on a browser-served surface', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    __resetDisclosureStoreForTests();
    mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [] });
    mocks.getPrivacyDisclosure.mockResolvedValue({
      data: {
        title_template: '{provider} is not hosted by your institution.',
        long: 'SERVED-LONG-MARKER',
        short: 'SERVED-SHORT-MARKER',
        acknowledged: true,
      },
    });
  });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  /**
   * ⚠ The note is on the PAGE, above the tabs, not inside a panel — a per-tab
   * note would be missed by exactly the user who opens the catalog on their own
   * institution's tab and never visits Public.
   *
   * The fixture is one public provider, which also exercises the default-tab
   * rule's last clause: Local is the computed default and is empty here, so the
   * catalog opens on the first tab that has anything in it rather than on a
   * blank panel.
   */
  it('says once, at the top, that the host owns the choice', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(
      <ProviderCatalog
        providers={[provider('anthropic')]}
        mode="settings"
        configuredProvider={null}
      />
    );

    const note = await screen.findByTestId('host-managed-model-note');
    expect(note.textContent).toMatch(/biorouter configure/);
    // Still a provider page: the rows are not taken away, because storing a
    // key is not what gets refused.
    expect(screen.getByText('anthropic')).toBeInTheDocument();
    expect(screen.getByTestId('add-custom-provider-card')).toBeInTheDocument();
  });

  /** The control: passes before and after. */
  it('adds nothing in the desktop application', async () => {
    render(
      <ProviderCatalog
        providers={[provider('anthropic')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    // Settle the disclosure fetch first, so this asserts the resolved page
    // rather than winning a race against a note that has not arrived yet.
    await screen.findByTestId('non-private-model-note');
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();
  });
});
