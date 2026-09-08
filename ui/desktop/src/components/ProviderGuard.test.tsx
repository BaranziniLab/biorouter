import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProviderGuard, { ONBOARDING_SKIPPED_KEY } from './ProviderGuard';
import { persistDetectedProviderSetup } from './onboarding/CommercialSetupCard';

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
  }: {
    onCommercialSuccess?: (setup: unknown) => void | Promise<void>;
    onLocalComplete?: () => void;
  }) => (
    <div>
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
    configReads({});
    mocks.upsert.mockResolvedValue(undefined);
    mocks.getProviders.mockResolvedValue([]);
  });

  it('persists the detected provider contract before opening its model picker', async () => {
    renderGuard();

    // The brand mark renders in both the checking loader and the onboarding
    // header; assert it appears without re-checking document attachment, since
    // the loader's mark detaches the instant `isChecking` flips to the header.
    expect(await screen.findByRole('img', { name: 'BioRouter' })).toBeTruthy();
    fireEvent.click(await screen.findByRole('button', { name: 'Complete detection' }));

    await waitFor(() => {
      expect(mocks.upsert.mock.calls).toEqual([
        ['XIAOMI_MIMO_API_KEY', 'mimo-secret', true],
        ['XIAOMI_MIMO_HOST', 'https://api.xiaomimimo.com', false],
        ['BIOROUTER_PROVIDER', 'xiaomi_mimo', false],
      ]);
    });
  });

  it('blocks the application while neither a provider nor a skip is recorded', async () => {
    renderGuard();
    expect(await screen.findByText('CATALOG')).toBeInTheDocument();
    expect(screen.queryByText('Application')).toBeNull();
  });

  it('lets a configured install straight through', async () => {
    configReads({ provider: 'versa_azure' });
    renderGuard();
    expect(await screen.findByText('Application')).toBeInTheDocument();
    expect(screen.queryByText('CATALOG')).toBeNull();
  });
});

describe('ProviderGuard — entering without a provider', () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
