import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails, ProviderTier } from '../../../api';
import ProviderCatalog from './ProviderCatalog';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';

// Task 30A: the API-provider section carries the served one-line disclosure.
// ⚠ The fixture is deliberately not the product's sentence — Step 5's gate (1)
// counts definitions of that sentence across `ui/desktop/src/` and expects one,
// and a `--include='*.tsx'` grep does not skip test files.
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
// ⚠ `importActual` spread, not a bare factory: `providerOrdering` imports
// `CODING_AGENT_ORDER` from this module to decide which public providers are AI
// agents, so a mock that dropped it would silently un-section the Public tab.
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

/**
 * §14.5 — the two taxonomies must be the same words in the same place, now that
 * the three groups are three tabs.
 *
 * ⚠ These assertions live here, not in `providerOrdering.test.ts`, and that is
 * the whole point of this file. A predecessor of the catalog imported
 * `getOrderedProviderGroups` for its *ordering* and then ignored `label`
 * entirely, printing three hardcoded literals of its own. Relabelling the data
 * alone changes nothing a user can see, and a unit test of the data alone would
 * be green while the screen still said "Institutional Models".
 *
 * ⚠ **Tabs make an absence cheap and therefore dangerous.** A panel the test
 * never opens renders nothing, so `queryByText(...) === null` is satisfied by a
 * label that is present and merely on another tab. Every assertion below either
 * clicks the tab first, or is paired with one that does.
 */
function provider(
  name: string,
  backend: { tier?: ProviderTier; runs_locally?: boolean } = {}
): ProviderDetails {
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
      tier: backend.tier ?? 'public',
      runs_locally: backend.runs_locally ?? false,
    },
  } as ProviderDetails;
}

const all = [
  provider('llamacpp', { tier: 'private', runs_locally: true }),
  provider('versa_azure', { tier: 'private' }),
  provider('azure_openai'),
  provider('anthropic'),
];

function renderCatalog() {
  return render(<ProviderCatalog providers={all} mode="settings" configuredProvider={null} />);
}

/**
 * ⚠ **Radix's `TabsTrigger` activates on `mousedown`, not on a synthetic
 * `click`.** A `fireEvent.click` alone leaves the panel untouched — and because
 * an unopened panel renders nothing, every assertion after it quietly tests
 * whichever tab happened to be open, passing or failing for the wrong reason.
 */
const clickTab = (key: 'local' | 'institutional' | 'commercial') => {
  const trigger = screen.getByTestId(`catalog-tab-${key}`);
  fireEvent.mouseDown(trigger);
  fireEvent.click(trigger);
};

describe('ProviderCatalog — the privacy taxonomy, on screen', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The served copy is held in module state (one install, one disclosure), so
    // it outlives `cleanup()` and has to be dropped between tests.
    __resetDisclosureStoreForTests();
    mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [] });
    mocks.getPrivacyDisclosure.mockResolvedValue({
      data: {
        title_template: '{provider} is not hosted by your institution.',
        long: 'SERVED-LONG-MARKER',
        short: 'SERVED-SHORT-MARKER — this model can read files on this computer.',
        acknowledged: true,
      },
    });
  });

  it('each tab names the two taxonomies with the same words', () => {
    renderCatalog();

    clickTab('local');
    expect(screen.getByText(/Private · Local/)).toBeInTheDocument();

    clickTab('institutional');
    expect(screen.getByText(/Private · Institutional/)).toBeInTheDocument();

    clickTab('commercial');
    expect(screen.getByText(/Public · Commercial/)).toBeInTheDocument();
  });

  it('the old headings are gone from every panel, not merely from the open one', () => {
    renderCatalog();

    // The half a data-only relabel cannot satisfy — and asserted per tab,
    // because an unopened panel is empty for reasons that have nothing to do
    // with this change.
    for (const tab of ['local', 'institutional', 'commercial'] as const) {
      clickTab(tab);
      expect(screen.queryByText('Local Models')).toBeNull();
      expect(screen.queryByText('Institutional Models')).toBeNull();
      expect(screen.queryByText('Commercial Models')).toBeNull();
    }
  });

  it('says why an institutional endpoint is private, and why a cloud account is not', () => {
    renderCatalog();

    // §14.5, verbatim: the reason is the recognised endpoint, not the vendor.
    clickTab('institutional');
    expect(screen.getByText(/recognises this institutional gateway endpoint/i)).toBeInTheDocument();

    // §14.5's note: NOT "a direct cloud account, even if your institution pays
    // for it" — `azure.rs` defaults AZURE_OPENAI_ENDPOINT to the UCSF gateway
    // itself, so that wording would claim something the configuration
    // contradicts.
    clickTab('commercial');
    expect(screen.getByText(/can't verify where/i)).toHaveTextContent(/endpoint points/i);
  });

  /**
   * Task 30A (issue #56, DR-17 requirement 3). The API-provider section is one of
   * the surfaces that carries the disclosure permanently — it reads no
   * acknowledgement and never goes quiet, which is what makes "shown once,
   * forcefully" a defensible design rather than a one-off popup.
   */
  it('the API-provider section says what a model there can reach, in the served words', async () => {
    renderCatalog();
    clickTab('commercial');
    const note = await screen.findByTestId('non-private-model-note');
    expect(note).toHaveTextContent(/SERVED-SHORT-MARKER/);
    expect(note).toHaveTextContent(/can read files on this computer/i);
  });

  /**
   * ⚠ The disclosure belongs to the PUBLIC tab and nowhere else. Rendering it on
   * a private panel would attach "this model can read files on this computer" to
   * a local model, which is both false and the opposite of the warning's point.
   */
  it('carries the disclosure only under Public', async () => {
    renderCatalog();
    clickTab('commercial');
    await screen.findByTestId('non-private-model-note');

    clickTab('local');
    expect(screen.queryByTestId('non-private-model-note')).toBeNull();
    clickTab('institutional');
    expect(screen.queryByTestId('non-private-model-note')).toBeNull();
  });

  it('renders nothing there rather than inventing prose when the copy cannot be fetched', async () => {
    mocks.getPrivacyDisclosure.mockRejectedValue(new Error('offline'));
    renderCatalog();
    clickTab('commercial');
    expect(await screen.findByText(/Public · Commercial/)).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByTestId('non-private-model-note')).toBeNull());
  });
});
