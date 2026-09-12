import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
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
  checkProvider: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
  // The configure form's submit handler validates the saved keys through it.
  checkProvider: mocks.checkProvider,
}));
// The configure modal asks which provider is bound before offering "Remove".
vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: async () => ({ provider: 'versa_azure', model: 'm' }),
  }),
}));
// A successful save opens the model picker; what it shows is its own suite's
// business (`SwitchModelModal.*.test.tsx`), not this one's.
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
// The Local tab mounts `OllamaInlineCard`, whose mount effect calls
// `checkOllamaStatus()` -> `fetch('http://127.0.0.1:11434/api/tags')`. Left
// un-mocked that is a REAL request to whatever the developer is running: with
// `ollama serve` up it resolves `isRunning: true`, takes the branch at
// OllamaInlineCard.tsx:39, fires a second request for `hasModel()`, and lands two
// more state updates after the test body. CI, where nothing listens on 11434,
// takes neither. Reporting "not running" is what CI sees, so that is what every
// machine should see. App.test.tsx, App.routing.test.tsx and
// LocalModelInventory.test.tsx already mock this module; this suite did not.
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

type Backend = {
  tier?: ProviderTier;
  runs_locally?: boolean;
  institutions?: { id: string; display_name?: string | null }[];
  affiliation?: ProviderDetails['affiliation'];
  resolved_tier?: ProviderTier | null;
  is_configured?: boolean;
  unavailable_reason?: string | null;
};

function provider(name: string, backend: Backend = {}, display = name): ProviderDetails {
  return {
    name,
    is_configured: backend.is_configured ?? true,
    provider_type: 'Builtin',
    affiliation: backend.affiliation,
    resolved_tier: backend.resolved_tier ?? null,
    unavailable_reason: backend.unavailable_reason ?? null,
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
    expect(note).toHaveTextContent(/recognizes its endpoint as private/i);
    // Case-sensitive on purpose: this used to be `/Add Custom Provider/i`,
    // which kept passing when the control was renamed to sentence case — so
    // it was asserting the words, not that the note names the real label.
    expect(note).toHaveTextContent('Add custom provider');
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

  /**
   * F6 of the 2026-09-10 provider QA run, as the renderer can reproduce it: the
   * row read "Codex · Not installed" and "✓ Configured" on one line.
   *
   * The check is `is_configured`, and the daemon no longer grants it to a coding
   * agent whose CLI does not resolve — but the catalog reads the provider list
   * once, when the page opens. Here the list was read while Codex was installed;
   * the CLI is then removed and "Check again" says so. Unless the re-check also
   * re-reads the list, the stale check sits beside the fresh pill.
   */
  it('drops the Configured check when a re-check finds the CLI gone', async () => {
    const NOT_INSTALLED = 'Codex is not installed, or is not on a path Biorouter searches';
    mocks.fetchCodingAgentStatus
      .mockResolvedValueOnce({ agents: [agent('codex', { state: 'signed_in_subscription' })] })
      .mockResolvedValueOnce({ agents: [agent('codex', { state: 'not_installed' })] });

    function CatalogWithLiveList() {
      const [rows, setRows] = useState([provider('codex', {}, 'Codex')]);
      return (
        <ProviderCatalog
          providers={rows}
          mode="settings"
          configuredProvider={null}
          // What the daemon serves once the CLI is gone.
          refreshProviders={() =>
            setRows([
              provider(
                'codex',
                { is_configured: false, unavailable_reason: NOT_INSTALLED },
                'Codex'
              ),
            ])
          }
        />
      );
    }

    render(<CatalogWithLiveList />);
    clickTab('commercial');
    const row = await screen.findByTestId('provider-card-codex');
    await within(row).findByText('Ready · signed in on your subscription');
    expect(within(row).getByText('Configured')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('provider-row-toggle-codex'));
    fireEvent.click(screen.getByTestId('coding-agent-recheck-codex'));

    await within(row).findByText('Not installed');
    await waitFor(() => expect(within(row).queryByText('Configured')).toBeNull());
  });

  // The other half: the mount probe must not re-read a list the page fetched
  // at the same moment, or opening the catalog costs two provider sweeps.
  it('re-reads the provider list only on an explicit re-check', async () => {
    const refreshProviders = vi.fn();
    mocks.fetchCodingAgentStatus.mockResolvedValue({
      agents: [agent('codex', { state: 'not_installed' })],
    });
    render(
      <ProviderCatalog
        providers={[provider('codex', { is_configured: false }, 'Codex')]}
        mode="settings"
        configuredProvider={null}
        refreshProviders={refreshProviders}
      />
    );
    clickTab('commercial');
    await screen.findByText('Not installed');
    expect(refreshProviders).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('provider-row-toggle-codex'));
    fireEvent.click(screen.getByTestId('coding-agent-recheck-codex'));
    await waitFor(() => expect(refreshProviders).toHaveBeenCalledTimes(1));
  });

  /**
   * The same contradiction from the other side, found by driving the running
   * app: `CODEX_COMMAND` corrected in the configure form. The save re-read the
   * provider list, so the check came back — beside a pill still saying "Not
   * installed" from the probe taken when the path was wrong. A change to an
   * agent's setup has to re-probe as well.
   */
  it('re-probes when an agent’s command key is corrected in the configure form', async () => {
    const NOT_INSTALLED = 'Codex is not installed, or is not on a path Biorouter searches';
    const codexKeys = {
      config_keys: [{ name: 'CODEX_COMMAND', required: true, secret: false, default: 'codex' }],
    };
    const codexRow = (backend: Backend) => {
      const row = provider('codex', backend, 'Codex');
      return { ...row, metadata: { ...row.metadata, ...codexKeys } } as ProviderDetails;
    };
    mocks.checkProvider.mockResolvedValue({ data: {} });
    mocks.upsert.mockResolvedValue(undefined);
    mocks.fetchCodingAgentStatus
      .mockResolvedValueOnce({ agents: [agent('codex', { state: 'not_installed' })] })
      .mockResolvedValueOnce({ agents: [agent('codex', { state: 'signed_in_subscription' })] });

    function CatalogWithLiveList() {
      const [rows, setRows] = useState([
        codexRow({ is_configured: false, unavailable_reason: NOT_INSTALLED }),
      ]);
      return (
        <ProviderCatalog
          providers={rows}
          mode="settings"
          configuredProvider={null}
          refreshProviders={() => setRows([codexRow({ is_configured: true })])}
        />
      );
    }

    render(<CatalogWithLiveList />);
    clickTab('commercial');
    const row = await screen.findByTestId('provider-card-codex');
    await within(row).findByText('Not installed');

    fireEvent.click(within(row).getByRole('button', { name: 'Configure' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));

    await within(row).findByText('Ready · signed in on your subscription');
    expect(within(row).getByText('Configured')).toBeInTheDocument();
    expect(mocks.upsert).toHaveBeenCalledWith('CODEX_COMMAND', 'codex', false);
  });

  // "Use Codex" saves the command key, which is what makes the daemon report it
  // configured — so the row behind the picker that opens must re-read the list.
  it('re-reads the provider list after "Use" saves an agent’s command key', async () => {
    const refreshProviders = vi.fn();
    mocks.upsert.mockResolvedValue(undefined);
    mocks.fetchCodingAgentStatus.mockResolvedValue({
      agents: [agent('codex', { state: 'signed_in_subscription' })],
    });
    render(
      <ProviderCatalog
        providers={[provider('codex', { is_configured: false }, 'Codex')]}
        mode="settings"
        configuredProvider={null}
        refreshProviders={refreshProviders}
      />
    );
    clickTab('commercial');
    fireEvent.click(await screen.findByTestId('provider-row-toggle-codex'));
    fireEvent.click(await screen.findByTestId('coding-agent-connect-codex'));

    await screen.findByTestId('switch-model-modal');
    expect(refreshProviders).toHaveBeenCalledTimes(1);
    expect(mocks.upsert).toHaveBeenCalledWith('CODEX_COMMAND', 'codex', false);
  });

  /** The control: a usable agent keeps its check, so the case above is not "never show it". */
  it('keeps the Configured check on an agent that is ready', async () => {
    withAgents([agent('claude_code', { state: 'signed_in_subscription' })]);
    clickTab('commercial');
    const row = await screen.findByTestId('provider-card-claude_code');
    await within(row).findByText('Ready · signed in on your subscription');
    expect(within(row).getByText('Configured')).toBeInTheDocument();
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
