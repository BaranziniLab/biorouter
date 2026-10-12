import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { createPortal } from 'react-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProviderGuard, { ONBOARDING_SKIPPED_KEY } from './ProviderGuard';
import { persistDetectedProviderSetup } from './onboarding/CommercialSetupCard';
import { firstRunCopy } from './onboarding/copy';
import { providerPageCopy } from './settings/providers/copy';
import {
  resetFirstRunNoticeDecisionForTests,
  startFirstRunNoticeCount,
} from './privacy/firstRunNoticeDecision';
import { FirstRunPrivacyNoticeGate } from './privacy/FirstRunPrivacyNoticeGate';
import {
  holdFirstRunNoticeForFirstPage,
  resetFirstRunNoticeHandoffForTests,
} from './privacy/firstRunNoticeHandoff';
import type { Session } from '../api';

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
  getProviders: vi.fn(),
  navigate: vi.fn(),
  listSessions: vi.fn(),
  location: { pathname: '/', search: '' },
}));

// The day-one notice's count, which the first-run page waits for (FR-R4-4). The real decision module
// runs; only the daemon call under it is the test's.
vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  listSessions: mocks.listSessions,
}));

vi.mock('../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  userActionHeaders: async () => ({}),
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
  useLocation: () => mocks.location,
}));

/**
 * The catalog is stubbed to a marker plus one button per callback the guard
 * owns. What is under test here is the *guard*: which screen it shows, what it
 * writes, and when it lets the application through — not the catalog's own
 * rendering, which has its own suites next door.
 */
vi.mock('./settings/providers/ProviderCatalog', () => ({
  default: ({
    onCommercialSuccess,
    onLocalComplete,
    loading,
    holdUntilSettled,
    tabsHost,
  }: {
    onCommercialSuccess?: (setup: unknown) => void | Promise<void>;
    onLocalComplete?: () => void;
    loading?: boolean;
    holdUntilSettled?: boolean;
    tabsHost?: HTMLElement | null;
  }) => (
    <div
      data-testid="catalog-stub"
      data-loading={String(Boolean(loading))}
      data-hold={String(Boolean(holdUntilSettled))}
    >
      CATALOG
      <button
        onClick={() =>
          void onCommercialSuccess?.({
            provider: 'xiaomi_mimo',
            model: 'mimo-v2-flash',
            models: ['mimo-v2-flash'],
            apiKey: 'mimo-secret',
            apiKeyConfigKey: 'XIAOMI_MIMO_API_KEY',
            extraConfig: { XIAOMI_MIMO_HOST: 'https://api.xiaomimimo.com' },
          })
        }
      >
        Complete detection
      </button>
      <button onClick={() => onLocalComplete?.()}>Local ready</button>
      {/* The catalog draws its tabs into the band's slot; a marker proves where the slot is. */}
      {tabsHost ? createPortal(<span data-testid="tabs-marker" />, tabsHost) : null}
    </div>
  ),
}));

function renderGuard() {
  return render(
    <ProviderGuard didSelectProvider={false}>
      <div>Application</div>
    </ProviderGuard>
  );
}

/** `read` answers per key, which is what the skip flow turns on. */
function configReads({ provider = '', skipped = false }: { provider?: string; skipped?: boolean }) {
  mocks.read.mockImplementation(async (key: string) =>
    key === ONBOARDING_SKIPPED_KEY ? skipped : provider
  );
}

describe('ProviderGuard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.location.pathname = '/';
    mocks.location.search = '';
    configReads({});
    mocks.upsert.mockResolvedValue(undefined);
    mocks.getProviders.mockResolvedValue([]);
    localStorage.clear();
    resetFirstRunNoticeDecisionForTests();
    resetFirstRunNoticeHandoffForTests();
    mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
  });

  it('persists the detected provider contract before opening its model picker', async () => {
    renderGuard();

    fireEvent.click(await screen.findByRole('button', { name: 'Complete detection' }));

    await waitFor(() => {
      expect(mocks.upsert.mock.calls).toEqual([
        ['XIAOMI_MIMO_API_KEY', 'mimo-secret', true],
        ['XIAOMI_MIMO_HOST', 'https://api.xiaomimimo.com', false],
        ['BIOROUTER_PROVIDER', 'xiaomi_mimo', false],
      ]);
    });
  });

  // FR-R1-7, R-29: the wordmark is the logo. The loader is a frame or two between the boot splash
  // and the page, and it used to draw an 84px BR monogram on the recessed grey: a second logo and
  // a second ground before the page's wordmark. It now draws nothing, on the page ground.
  it('draws no logo while it reads the provider, on the ground the page stands on', async () => {
    let answer!: (value: unknown) => void;
    mocks.read.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    const { container } = renderGuard();
    const loader = screen.getByTestId('provider-guard-loading');
    expect(loader).toHaveClass('bg-background-canvas');
    expect(loader).not.toHaveClass('bg-background-muted');
    expect(loader).toHaveAttribute('aria-busy', 'true');
    expect(loader).toBeEmptyDOMElement();
    expect(container.querySelector('svg')).toBeNull();
    expect(screen.queryByRole('img')).toBeNull();
    answer('');
    expect(await screen.findByText('CATALOG')).toBeInTheDocument();
    expect(screen.queryByTestId('provider-guard-loading')).toBeNull();
  });

  it('blocks the application while neither a provider nor a skip is recorded', async () => {
    renderGuard();
    expect(await screen.findByText('CATALOG')).toBeInTheDocument();
    expect(screen.queryByText('Application')).toBeNull();
  });

  // R-34, vision FR-R1-2 / FR-S-1 / FR-S-4: the first-run page is the redesigned app's page, one
  // 44px band and then the column on the page ground, as the `#/welcome` page and Settings are.
  it('stands on the page ground under one band, not a hero header on the recessed ground', async () => {
    const { container } = renderGuard();
    await screen.findByText('CATALOG');
    const root = container.firstElementChild as HTMLElement;
    expect(root.className).toContain('bg-background-canvas');
    expect(root.className).not.toContain('bg-background-muted');
    expect(root.className).not.toContain('br-ground-recessed');
    // The provider page's chrome rules: the band's lead clears the traffic lights, and the
    // titlebar strip stops taking the band's clicks (`providers.css`).
    expect(root.className).toContain('br-provider-page');

    const band = screen.getByTestId('first-run-band');
    expect(band).toHaveAttribute('data-band');
    expect(band).toHaveClass('biorouter-page-header');
  });

  it('puts the wordmark, a 14px title with its help, the tabs and the two ways out in the band', async () => {
    renderGuard();
    await screen.findByText('CATALOG');
    const band = screen.getByTestId('first-run-band');

    // The wordmark is the logo (R-29), at rest: no entrance animation on a launch (FR-S-15). The
    // old `.biorouter-icon-animation` rule is gone from main.css (309338135), so the check names
    // no class: nothing from the mark up to the body may carry an animation class or style.
    const mark = within(band).getByTestId('first-run-wordmark');
    for (let el: Element | null = mark; el && el !== document.body; el = el.parentElement) {
      expect(el.getAttribute('class') ?? '').not.toMatch(/animat|entrance/i);
      expect((el as HTMLElement).style.animationName ?? '').toBe('');
    }

    const title = within(band).getByRole('heading', { level: 1 });
    expect(title).toHaveTextContent(firstRunCopy.title);
    expect(title).toHaveClass('text-label');
    // The lead is the InfoTip's, as on `#/welcome`; no paragraph under the title.
    const tip = within(band).getByRole('button', { name: `About ${firstRunCopy.title}` });
    expect(tip).toHaveAccessibleDescription(
      expect.stringContaining(providerPageCopy.onboardingHelp)
    );
    expect(screen.queryByText(/integrated research environment/)).toBeNull();

    // The catalog's tabs are drawn into the band's slot.
    expect(within(band).getByTestId('tabs-marker')).toBeInTheDocument();

    // The ways out are buttons in one ink, with words and no arrow, and the skip is said once.
    const skip = within(band).getByRole('button', { name: firstRunCopy.skip });
    const crew = within(band).getByRole('button', { name: firstRunCopy.openCrew });
    for (const exit of [skip, crew]) expect(exit.textContent).not.toMatch(/→/);
    expect(screen.getAllByRole('button', { name: firstRunCopy.skip })).toHaveLength(1);
    expect(screen.queryByTestId('onboarding-skip-foot')).toBeNull();
  });

  it('opens Crew from the band', async () => {
    renderGuard();
    fireEvent.click(await screen.findByRole('button', { name: firstRunCopy.openCrew }));
    expect(mocks.navigate).toHaveBeenCalledWith('/crew');
  });

  // FR-R1-1: the catalog is told the list is still in flight, so it draws its skeleton rather
  // than a tab that says there is nothing in it, and it holds its opening tab until it is known.
  it('tells the catalog the provider list is loading until the daemon answers', async () => {
    let answer!: (value: unknown[]) => void;
    mocks.getProviders.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    renderGuard();
    const catalog = await screen.findByTestId('catalog-stub');
    expect(catalog).toHaveAttribute('data-loading', 'true');
    expect(catalog).toHaveAttribute('data-hold', 'true');
    answer([]);
    await waitFor(() => expect(catalog).toHaveAttribute('data-loading', 'false'));
  });

  it('stops loading when the list cannot be read, rather than holding the skeleton forever', async () => {
    mocks.getProviders.mockRejectedValue(new Error('daemon down'));
    renderGuard();
    const catalog = await screen.findByTestId('catalog-stub');
    await waitFor(() => expect(catalog).toHaveAttribute('data-loading', 'false'));
  });

  // FR-R4-4: on an upgraded install the privacy notice opens over this page, and its count took a
  // second longer than the page, so the notice rose over a person already reading the list. The
  // count starts at boot and the catalog keeps its skeleton until the answer is in, whichever way
  // it goes, so the page and the notice arrive together.
  describe('the day-one notice count', () => {
    /** A count the test settles by hand, started the way the boot starts it. */
    async function countInFlight() {
      let settle!: (sessions: Session[]) => void;
      let fail!: (error: unknown) => void;
      mocks.listSessions.mockReturnValue(
        new Promise((resolve, reject) => {
          settle = (sessions) => resolve({ data: { sessions } });
          fail = reject;
        })
      );
      startFirstRunNoticeCount();
      await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
      return { settle, fail };
    }

    const BACKFILLED = [
      {
        id: 's1',
        privacy_tier: 'private',
        privacy_reason: 'backfill:versa_azure',
        provider_name: 'versa_azure',
      },
    ] as unknown as Session[];

    it('keeps the skeleton after the provider list has answered, until the count does', async () => {
      const count = await countInFlight();
      renderGuard();
      const catalog = await screen.findByTestId('catalog-stub');
      await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
      // The list is in; the notice's answer is not. Nothing is drawn yet.
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(catalog).toHaveAttribute('data-loading', 'true');

      count.settle(BACKFILLED);
      await waitFor(() => expect(catalog).toHaveAttribute('data-loading', 'false'));
    });

    // The point of the hold. The same answer opens the notice and lets the catalog draw, so in the
    // first commit where the dialog exists the catalog is already out of its skeleton: the page
    // never settles first and then blurs under a person who has begun to read it.
    it('opens the notice in the same commit the catalog draws', async () => {
      const count = await countInFlight();
      render(
        <>
          <ProviderGuard didSelectProvider={false}>
            <div>Application</div>
          </ProviderGuard>
          <FirstRunPrivacyNoticeGate />
        </>
      );
      const catalog = await screen.findByTestId('catalog-stub');
      await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
      expect(catalog).toHaveAttribute('data-loading', 'true');
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();

      count.settle(BACKFILLED);
      await screen.findByTestId('first-run-privacy-notice');
      // No await since the dialog appeared: this is the commit it appeared in.
      expect(catalog).toHaveAttribute('data-loading', 'false');
    });

    it('lets go on a no-show answer too', async () => {
      const count = await countInFlight();
      renderGuard();
      const catalog = await screen.findByTestId('catalog-stub');
      expect(catalog).toHaveAttribute('data-loading', 'true');
      count.settle([]);
      await waitFor(() => expect(catalog).toHaveAttribute('data-loading', 'false'));
    });

    it('lets go when the daemon does not answer the count', async () => {
      const count = await countInFlight();
      renderGuard();
      const catalog = await screen.findByTestId('catalog-stub');
      count.fail(new Error('daemon down'));
      await waitFor(() => expect(catalog).toHaveAttribute('data-loading', 'false'));
    });

    it('waits for nothing when nobody started a count', async () => {
      renderGuard();
      const catalog = await screen.findByTestId('catalog-stub');
      await waitFor(() => expect(catalog).toHaveAttribute('data-loading', 'false'));
    });

    it('never takes a drawn catalog back to its skeleton when a count starts later', async () => {
      renderGuard();
      const catalog = await screen.findByTestId('catalog-stub');
      await waitFor(() => expect(catalog).toHaveAttribute('data-loading', 'false'));
      await countInFlight();
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(catalog).toHaveAttribute('data-loading', 'false');
    });

    it('does not hold an install that has a provider: it has no skeleton to hold', async () => {
      configReads({ provider: 'versa_azure' });
      await countInFlight();
      renderGuard();
      expect(await screen.findByText('Application')).toBeInTheDocument();
    });
  });

  // FR-R5-1: on a slower launch the boot's count lands while this guard is still reading the
  // provider, and the dialog opened at once, over the empty loader, with the band popping in behind
  // its blur. The gate now waits for the page this guard draws. The boot declares the wait
  // (`renderer.tsx`); this guard clears it from an effect once it stops showing the loader.
  describe('the day-one notice waits for this page (FR-R5-1)', () => {
    const DUE = [
      {
        id: 's1',
        privacy_tier: 'private',
        privacy_reason: 'backfill:versa_azure',
        provider_name: 'versa_azure',
      },
    ] as unknown as Session[];

    /**
     * How long the dialog gets once the page has drawn. Well under the hand-off's bound
     * (`FIRST_RUN_NOTICE_HANDOFF_WAIT_MS`, 3 s): a guard that never said it had drawn would still
     * see the dialog open, 3 s late, and a wait as long as the bound would call that a pass.
     */
    const PROMPT = { timeout: 1000 };

    /** The provider read, settled by hand: the guard shows its loader until it is. */
    function slowRead(answer: { provider?: string }) {
      let release!: () => void;
      const reading = new Promise<void>((resolve) => {
        release = resolve;
      });
      mocks.read.mockImplementation(async (key: string) => {
        await reading;
        return key === ONBOARDING_SKIPPED_KEY ? false : (answer.provider ?? '');
      });
      return release;
    }

    /** The launch as the boot starts it: the count is answered (the order that raced) and the page is awaited. */
    async function answeredAndAwaiting() {
      mocks.listSessions.mockResolvedValue({ data: { sessions: DUE } });
      startFirstRunNoticeCount();
      holdFirstRunNoticeForFirstPage();
      await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
      await new Promise((resolve) => setTimeout(resolve, 0));
    }

    function renderGuardAndGate() {
      return render(
        <>
          <ProviderGuard didSelectProvider={false}>
            <div>Application</div>
          </ProviderGuard>
          <FirstRunPrivacyNoticeGate />
        </>
      );
    }

    it('does not open over the loader, and opens once the first-run band is drawn', async () => {
      await answeredAndAwaiting();
      const release = slowRead({});
      renderGuardAndGate();

      // The count is in, the notice is due, and the guard is still reading: the empty loader only.
      expect(screen.getByTestId('provider-guard-loading')).toBeInTheDocument();
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();

      release();
      await screen.findByTestId('first-run-privacy-notice', {}, PROMPT);
      // The commit the dialog appeared in is after the band's: the page is under it, not behind it.
      expect(screen.getByTestId('first-run-band')).toBeInTheDocument();
      expect(screen.queryByTestId('provider-guard-loading')).toBeNull();
    });

    it('opens over the application when the install already has a provider', async () => {
      await answeredAndAwaiting();
      const release = slowRead({ provider: 'versa_azure' });
      renderGuardAndGate();
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();

      release();
      await screen.findByTestId('first-run-privacy-notice', {}, PROMPT);
      expect(screen.getByText('Application')).toBeInTheDocument();
    });

    it('draws at once on Crew, which never shows the loader', async () => {
      mocks.location.pathname = '/crew';
      await answeredAndAwaiting();
      renderGuardAndGate();
      expect(screen.getByText('Application')).toBeInTheDocument();
      await screen.findByTestId('first-run-privacy-notice', {}, PROMPT);
    });

    it('keeps the hold while the guard has not read its provider, however long that takes', async () => {
      await answeredAndAwaiting();
      slowRead({}); // never released
      renderGuardAndGate();
      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();
    });
  });

  it('lets a configured install straight through', async () => {
    configReads({ provider: 'versa_azure' });
    renderGuard();
    expect(await screen.findByText('Application')).toBeInTheDocument();
    expect(screen.queryByText('CATALOG')).toBeNull();
  });

  it('lets a nonblank pair resume link reach the resume loader without global setup', async () => {
    mocks.location.pathname = '/pair';
    mocks.location.search = '?resumeSessionId=20260923_1';
    renderGuard();
    expect(await screen.findByText('Application')).toBeInTheDocument();
    expect(screen.queryByText('CATALOG')).toBeNull();
  });

  it('keeps a blank resume query on the normal onboarding path', async () => {
    mocks.location.pathname = '/pair';
    mocks.location.search = '?resumeSessionId=   ';
    renderGuard();
    expect(await screen.findByText('CATALOG')).toBeInTheDocument();
    expect(screen.queryByText('Application')).toBeNull();
  });
});

describe('ProviderGuard — entering without a provider', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.location.pathname = '/';
    mocks.location.search = '';
    configReads({});
    mocks.upsert.mockResolvedValue(undefined);
    mocks.getProviders.mockResolvedValue([]);
  });

  it('records the skip and shows the application', async () => {
    renderGuard();
    fireEvent.click(await screen.findByTestId('onboarding-skip-header'));

    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith(ONBOARDING_SKIPPED_KEY, true, false)
    );
    expect(await screen.findByText('Application')).toBeInTheDocument();
  });

  /**
   * ⚠ The skip is a **config** key, not renderer state: a user who skipped setup
   * yesterday must not meet the wall again on the next launch. Rendering the
   * children from the persisted flag on a cold mount is what makes that true.
   */
  it('honours a skip recorded on an earlier launch', async () => {
    configReads({ skipped: true });
    renderGuard();
    expect(await screen.findByText('Application')).toBeInTheDocument();
    expect(screen.queryByText('CATALOG')).toBeNull();
  });

  /**
   * ⚠ **Choosing a provider retires the skip.** Left set, it would suppress the
   * first-run screen on a machine that later *lost* its provider — the one
   * situation the wall exists for, silently disabled by a flag the user set
   * months earlier for an unrelated reason.
   */
  it('clears the skip once a provider is configured', async () => {
    configReads({ provider: 'versa_azure', skipped: true });
    renderGuard();
    await waitFor(() =>
      expect(mocks.upsert).toHaveBeenCalledWith(ONBOARDING_SKIPPED_KEY, false, false)
    );
    expect(await screen.findByText('Application')).toBeInTheDocument();
  });

  /**
   * ⚠ A skip that could not be written must not be reported as one. Letting the
   * user through anyway would put them in an app whose first-run screen returns
   * on the next launch with no explanation.
   */
  it('keeps the wall up when the skip cannot be saved', async () => {
    mocks.upsert.mockRejectedValue(new Error('read-only config'));
    renderGuard();
    fireEvent.click(await screen.findByTestId('onboarding-skip-header'));
    await waitFor(() => expect(mocks.upsert).toHaveBeenCalled());
    expect(screen.getByText('CATALOG')).toBeInTheDocument();
    expect(screen.queryByText('Application')).toBeNull();
  });

  it('lets Crew open without persisting a global onboarding skip', async () => {
    mocks.location.pathname = '/crew';
    const view = renderGuard();

    expect(await screen.findByText('Application')).toBeInTheDocument();
    expect(mocks.upsert).not.toHaveBeenCalledWith(ONBOARDING_SKIPPED_KEY, true, false);

    mocks.location.pathname = '/';
    view.rerender(
      <ProviderGuard didSelectProvider={false}>
        <div>Application</div>
      </ProviderGuard>
    );

    expect(await screen.findByText('CATALOG')).toBeInTheDocument();
    expect(mocks.upsert).not.toHaveBeenCalledWith(ONBOARDING_SKIPPED_KEY, true, false);
  });
});

/**
 * The write sequence itself, where it lives. ⚠ The ORDER is the contract: the
 * endpoint config is written before `BIOROUTER_PROVIDER` so the saved provider
 * targets the same endpoint detection validated against.
 */
describe('persistDetectedProviderSetup', () => {
  it('writes the secret, then the endpoint config, then the selection', async () => {
    const upsert = vi.fn().mockResolvedValue(undefined);
    await persistDetectedProviderSetup(upsert, {
      provider: 'xiaomi_mimo',
      model: 'mimo-v2-flash',
      models: ['mimo-v2-flash'],
      apiKey: 'mimo-secret',
      apiKeyConfigKey: 'XIAOMI_MIMO_API_KEY',
      extraConfig: { XIAOMI_MIMO_HOST: 'https://api.xiaomimimo.com' },
    });
    expect(upsert.mock.calls).toEqual([
      ['XIAOMI_MIMO_API_KEY', 'mimo-secret', true],
      ['XIAOMI_MIMO_HOST', 'https://api.xiaomimimo.com', false],
      ['BIOROUTER_PROVIDER', 'xiaomi_mimo', false],
    ]);
  });
});
