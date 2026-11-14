import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ProviderGuard from './ProviderGuard';
import { BROWSER_SURFACE_MARKER } from '../utils/surface';
import { resetProviderSetupAccessForTests } from '../utils/providerSetupAccess';
import { firstRunCopy } from './onboarding/copy';
import { HOST_KEY_STORAGE_NOTICE } from './privacy/hostManagedModelCopy';

/**
 * SD-1's dead end, and SD-18's way out of it.
 *
 * A fresh browser user lands in onboarding, which is exactly where provider selection happens,
 * and every card there writes `BIOROUTER_PROVIDER`. A daemon that admits the served page to
 * provider setup (SD-18, `GET /config/provider_setup` says `allowed`) takes those writes, so the
 * browser gets the desktop's own first-run page. One that does not would answer every card with
 * a 409 addressed to an AI agent, so that browser keeps the host panel: the daemon's reason and
 * the command to run on the host.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
  getProviders: vi.fn(),
  navigate: vi.fn(),
  getProviderSetupAccess: vi.fn(),
  catalog: { onModelSelected: undefined as undefined | (() => void) },
}));

vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getProviderSetupAccess: mocks.getProviderSetupAccess,
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    upsert: mocks.upsert,
    getProviders: mocks.getProviders,
  }),
}));

vi.mock('react-router-dom', () => ({
  useNavigate: () => mocks.navigate,
  useLocation: () => ({ pathname: '/' }),
}));

// The catalog gets a distinguishable marker, because the assertion that matters
// most here is an ABSENCE — a stub rendering `null` could not tell
// "not rendered" from "rendered and empty". It keeps the completion callback, so a test can
// finish setup the way the real catalog does.
vi.mock('./settings/providers/ProviderCatalog', () => ({
  default: ({ onModelSelected }: { onModelSelected?: () => void }) => {
    mocks.catalog.onModelSelected = onModelSelected;
    return <div>CATALOG</div>;
  },
}));

const CARD_MARKERS = ['CATALOG'];

function renderGuard() {
  return render(
    <ProviderGuard didSelectProvider={false}>
      <div>Application</div>
    </ProviderGuard>
  );
}

const DAEMON_REASON = 'This server keeps its model choice for the person who started it.';

describe('ProviderGuard on a browser-served surface', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetProviderSetupAccessForTests();
    mocks.catalog.onModelSelected = undefined;
    mocks.read.mockResolvedValue('');
    mocks.upsert.mockResolvedValue(undefined);
    mocks.getProviders.mockResolvedValue([]);
    mocks.getProviderSetupAccess.mockResolvedValue({
      data: { allowed: false, reason: DAEMON_REASON },
    });
  });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
    resetProviderSetupAccessForTests();
  });

  describe('when the daemon refuses provider setup', () => {
    /**
     * ⚠ The user must not reach a picker whose every path ends in a refusal.
     *
     * ⚠ **The skip is withheld here too**, and for the same reason rather than as
     * an oversight: "continue without a provider" would lead to a chat that this
     * tab can never configure — a second dead end wearing the clothes of an escape.
     */
    it('replaces the catalog with the reason the daemon gave and the command to run on the host', async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      renderGuard();

      const panel = await screen.findByTestId('host-managed-model-panel');
      expect(panel).toHaveTextContent('biorouter configure');
      // The daemon knows why it refused, so its sentence is the panel's one visible line.
      expect(screen.getByTestId('host-managed-model-reason')).toHaveTextContent(DAEMON_REASON);
      // Named, so a reader knows the choice is on the host rather than missing.
      expect(screen.getByText('Choose a model on the host')).toBeInTheDocument();

      for (const marker of CARD_MARKERS) {
        expect(screen.queryByText(marker)).toBeNull();
      }
      expect(screen.queryByTestId('onboarding-skip-header')).toBeNull();
      expect(mocks.getProviders).not.toHaveBeenCalled();
    });

    // FR-S-13: the panel carries the reason, so the band above it says nothing of its own beyond
    // its title: the sentence that used to sit under the old header said the same thing twice.
    it('heads the panel with the band alone, with no second sentence above it', async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      renderGuard();
      await screen.findByTestId('host-managed-model-panel');
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(firstRunCopy.hostTitle);
      expect(screen.queryByText(/is being served to this browser/)).toBeNull();
      expect(screen.queryByRole('tab')).toBeNull();
    });

    /**
     * The reason, not just the instruction. SD-1 is a privacy boundary, and a screen that said
     * only "run this on the host" would read as a limitation rather than as the thing keeping a
     * private conversation off a public model. It is the title's InfoTip, which is always in
     * the page for a screen reader.
     */
    it('says why the choice stays on the host, not only that it does', async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      renderGuard();

      const panel = await screen.findByTestId('host-managed-model-panel');
      expect(panel.textContent).toMatch(/private conversation/i);
      expect(panel.textContent).toMatch(/public model/i);
    });

    /** Fail closed: a daemon that cannot answer (an older one has no route) keeps the panel. */
    it('keeps the host panel when the daemon cannot say', async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      vi.spyOn(console, 'error').mockImplementation(() => undefined);
      mocks.getProviderSetupAccess.mockRejectedValue(new Error('404'));
      renderGuard();

      const panel = await screen.findByTestId('host-managed-model-panel');
      expect(panel).toHaveTextContent('biorouter configure');
      expect(screen.queryByText('CATALOG')).toBeNull();
    });
  });

  describe('when the daemon allows provider setup (SD-18)', () => {
    beforeEach(() => {
      mocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: true } });
    });

    it('renders the desktop first-run page: the catalog, Skip and Open Crew', async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      renderGuard();

      expect(await screen.findByText('CATALOG')).toBeInTheDocument();
      expect(screen.queryByTestId('host-managed-model-panel')).toBeNull();
      expect(screen.queryByText(/biorouter configure/)).toBeNull();
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(firstRunCopy.title);
      expect(screen.getByTestId('onboarding-skip-header')).toBeInTheDocument();
      expect(screen.getByTestId('onboarding-open-crew')).toBeInTheDocument();
      // The catalog needs the daemon's provider list, which a browser now fetches too.
      await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
    });

    // A browser's keys are written on the host, so the band's help says where, not "keychain".
    it('says the keys are kept on the host, not in the browser', async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      renderGuard();
      await screen.findByText('CATALOG');
      expect(screen.getByTestId('first-run-band')).toHaveTextContent(HOST_KEY_STORAGE_NOTICE);
    });

    it('continues into the app on its own once setup completes', async () => {
      document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
      renderGuard();
      await screen.findByText('CATALOG');

      act(() => mocks.catalog.onModelSelected?.());

      expect(await screen.findByText('Application')).toBeInTheDocument();
      expect(screen.queryByText('CATALOG')).toBeNull();
      expect(mocks.navigate).toHaveBeenCalledWith('/', { replace: true });
    });
  });

  /**
   * ⚠ **The control.** Its job is to catch over-reach: a helper that answered `browser`
   * whenever `window.electron` looked unusual — or one that read a module-level snapshot —
   * would strip the desktop application's own onboarding down to a panel telling the user to
   * go and run a command in a terminal. The desktop never asks the daemon either.
   */
  it('leaves the desktop onboarding exactly as it was', async () => {
    renderGuard();

    for (const marker of CARD_MARKERS) {
      expect(await screen.findByText(marker)).toBeInTheDocument();
    }
    expect(screen.queryByTestId('host-managed-model-panel')).toBeNull();
    // …and the desktop keeps its way past the wall.
    expect(screen.getByTestId('onboarding-skip-header')).toBeInTheDocument();
    expect(mocks.getProviderSetupAccess).not.toHaveBeenCalled();
  });

  /**
   * A browser session whose host *has* a provider is a working session, and
   * must not be diverted. The panel is for the unconfigured case only.
   *
   * ⚠ Fails against a plausible wrong implementation that renders the panel
   * whenever the surface is a browser, rather than only inside the
   * no-provider branch.
   */
  it('does not divert a browser session whose host is already configured', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    mocks.read.mockResolvedValue('versa_azure');
    renderGuard();

    await waitFor(() => expect(screen.getByText('Application')).toBeInTheDocument());
    expect(screen.queryByTestId('host-managed-model-panel')).toBeNull();
  });
});
