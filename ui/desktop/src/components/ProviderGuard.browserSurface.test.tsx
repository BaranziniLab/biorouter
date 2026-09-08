import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ProviderGuard from './ProviderGuard';
import { BROWSER_SURFACE_MARKER } from '../utils/surface';

/**
 * SD-1's dead end: a fresh browser user lands in onboarding, which is exactly
 * where provider selection happens, and every card there writes
 * `BIOROUTER_PROVIDER` — a write the browser-served daemon refuses with a 409
 * whose body is addressed to an AI agent and tells the reader to open the
 * desktop application they do not have.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
  getProviders: vi.fn(),
  navigate: vi.fn(),
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
}));

// The catalog gets a distinguishable marker, because the assertion that matters
// most here is an ABSENCE — a stub rendering `null` could not tell
// "not rendered" from "rendered and empty".
vi.mock('./settings/providers/ProviderCatalog', () => ({
  default: () => <div>CATALOG</div>,
}));

const CARD_MARKERS = ['CATALOG'];

function renderGuard() {
  return render(
    <ProviderGuard didSelectProvider={false}>
      <div>Application</div>
    </ProviderGuard>
  );
}

describe('ProviderGuard on a browser-served surface', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue('');
    mocks.upsert.mockResolvedValue(undefined);
    mocks.getProviders.mockResolvedValue([]);
  });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  /**
   * ⚠ The user must not reach a picker whose every path ends in a refusal: every
   * route through the catalog writes `BIOROUTER_PROVIDER`, and a browser-served
   * daemon refuses that write with a 409 addressed to an AI agent.
   *
   * ⚠ **The skip is withheld here too**, and for the same reason rather than as
   * an oversight: "continue without a provider" would lead to a chat that this
   * tab can never configure — a second dead end wearing the clothes of an escape.
   */
  it('replaces the provider catalog with what to run on the host', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderGuard();

    const panel = await screen.findByTestId('host-managed-model-panel');
    expect(panel).toHaveTextContent('biorouter configure');
    expect(panel).toHaveTextContent('biorouter serve');
    // Named, so a reader knows the choice is on the host rather than missing.
    expect(screen.getByText('Choose a model on the host')).toBeInTheDocument();

    for (const marker of CARD_MARKERS) {
      expect(screen.queryByText(marker)).toBeNull();
    }
    expect(screen.queryByTestId('onboarding-skip-header')).toBeNull();
  });

  /**
   * The reason, not just the instruction. SD-1 is a privacy boundary, and a
   * screen that said only "run this on the host" would read as a limitation
   * rather than as the thing keeping a private conversation off a public model.
   *
   */
  it('says why a browser tab cannot choose, not only that it cannot', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderGuard();

    const panel = await screen.findByTestId('host-managed-model-panel');
    expect(panel.textContent).toMatch(/private conversation/i);
    expect(panel.textContent).toMatch(/public model/i);
  });

  /**
   * ⚠ **The control, and the one test here that passes both before and after.**
   * Its job is to catch over-reach: a helper that answered `browser` whenever
   * `window.electron` looked unusual — or one that read a module-level snapshot
   * — would strip the desktop application's own onboarding down to a panel
   * telling the user to go and run a command in a terminal.
   */
  it('leaves the desktop onboarding exactly as it was', async () => {
    renderGuard();

    for (const marker of CARD_MARKERS) {
      expect(await screen.findByText(marker)).toBeInTheDocument();
    }
    expect(screen.queryByTestId('host-managed-model-panel')).toBeNull();
    // …and the desktop keeps its way past the wall.
    expect(screen.getByTestId('onboarding-skip-header')).toBeInTheDocument();
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
