import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails, ProviderTier } from '../../../api';
import ProviderCatalog, { defaultCatalogTab, tabFromHint } from './ProviderCatalog';
import { getOrderedProviderGroups } from './providerOrdering';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';
import type { CodingAgentAuth, CodingAgentAvailability } from '../../onboarding/codingAgentStatus';

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
// The real dock wires xterm and the terminal:create IPC, neither of which exists
// in jsdom. Every other suite in the repo mocks it the same way.
vi.mock('../../InAppTerminalDock', () => ({
  default: () => <div data-testid="in-app-terminal-dock" />,
}));

type Backend = {
  tier?: ProviderTier;
  runs_locally?: boolean;
  institutions?: { id: string; display_name?: string | null }[];
  affiliation?: ProviderDetails['affiliation'];
  resolved_tier?: ProviderTier | null;
  is_configured?: boolean;
};

function provider(name: string, backend: Backend = {}, display = name): ProviderDetails {
  return {
    name,
    is_configured: backend.is_configured ?? true,
    provider_type: 'Builtin',
    affiliation: backend.affiliation,
    resolved_tier: backend.resolved_tier ?? null,
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
  } as ProviderDetails;
}

const UCSF = [{ id: 'ucsf', display_name: 'UCSF' }];
const PRIVATE_REMOTE = { tier: 'private' as ProviderTier, runs_locally: false };
const PRIVATE_LOCAL = { tier: 'private' as ProviderTier, runs_locally: true };

const agent = (
  kind: 'claude_code' | 'codex',
  auth: CodingAgentAuth,
  over: Partial<CodingAgentAvailability> = {}
): CodingAgentAvailability => ({
  kind,
  providerId: kind,
  displayName: kind === 'codex' ? 'Codex' : 'Claude Code',
  path: null,
  version: null,
  auth,
  loginCommand: `${kind} auth login`,
  installHint: `install ${kind}`,
  ...over,
});

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

const openTab = () =>
  ['local', 'institutional', 'commercial'].find(
    (key) => screen.getByTestId(`catalog-tab-${key}`).getAttribute('data-state') === 'active'
  );

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [] });
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'L', short: 'S', acknowledged: true },
  });
});

/**
 * The default-tab rule, as a pure function.
 *
 * ⚠ Tested here rather than only through the rendered catalog, because the
 * interesting clauses are the ones that need a *combination* of state — a
 * configured provider that is also an agent, an empty computed tab — and driving
 * each of those through a mount would take five renders to say what five
 * assertions say.
 */
describe('defaultCatalogTab', () => {
  const groups = (rows: ProviderDetails[]) => getOrderedProviderGroups(rows);
  const rows = [
    provider('llamacpp', PRIVATE_LOCAL),
    provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }),
    provider('anthropic'),
    provider('claude_code', {}, 'Claude Code'),
  ];

  it('opens the tab holding the provider that is already bound', () => {
    expect(
      defaultCatalogTab({
        groups: groups(rows),
        configuredProvider: 'versa_azure',
        hasSubscriptionReadyAgent: false,
      })
    ).toBe('institutional');
  });

  it('opens Public when a coding agent is signed in on a subscription and nothing is bound', () => {
    expect(
      defaultCatalogTab({
        groups: groups(rows),
        configuredProvider: null,
        hasSubscriptionReadyAgent: true,
      })
    ).toBe('commercial');
  });

  it('opens Local otherwise', () => {
    expect(
      defaultCatalogTab({
        groups: groups(rows),
        configuredProvider: null,
        hasSubscriptionReadyAgent: false,
      })
    ).toBe('local');
  });

  /**
   * ⚠ **The bound provider outranks the ready agent**, and the fixture makes the
   * two disagree on purpose. A rule that checked the agent first would drag a
   * user who works on Versa every day onto Public because a `claude` binary
   * happens to be signed in on the machine.
   */
  it('prefers the bound provider over a ready agent', () => {
    expect(
      defaultCatalogTab({
        groups: groups(rows),
        configuredProvider: 'llamacpp',
        hasSubscriptionReadyAgent: true,
      })
    ).toBe('local');
  });

  it('lets a route hint override every computed default, in either spelling', () => {
    for (const hint of ['public', 'commercial', 'PUBLIC ']) {
      expect(
        defaultCatalogTab({
          groups: groups(rows),
          configuredProvider: 'versa_azure',
          hasSubscriptionReadyAgent: false,
          routeHint: hint,
        })
      ).toBe('commercial');
    }
    expect(tabFromHint('nonsense')).toBeNull();
    expect(tabFromHint(undefined)).toBeNull();
  });

  /**
   * ⚠ Reachable, not hypothetical: a build with no local providers at all sends
   * every unconfigured user to an empty Local tab under the rule above.
   */
  it('falls to the first non-empty tab rather than opening an empty one', () => {
    expect(
      defaultCatalogTab({
        groups: groups([provider('anthropic')]),
        configuredProvider: null,
        hasSubscriptionReadyAgent: false,
      })
    ).toBe('commercial');
  });

  /** …but an explicit hint still wins, even when it names an empty tab. */
  it('honours a hint at an empty tab, because a CTA said to go there', () => {
    expect(
      defaultCatalogTab({
        groups: groups([provider('anthropic')]),
        configuredProvider: null,
        hasSubscriptionReadyAgent: false,
        routeHint: 'institutional',
      })
    ).toBe('institutional');
  });
});

describe('ProviderCatalog — institutions', () => {
  /**
   * ⚠ **The heading is the daemon's word, not ours.** The fixture's display name
   * is deliberately not "UCSF": a catalog that printed a literal would pass with
   * the real payload and fail here, which is the only way to tell the two apart.
   */
  it('heads each institution group with the display name the daemon sent', () => {
    render(
      <ProviderCatalog
        providers={[
          provider('versa_azure', {
            ...PRIVATE_REMOTE,
            institutions: [{ id: 'ucsf', display_name: 'Fictional University' }],
          }),
        ]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('institutional');
    expect(screen.getByText('Fictional University')).toBeInTheDocument();
  });

  it('renders the real UCSF payload as UCSF, holding both Versa rows', () => {
    render(
      <ProviderCatalog
        providers={[
          provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }, 'Versa API Azure'),
          provider('versa_bedrock', { ...PRIVATE_REMOTE, institutions: UCSF }, 'Versa API Bedrock'),
        ]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('institutional');
    const group = screen.getByTestId('catalog-section-ucsf');
    expect(group).toHaveTextContent('UCSF');
    expect(group).toHaveTextContent('Versa API Azure');
    expect(group).toHaveTextContent('Versa API Bedrock');
    expect(screen.queryByTestId('catalog-section-unaffiliated')).toBeNull();
  });

  /**
   * The instance-resolved affiliation outranks the shipped metadata. ⚠ A
   * fallback written as a *preference* would keep the institution's name on a
   * gateway the daemon has already repointed and demoted.
   */
  it('drops the group when a resolved instance names no institution', () => {
    render(
      <ProviderCatalog
        providers={[
          provider('versa_azure', {
            ...PRIVATE_REMOTE,
            institutions: UCSF,
            resolved_tier: 'private',
            affiliation: { kind: 'unstated', institutions: [] },
          }),
        ]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('institutional');
    expect(screen.queryByTestId('catalog-section-ucsf')).toBeNull();
    expect(screen.getByTestId('catalog-section-unaffiliated')).toBeInTheDocument();
  });

  it('always explains where other institutions come from', () => {
    render(<ProviderCatalog providers={[]} mode="settings" configuredProvider={null} />);
    clickTab('institutional');
    const note = screen.getByTestId('other-institutions-note');
    expect(note).toHaveTextContent(/recognises its endpoint as private/i);
    expect(note).toHaveTextContent(/Add Custom Provider/i);
  });
});

describe('ProviderCatalog — AI agents', () => {
  const withAgents = (agents: CodingAgentAvailability[]) => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({ agents });
    return render(
      <ProviderCatalog
        providers={[
          provider('claude_code', {}, 'Claude Code'),
          provider('codex', {}, 'Codex'),
          provider('anthropic', {}, 'Anthropic'),
        ]}
        mode="settings"
        configuredProvider={null}
      />
    );
  };

  it.each([
    ['not_installed', 'Not installed'],
    ['signed_out', 'Installed · not signed in'],
    ['signed_in_with_api_key', 'Signed in with an API key'],
    ['signed_in_subscription', 'Ready · signed in on your subscription'],
  ] as const)('renders the %s pill on the agent row', async (state, label) => {
    withAgents([agent('claude_code', { state } as CodingAgentAuth)]);
    clickTab('commercial');
    expect(await screen.findByText(label)).toBeInTheDocument();
  });

  it('renders the indeterminate pill and its detail', async () => {
    withAgents([agent('claude_code', { state: 'indeterminate', detail: 'probe exploded' })]);
    clickTab('commercial');
    expect(await screen.findByText('Status unclear')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('provider-row-toggle-claude_code'));
    expect(screen.getByTestId('coding-agent-detail-claude_code')).toHaveTextContent(
      'probe exploded'
    );
  });

  /**
   * ⚠ **Fetched ONCE.** `GET /coding_agents/status` spawns both vendor CLIs, so
   * the probe is mounted at the catalog rather than at each row — two rows and a
   * tab change must not become three probes, and a timer must never appear.
   */
  it('probes the vendor CLIs once, not per row and not on a timer', async () => {
    vi.useFakeTimers();
    try {
      withAgents([
        agent('claude_code', { state: 'signed_in_subscription' }),
        agent('codex', { state: 'signed_out' }),
      ]);
      await vi.advanceTimersByTimeAsync(60_000);
      clickTab('local');
      clickTab('commercial');
      await vi.advanceTimersByTimeAsync(60_000);
      expect(mocks.fetchCodingAgentStatus).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('re-probes exactly once when the user asks it to', async () => {
    withAgents([agent('claude_code', { state: 'signed_out' })]);
    clickTab('commercial');
    fireEvent.click(await screen.findByTestId('provider-row-toggle-claude_code'));
    fireEvent.click(screen.getByTestId('coding-agent-recheck-claude_code'));
    await waitFor(() => expect(mocks.fetchCodingAgentStatus).toHaveBeenCalledTimes(2));
  });

  it('puts the agents ahead of the pinned API providers, in their fixed order', async () => {
    withAgents([
      // Served the "wrong" way round on purpose: the order is the catalog's,
      // not the daemon's response order.
      agent('codex', { state: 'signed_out' }),
      agent('claude_code', { state: 'signed_out' }),
    ]);
    clickTab('commercial');
    await screen.findByTestId('catalog-section-agents');
    const rendered = screen
      .getAllByTestId(/^provider-card-/)
      .map((node) => node.getAttribute('data-testid'));
    expect(rendered).toEqual([
      'provider-card-claude_code',
      'provider-card-codex',
      'provider-card-anthropic',
    ]);
  });
});

describe('ProviderCatalog — modes', () => {
  it('opens a local row into its setup panel in onboarding mode', async () => {
    render(
      <ProviderCatalog
        providers={[
          provider('llamacpp', PRIVATE_LOCAL, 'Llama Server'),
          provider('ollama', PRIVATE_LOCAL, 'Ollama'),
        ]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    // The recommended local provider opens by default; the other does not.
    expect(await screen.findByTestId('provider-row-panel-llamacpp')).toBeInTheDocument();
    expect(screen.queryByTestId('provider-row-panel-ollama')).toBeNull();

    // One at a time: opening the second closes the first.
    fireEvent.click(screen.getByTestId('provider-row-toggle-ollama'));
    expect(screen.getByTestId('provider-row-panel-ollama')).toBeInTheDocument();
    expect(screen.queryByTestId('provider-row-panel-llamacpp')).toBeNull();
  });

  it('leaves local rows as click-to-configure in settings mode', () => {
    render(
      <ProviderCatalog
        providers={[provider('llamacpp', PRIVATE_LOCAL, 'Llama Server')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    expect(screen.queryByTestId('provider-row-toggle-llamacpp')).toBeNull();
    expect(screen.getByTestId('provider-card-llamacpp')).toBeInTheDocument();
  });

  /**
   * ⚠ The institutional setup form is offered only where it configures one of
   * that section's own providers. A form written for UCSF's Versa endpoints
   * under another institution's heading would ask for the wrong credentials and
   * bind the wrong gateway.
   */
  it('offers the institutional setup form only under an institution it configures', () => {
    const { unmount } = render(
      <ProviderCatalog
        providers={[provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF })]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    clickTab('institutional');
    expect(screen.getByLabelText('UCSF-hosted provider')).toBeInTheDocument();
    unmount();

    render(
      <ProviderCatalog
        providers={[
          provider('someone_elses_gateway', {
            ...PRIVATE_REMOTE,
            institutions: [{ id: 'elsewhere', display_name: 'Elsewhere University' }],
          }),
        ]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    clickTab('institutional');
    expect(screen.getByText('Elsewhere University')).toBeInTheDocument();
    expect(screen.queryByLabelText('UCSF-hosted provider')).toBeNull();
  });

  it('puts the paste-a-key detector at the top of the API section in onboarding only', () => {
    const { unmount } = render(
      <ProviderCatalog
        providers={[provider('anthropic')]}
        mode="onboarding"
        configuredProvider={null}
      />
    );
    clickTab('commercial');
    expect(screen.getByLabelText('Commercial provider API key')).toBeInTheDocument();
    unmount();

    render(
      <ProviderCatalog
        providers={[provider('anthropic')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('commercial');
    expect(screen.queryByLabelText('Commercial provider API key')).toBeNull();
  });

  it('opens on the bound provider tab without being told which one that is', async () => {
    render(
      <ProviderCatalog
        providers={[
          provider('llamacpp', PRIVATE_LOCAL),
          provider('versa_azure', { ...PRIVATE_REMOTE, institutions: UCSF }),
        ]}
        mode="settings"
        configuredProvider="versa_azure"
      />
    );
    await waitFor(() => expect(openTab()).toBe('institutional'));
  });
});
