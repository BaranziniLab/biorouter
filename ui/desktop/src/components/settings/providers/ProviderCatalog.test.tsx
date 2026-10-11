import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails, ProviderTier } from '../../../api';
import ProviderCatalog, {
  AGENT_PROBE_WAIT_MS,
  defaultCatalogTab,
  tabFromHint,
} from './ProviderCatalog';
import { getOrderedProviderGroups } from './providerOrdering';
import { catalogCopy, customProviderCopy, providerPageCopy } from './copy';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';
import type { CodingAgentAuth, CodingAgentAvailability } from '../../onboarding/codingAgentStatus';

const mocks = vi.hoisted(() => ({
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  fetchCodingAgentStatus: vi.fn(),
  upsert: vi.fn(),
  checkProvider: vi.fn(),
  read: vi.fn(),
  getCustomProvider: vi.fn(),
  updateCustomProvider: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
  // The configure form's submit handler validates the saved keys through it.
  checkProvider: mocks.checkProvider,
  getCustomProvider: mocks.getCustomProvider,
  updateCustomProvider: mocks.updateCustomProvider,
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
    read: mocks.read,
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
  mocks.read.mockResolvedValue(null);
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

  // The ready pill is not in this table: on a CONFIGURED agent it would be the same news as
  // "Configured" (see 'says one status'), so it is asserted on a row that is not set up yet.
  it.each([
    ['not_installed', 'Not installed'],
    ['signed_out', 'Installed · not signed in'],
    ['signed_in_with_api_key', 'Signed in with an API key'],
  ] as const)('renders the %s pill on the agent row', async (state, label) => {
    withAgents([agent('claude_code', { state } as CodingAgentAuth)]);
    clickTab('commercial');
    expect(await screen.findByText(label)).toBeInTheDocument();
  });

  it('invites setup with the ready pill on an agent that is not configured yet', async () => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({
      agents: [agent('claude_code', { state: 'signed_in_subscription' })],
    });
    render(
      <ProviderCatalog
        providers={[provider('claude_code', { is_configured: false }, 'Claude Code')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('commercial');
    const row = await screen.findByTestId('provider-card-claude_code');
    expect(
      await within(row).findByText('Ready · signed in on your subscription')
    ).toBeInTheDocument();
    expect(within(row).queryByText('Configured')).toBeNull();
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
    // The row opens only once the probe has been read, so the probe is in before the status is.
    fireEvent.click(await screen.findByTestId('provider-row-toggle-codex'));
    // One status: a configured, ready agent says "Configured" and nothing else.
    expect(within(row).getByText('Configured')).toBeInTheDocument();
    expect(within(row).queryByText('Ready · signed in on your subscription')).toBeNull();

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
    const readyStatus = {
      agents: [
        agent('codex', { state: 'signed_in_subscription' }, { path: '/opt/homebrew/bin/codex' }),
      ],
    };
    mocks.fetchCodingAgentStatus
      .mockResolvedValueOnce({ agents: [agent('codex', { state: 'not_installed' })] })
      .mockResolvedValueOnce(readyStatus)
      .mockResolvedValueOnce(readyStatus);

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

    fireEvent.click(within(row).getByRole('button', { name: catalogCopy.configureRow('Codex') }));
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));

    // The corrected command re-probed: the stale "Not installed" is gone, and the ready agent
    // says "Configured" alone.
    await waitFor(() => {
      expect(within(row).getByText('Configured')).toBeInTheDocument();
      expect(within(row).queryByText('Not installed')).toBeNull();
    });
    expect(within(row).queryByText('Ready · signed in on your subscription')).toBeNull();
    expect(mocks.upsert).toHaveBeenCalledWith('CODEX_COMMAND', 'codex', false);
    expect(screen.getByTestId('switch-model-modal')).toBeInTheDocument();
    expect(mocks.fetchCodingAgentStatus).toHaveBeenCalledTimes(3);
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

  /**
   * The control: a usable agent keeps its check, so the case above is not "never show it". And it
   * keeps it ALONE (V4-26): a ready, configured agent used to say "Ready · signed in…" beside
   * "Configured", the same news twice on one row.
   */
  it('says one status on an agent that is ready and configured', async () => {
    withAgents([agent('claude_code', { state: 'signed_in_subscription' })]);
    clickTab('commercial');
    const row = await screen.findByTestId('provider-card-claude_code');
    // The toggle exists only once the probe has been read.
    await screen.findByTestId('provider-row-toggle-claude_code');
    expect(within(row).getByText('Configured')).toBeInTheDocument();
    expect(within(row).queryByText('Ready · signed in on your subscription')).toBeNull();
  });

  it('draws the daemon’s backticked command as inline code, not with the marks', async () => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [] });
    const row = provider('claude_code', {}, 'Claude Code');
    row.metadata.description = 'Uses your own installed `claude` command-line app.';
    render(<ProviderCatalog providers={[row]} mode="settings" configuredProvider={null} />);
    clickTab('commercial');
    const description = await screen.findByTestId('provider-description-claude_code');
    expect(description).toHaveTextContent('Uses your own installed claude command-line app.');
    expect(description.querySelector('code')).toHaveTextContent('claude');
  });

  it('puts the disclosure chevron after the name, so every title starts on one x', async () => {
    withAgents([agent('claude_code', { state: 'signed_out' })]);
    clickTab('commercial');
    const toggle = await screen.findByTestId('provider-row-toggle-claude_code');
    const name = within(toggle).getByText('Claude Code');
    const chevron = toggle.querySelector('.br-provider-chevron');
    expect(chevron).not.toBeNull();
    // The name comes first in the toggle's DOM, and nothing precedes it.
    expect(toggle.querySelector('p, svg')).toBe(name);
    expect(name.compareDocumentPosition(chevron!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(chevron).toHaveAttribute('data-expanded', 'false');
    fireEvent.click(toggle);
    expect(chevron).toHaveAttribute('data-expanded', 'true');
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

/**
 * W2-PRV-1. Configs saved before the Azure OpenAI default was removed keep
 * UCSF's gateway as their endpoint, and send a commercial key and every
 * transcript there under a Public label. The row says so, and offers Configure;
 * nothing is rewritten.
 */
describe('Azure OpenAI saved with the retired default endpoint', () => {
  const renderCatalog = () =>
    render(
      <ProviderCatalog
        providers={[provider('azure_openai', {}, 'Azure OpenAI')]}
        mode="settings"
        refreshProviders={vi.fn()}
        initialTab="public"
      />
    );

  it('says what the endpoint is, with Configure', async () => {
    mocks.read.mockImplementation(async (key: string) =>
      key === 'AZURE_OPENAI_ENDPOINT' ? 'https://unified-api.ucsf.edu/general/' : null
    );
    renderCatalog();
    const notice = await screen.findByTestId('azure-retired-endpoint-notice');
    expect(notice).toHaveTextContent("This endpoint is UCSF's gateway");
    expect(notice).toHaveTextContent('use Versa API Azure for UCSF');
    expect(within(notice).getByRole('button', { name: 'Configure' })).toBeInTheDocument();
    expect(mocks.upsert).not.toHaveBeenCalled();
  });

  it('says nothing for the person’s own Azure resource', async () => {
    mocks.read.mockImplementation(async (key: string) =>
      key === 'AZURE_OPENAI_ENDPOINT' ? 'https://my-company.openai.azure.com' : null
    );
    renderCatalog();
    await waitFor(() => expect(mocks.read).toHaveBeenCalledWith('AZURE_OPENAI_ENDPOINT', false));
    expect(screen.queryByTestId('azure-retired-endpoint-notice')).toBeNull();
  });
});

/**
 * W2-PRV-13. A built-in declarative provider opened as "Configure  Provider"
 * with "Update Provider" and "Leave blank to keep existing key" whether or not
 * a key was ever saved.
 */
describe('a declarative provider nobody has set up', () => {
  it('opens as "Set up <name>", asking for the key', async () => {
    mocks.getCustomProvider.mockResolvedValue({
      data: {
        is_editable: false,
        config: {
          engine: 'openai',
          display_name: 'DeepSeek',
          base_url: 'https://api.deepseek.com',
          models: [{ name: 'deepseek-chat' }],
          supports_streaming: true,
        },
      },
    });
    const deepseek = {
      ...provider('custom_deepseek', { is_configured: false }, 'DeepSeek'),
      provider_type: 'Declarative',
    } as ProviderDetails;
    render(
      <ProviderCatalog
        providers={[deepseek]}
        mode="settings"
        refreshProviders={vi.fn()}
        initialTab="public"
      />
    );
    fireEvent.click(await screen.findByTestId('provider-card-custom_deepseek'));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Set up DeepSeek')).toBeInTheDocument();
    expect(within(dialog).queryByText(/Provider$/)).toBeNull();
    expect(within(dialog).getByRole('button', { name: 'Save' })).toBeInTheDocument();
    expect(within(dialog).queryByPlaceholderText(/keep existing key/)).toBeNull();
  });
});

/**
 * Review of W2-PRV-2, round 3. The daemon refuses an update that moves a custom
 * provider's URL while keeping its saved key unless the request proves a person
 * made it, so the catalog's update carries that proof.
 */
describe('updating a custom provider', () => {
  it('sends the proof that the user made the change', async () => {
    mocks.getCustomProvider.mockResolvedValue({
      data: {
        is_editable: true,
        config: {
          engine: 'openai',
          display_name: 'Lab gateway',
          base_url: 'https://lab.example/v1',
          models: [{ name: 'lab-model' }],
          supports_streaming: true,
        },
      },
    });
    mocks.updateCustomProvider.mockResolvedValue({ data: 'ok' });
    const lab = {
      ...provider('custom_lab', { is_configured: true }, 'Lab gateway'),
      provider_type: 'Custom',
    } as ProviderDetails;
    render(
      <ProviderCatalog
        providers={[lab]}
        mode="settings"
        refreshProviders={vi.fn()}
        initialTab="public"
      />
    );
    fireEvent.click(await screen.findByTestId('provider-card-custom_lab'));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: customProviderCopy.update }));
    await waitFor(() => expect(mocks.updateCustomProvider).toHaveBeenCalledTimes(1));
    expect(mocks.updateCustomProvider.mock.calls[0][0]).toMatchObject({
      path: { id: 'custom_lab' },
      headers: { 'X-User-Action': 'test-key' },
    });
  });
});

/**
 * The catalog's anatomy (codex simplicity redesign, spec 3.13): rows on the settings row
 * geometry, a section's reason behind an InfoTip beside its heading, the institution line as a
 * quiet question with its answer one hover away, and no bordered card anywhere in a panel.
 */
describe('ProviderCatalog — anatomy', () => {
  it('draws a provider as a settings row, with no tile and no card', () => {
    render(
      <ProviderCatalog
        providers={[provider('anthropic', {}, 'Anthropic')]}
        mode="settings"
        configuredProvider={null}
        initialTab="public"
      />
    );
    const row = screen.getByTestId('provider-card-anthropic');
    expect(row).toHaveClass('biorouter-settings-row');
    expect(row.className).not.toMatch(/rounded-container|py-3|px-4/);
    // The list is the shared settings list, so the rows share one hairline recipe.
    expect(row.parentElement).toHaveClass('biorouter-settings-list');
    // ...and the catalog opts out of the line by the shared scope (R-27), which draws it again in
    // forced colours; `providers.css` keys the hovered row's fill on this same element.
    const catalog = row.closest('.br-provider-catalog');
    expect(catalog).not.toBeNull();
    expect(catalog).toHaveClass('br-rows-flat');
  });

  it('puts "Add custom provider" in the API list as a row', () => {
    render(
      <ProviderCatalog
        providers={[provider('anthropic', {}, 'Anthropic')]}
        mode="settings"
        configuredProvider={null}
        initialTab="public"
      />
    );
    const add = screen.getByTestId('add-custom-provider-card');
    expect(add).toHaveClass('biorouter-settings-row');
    expect(add).toHaveTextContent(catalogCopy.addCustom);
    expect(add.querySelector('svg')).not.toBeNull();
  });

  it('gives the agents’ section its reason as an InfoTip beside the heading', async () => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({
      agents: [agent('claude_code', { state: 'signed_out' })],
    });
    render(
      <ProviderCatalog
        providers={[provider('claude_code', {}, 'Claude Code'), provider('anthropic')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('commercial');
    const section = await screen.findByTestId('catalog-section-agents');
    expect(
      within(section).getByRole('heading', { name: 'AI agents · your subscription' })
    ).toBeInTheDocument();
    expect(
      within(section).getByRole('button', { name: 'About AI agents · your subscription' })
    ).toBeInTheDocument();
    // No paragraph under the heading.
    expect(section.querySelector('h2 + p')).toBeNull();
  });

  it('gives an institution’s section no tip: its note would only list the rows beneath it', () => {
    render(
      <ProviderCatalog
        providers={[
          provider('versa_azure', {
            ...PRIVATE_REMOTE,
            institutions: UCSF,
            resolved_tier: 'private',
            affiliation: { kind: 'institutions', institutions: UCSF },
          }),
        ]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('institutional');
    const section = screen.getByTestId('catalog-section-ucsf');
    expect(within(section).getByRole('heading', { name: 'UCSF' })).toBeInTheDocument();
    expect(within(section).queryByRole('button', { name: /^About / })).toBeNull();
  });

  it('asks "Don’t see your institution?" in one quiet line, with the answer in its InfoTip', () => {
    render(<ProviderCatalog providers={[]} mode="settings" configuredProvider={null} />);
    clickTab('institutional');
    const line = screen.getByTestId('other-institutions-note');
    expect(line).toHaveTextContent(catalogCopy.otherInstitutions);
    expect(line.className).not.toMatch(/border|rounded/);
    const tip = within(line).getByRole('button', {
      name: `About ${catalogCopy.otherInstitutions}`,
    });
    expect(tip).toBeInTheDocument();
    // Words, not an arrow.
    expect(catalogCopy.otherInstitutionsHelp).not.toMatch(/→/);
  });

  it('opens a panel with its tier line, not a heading that repeats the tab', () => {
    render(
      <ProviderCatalog
        providers={[provider('llamacpp', PRIVATE_LOCAL, 'Llama Server')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('local');
    expect(screen.queryByRole('heading', { name: /Private · Local/ })).toBeNull();
    expect(document.querySelector('.br-provider-tier-note')).toHaveTextContent(
      /^Private because inference runs on this machine/
    );
  });

  // V4-05 (request WS-PRIMITIVES-3). Radix makes every panel a Tab stop and this one draws no
  // focus edge, so Tab from the tab list would land on a panel that shows nothing.
  it('hands the panel’s Tab stop to its first control when it has rows', () => {
    render(
      <ProviderCatalog
        providers={[provider('llamacpp', PRIVATE_LOCAL, 'Llama Server')]}
        mode="settings"
        configuredProvider={null}
      />
    );
    clickTab('local');
    const panel = screen.getByRole('tabpanel');
    expect(panel).toHaveAttribute('tabindex', '-1');
    expect(within(panel).getAllByRole('button').length).toBeGreaterThan(0);
  });

  it('keeps the panel as a Tab stop when it has no control to hand it to', () => {
    render(<ProviderCatalog providers={[]} mode="settings" configuredProvider={null} />);
    clickTab('institutional');
    const panel = screen.getByRole('tabpanel');
    expect(within(panel).getByTestId('catalog-empty-institutional')).toBeInTheDocument();
    expect(panel).toHaveAttribute('tabindex', '0');
  });
});

/**
 * FR-R1-1 (first-run vision round 1). The first screen painted Local with "Nothing here yet." while
 * the list was in flight, filled it a beat later, then switched itself to Public when the agent
 * probe answered: a false empty state and a tab change under the pointer. A held catalog draws a
 * skeleton until its opening tab is known and then opens there at rest, and keeps it.
 */
describe('ProviderCatalog — settling (FR-R1-1)', () => {
  const rows = [
    provider('llamacpp', PRIVATE_LOCAL, 'Llama Server'),
    provider('claude_code', {}, 'Claude Code'),
    provider('anthropic', {}, 'Anthropic'),
  ];
  const ready = { agents: [agent('claude_code', { state: 'signed_in_subscription' })] };

  /** A probe that answers only when the test says so. */
  const pendingProbe = () => {
    let answer!: (value: unknown) => void;
    mocks.fetchCodingAgentStatus.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    return (value: unknown) => answer(value);
  };

  afterEach(() => {
    vi.useRealTimers();
  });

  it('draws the skeleton, never an empty tab, while the provider list loads', () => {
    render(<ProviderCatalog providers={[]} loading mode="onboarding" configuredProvider={null} />);
    expect(screen.getByRole('status', { name: providerPageCopy.loading })).toBeInTheDocument();
    expect(screen.queryByText(catalogCopy.emptyTab)).toBeNull();
    expect(screen.queryByRole('tab')).toBeNull();
  });

  it('waits for the agent probe when it can decide the tab, then opens on its answer at rest', async () => {
    const answer = pendingProbe();
    render(
      <ProviderCatalog
        providers={rows}
        mode="onboarding"
        configuredProvider={null}
        holdUntilSettled
      />
    );
    // The list is in, but a signed-in agent would open Public, so Local is not drawn first.
    expect(screen.getByRole('status', { name: providerPageCopy.loading })).toBeInTheDocument();
    expect(screen.queryByRole('tab')).toBeNull();

    await act(async () => answer(ready));
    expect(openTab()).toBe('commercial');
    expect(screen.queryByRole('status', { name: providerPageCopy.loading })).toBeNull();
  });

  it('opens without the probe once the wait is over, and a late answer never moves the tab', async () => {
    vi.useFakeTimers();
    const answer = pendingProbe();
    render(
      <ProviderCatalog
        providers={rows}
        mode="onboarding"
        configuredProvider={null}
        holdUntilSettled
      />
    );
    expect(screen.queryByRole('tab')).toBeNull();

    act(() => {
      vi.advanceTimersByTime(AGENT_PROBE_WAIT_MS);
    });
    expect(openTab()).toBe('local');

    // The probe answers after the catalog opened: the person is already reading Local.
    await act(async () => answer(ready));
    expect(openTab()).toBe('local');
  });

  it('does not wait for the probe when a bound provider already decides the tab', () => {
    pendingProbe();
    render(
      <ProviderCatalog
        providers={rows}
        mode="settings"
        configuredProvider="anthropic"
        holdUntilSettled
      />
    );
    expect(openTab()).toBe('commercial');
  });

  it('does not wait for the probe when a route hint decides the tab', () => {
    pendingProbe();
    render(
      <ProviderCatalog
        providers={rows}
        mode="settings"
        configuredProvider={null}
        initialTab="local"
        holdUntilSettled
      />
    );
    expect(openTab()).toBe('local');
  });

  it('holds while the bound provider has not been read yet', async () => {
    mocks.fetchCodingAgentStatus.mockResolvedValue({ agents: [] });
    const { rerender } = render(
      <ProviderCatalog
        providers={rows}
        mode="settings"
        configuredProvider={undefined}
        holdUntilSettled
      />
    );
    expect(screen.queryByRole('tab')).toBeNull();
    rerender(
      <ProviderCatalog
        providers={rows}
        mode="settings"
        configuredProvider="anthropic"
        holdUntilSettled
      />
    );
    await waitFor(() => expect(openTab()).toBe('commercial'));
  });
});
