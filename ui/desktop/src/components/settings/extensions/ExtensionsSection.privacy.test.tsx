import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  extensionsList: [] as Array<Record<string, unknown>>,
  addExtension: vi.fn(async () => undefined),
  removeExtension: vi.fn(async () => undefined),
  getExtensions: vi.fn(async () => []),
  read: vi.fn(async () => 'anthropic'),
  getProviders: vi.fn(async () => [
    {
      name: 'anthropic',
      is_configured: true,
      provider_type: 'Builtin',
      metadata: { name: 'anthropic', display_name: 'Anthropic', tier: 'public' },
    },
    {
      name: 'versa_azure',
      is_configured: true,
      provider_type: 'Builtin',
      metadata: { name: 'versa_azure', display_name: 'Versa', tier: 'private' },
    },
  ]),
  toggleExtensionDefault: vi.fn(async () => undefined),
  activateExtensionDefault: vi.fn(async () => undefined),
  deleteExtension: vi.fn(async () => undefined),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: mocks.extensionsList,
    addExtension: mocks.addExtension,
    removeExtension: mocks.removeExtension,
    getExtensions: mocks.getExtensions,
    read: mocks.read,
    getProviders: mocks.getProviders,
  }),
  // `PrivacyBadge` reads the master switch off this same context and fails
  // loudly if the mock omits it — see the warning in `ui/PrivacyBadge.tsx`.
  usePrivacyTiersEnabled: () => true,
}));

vi.mock('./index', () => ({
  toggleExtensionDefault: mocks.toggleExtensionDefault,
  activateExtensionDefault: mocks.activateExtensionDefault,
  deleteExtension: mocks.deleteExtension,
}));

vi.mock('../../../toasts', () => ({
  toastService: { success: vi.fn(), error: vi.fn() },
}));

import ExtensionsSection from './ExtensionsSection';

const omop = { type: 'stdio', name: 'ucsfomopagent', description: 'UCSF OMOP', enabled: true };
// NOT `developer` — that key is a shipped capability, which `ExtensionsSection`
// filters out of this list entirely (`isCapabilityExtension`), so a fixture
// built on it renders nothing and every assertion about it is vacuous.
const spoke = { type: 'stdio', name: 'spokeagent', description: 'SPOKE graph', enabled: true };

describe('ExtensionsSection — the pairing state Settings can actually compute', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.extensionsList = [omop, spoke];
    mocks.read.mockResolvedValue('anthropic');
  });

  it('the Settings extension card states which provider it judged against', async () => {
    render(<ExtensionsSection hideButtons />);

    expect(
      await screen.findByText(/unavailable in new chats \(default model is public\)/i)
    ).toBeInTheDocument();
    // ⚠ "The focused session" does not exist here: Settings has no session
    // awareness, and with tabs and splits there is no single focused chat once
    // the user has navigated away. So the card must name the thing it DID judge
    // against — the global default provider.
    expect(screen.getByText(/Anthropic/)).toBeInTheDocument();
  });

  it('says nothing about a private extension when the default provider is private', async () => {
    mocks.read.mockResolvedValue('versa_azure');
    render(<ExtensionsSection hideButtons />);

    await screen.findByText(/UCSF OMOP/);
    expect(screen.queryByText(/unavailable in new chats/i)).toBeNull();
  });

  it('says nothing about a public extension under a public default provider', async () => {
    mocks.extensionsList = [spoke];
    render(<ExtensionsSection hideButtons />);

    await screen.findByText(/SPOKE graph/);
    expect(screen.queryByText(/unavailable in new chats/i)).toBeNull();
  });

  /**
   * §13.5. Without this the whole catalogue pass-through — section → list →
   * item — could break silently and every card would simply say less, which is
   * indistinguishable from a slow load. The strings are asserted verbatim
   * because they are the design's, not the implementation's.
   *
   * `window.electron` is absent here, so `loadRegistry` falls all the way
   * through to the snapshot bundled with the app — which is the point: the
   * provenance a user reads must not depend on the network being up.
   */
  it('every card says where it came from, in §13.5 strings', async () => {
    render(<ExtensionsSection hideButtons />);

    expect(
      await screen.findByText('Private: published on the Biorouter marketplace')
    ).toBeInTheDocument();
    expect(screen.getByText('Public: published on the Biorouter marketplace')).toBeInTheDocument();
    // The catalogue on screen is the bundled one, and the screen says so.
    expect(screen.getByText(/showing bundled catalog \(offline\)/i)).toBeInTheDocument();
  });

  /**
   * §13.5 asks for "a badge plus provenance on every card". The provenance
   * prose landed and the badge did not, which is the half that survives being
   * skimmed — a user scanning twenty rows for the one that is Private reads
   * pills, not sentences.
   */
  it('every card carries the badge, not only the provenance sentence', async () => {
    render(<ExtensionsSection hideButtons />);
    await screen.findByText(/UCSF OMOP/);

    const badges = screen.getAllByTestId('privacy-badge');
    expect(badges).toHaveLength(2);
    expect(badges.map((b) => b.getAttribute('data-privacy')).sort()).toEqual(['private', 'public']);
  });

  /**
   * "Every card" includes the ones neither string fits. A built-in is not on the
   * marketplace and was not installed from a file, so both §13.5 marketplace
   * strings would be false statements — but saying nothing leaves the row that
   * most obviously ships with the app as the only row that will not say where it
   * came from.
   */
  it('a built-in card is badged and claims neither the marketplace nor a file', async () => {
    mocks.extensionsList = [
      {
        type: 'builtin',
        name: 'somebundledserver',
        display_name: 'Some Bundled Server',
        description: 'Ships with Biorouter',
        enabled: true,
      },
    ];
    render(<ExtensionsSection hideButtons />);
    await screen.findByText(/Ships with Biorouter/);

    expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'public');
    // Not a bare /marketplace/i: the freshness line above the list legitimately
    // says "Marketplace catalog", and matching it would make this pass for the
    // wrong reason. Both §13.5 marketplace sentences, by name.
    expect(screen.queryByText(/published on the Biorouter marketplace/i)).toBeNull();
    expect(screen.queryByText(/installed from a file/i)).toBeNull();
    expect(screen.getByText(/built into Biorouter/i)).toBeInTheDocument();
  });

  it('an extension the catalogue does not list is named as installed from a file', async () => {
    mocks.extensionsList = [
      { type: 'stdio', name: 'medcp', description: 'Local clinical MCP', enabled: true },
    ];
    render(<ExtensionsSection hideButtons />);

    expect(
      await screen.findByText(
        'Public: installed from a file, not on the marketplace. Any model can call it.'
      )
    ).toBeInTheDocument();
  });
});
