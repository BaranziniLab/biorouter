import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import BrowseExtensionsModal from './BrowseExtensionsModal';
import type { BrxtEnvVar, BrxtManifest } from '../../types/brxt';

/**
 * Issue #116. **These tests render the REAL `BrxtInstallModal`.**
 *
 * The suite this replaces stubbed it out, and that is precisely why the bug it
 * is written for shipped: a mocked child cannot show you that the marketplace
 * route was rendering "Drop your .brxt file here" underneath a fully populated
 * manifest card. The handoff assertions the old suite made are kept — they are
 * still the DR-23 mechanism — but they are now made against what the installer
 * actually did with the handoff rather than against the props it was handed.
 */

const loadRegistry = vi.hoisted(() => vi.fn());

// Spread the real module rather than listing members: a partial factory means
// every export this component newly reaches for (`effectivePrivacy`,
// `catalogFreshnessLine`) arrives `undefined` and the modal dies at render, in a
// test that has nothing to say about either. Only the two seams the test
// actually controls are replaced.
vi.mock('./registry', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./registry')>()),
  loadRegistry,
  extensionMatches: () => true,
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ addExtension: vi.fn() }),
  // `PrivacyBadge` reads the master switch off this same context and fails
  // loudly if the mock omits it — see the warning in `ui/PrivacyBadge.tsx`.
  usePrivacyTiersEnabled: () => true,
}));

const activateExtensionDefault = vi.hoisted(() => vi.fn(async () => {}));
vi.mock('../settings/extensions', () => ({ activateExtensionDefault }));

const upsertConfig = vi.hoisted(() => vi.fn(async () => ({ error: undefined })));
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  upsertConfig,
}));

vi.mock('../../utils/userAction', () => ({ userActionHeaders: async () => ({}) }));

vi.mock('../../toasts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../toasts')>()),
  toastService: { success: vi.fn(), error: vi.fn() },
}));

const envVar = (over: Partial<BrxtEnvVar> & { key: string }): BrxtEnvVar => ({
  required: false,
  auto_propagate: false,
  description: `${over.key} description`,
  secret: false,
  ...over,
});

const manifestWith = (env_vars: BrxtEnvVar[]): BrxtManifest => ({
  name: 'playwrightagent',
  display_name: 'Playwright Agent',
  description: 'Browser automation via Playwright.',
  version: '0.1.0',
  entry_point: 'main.py',
  repository: 'https://github.com/BaranziniLab/PlaywrightAgent',
  tools_count: 23,
  env_vars,
});

const REGISTRY_ENTRY = {
  id: 'playwright-agent',
  name: 'Playwright Agent',
  organization: 'Biorouter',
  version: '0.1.0',
  description: 'Browser automation',
  tags: [],
  download: 'https://example.test/playwright.brxt',
};

/** The single knob every test turns: what the bundle's manifest declares. */
function mockElectron(
  options: {
    validate?: () => Promise<unknown>;
    download?: () => Promise<unknown>;
    install?: () => Promise<unknown>;
  } = {}
) {
  const download = options.download ?? (async () => ({ path: '/tmp/playwright.brxt' }));
  const validate =
    options.validate ?? (async () => ({ manifest: manifestWith([]), skillsPreview: [] }));
  const install = options.install ?? (async () => ({ installDir: '/ext/playwrightagent' }));
  const electron = {
    downloadRegistryAsset: vi.fn(download),
    validateBrxtBundle: vi.fn(validate),
    installBrxtBundle: vi.fn(install),
    getPathForFile: vi.fn(() => '/tmp/dropped.brxt'),
    openBrxtFilePicker: vi.fn(async () => null),
  };
  (window as unknown as { electron: unknown }).electron = electron;
  return electron;
}

function renderModal(over: Partial<Parameters<typeof BrowseExtensionsModal>[0]> = {}) {
  const props = {
    onClose: vi.fn(),
    onInstalled: vi.fn(),
    installedNames: new Set<string>(),
    ...over,
  };
  render(<BrowseExtensionsModal {...props} />);
  return props;
}

/** Click Add and wait for the installer's first step to settle. */
async function addAndWait(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: 'Add' }));
}

const LOCAL_FILE_CONTROLS = [
  /Drop your \.brxt file here/i,
  /Drop a different \.brxt file here/i,
  /or click to browse/i,
];

function expectNoLocalFileControls() {
  for (const pattern of LOCAL_FILE_CONTROLS) {
    expect(screen.queryByText(pattern)).toBeNull();
  }
  expect(screen.queryByRole('button', { name: /Browse file/i })).toBeNull();
}

afterEach(() => {
  cleanup();
  delete (window as unknown as { electron?: unknown }).electron;
  vi.clearAllMocks();
});

beforeEach(() => {
  loadRegistry.mockResolvedValue({
    live: true,
    registry: { extensions: [REGISTRY_ENTRY], skills: [] },
  });
  mockElectron();
});

describe('BrowseExtensionsModal — marketplace install (issue #116)', () => {
  it('never shows local-file controls once a marketplace extension is chosen', async () => {
    const user = userEvent.setup();
    renderModal();

    await addAndWait(user);

    // The header names the extension the user picked, not the generic
    // "Add extension" the file route uses.
    expect(await screen.findByText('Install Playwright Agent')).toBeInTheDocument();
    await screen.findByText('From the Biorouter marketplace');
    expectNoLocalFileControls();
  });

  it('shows a downloading state before the bundle arrives, and no file controls in it', async () => {
    const user = userEvent.setup();
    let release: (value: { path: string }) => void = () => {};
    mockElectron({
      download: () =>
        new Promise<{ path: string }>((resolve) => {
          release = resolve;
        }),
    });
    renderModal();

    await addAndWait(user);

    expect(await screen.findByText(/Downloading Playwright Agent…/i)).toBeInTheDocument();
    expectNoLocalFileControls();

    release({ path: '/tmp/playwright.brxt' });
    await screen.findByRole('button', { name: 'Install extension' });
  });

  it('offers "Install extension" — not "Next: configure" — when the manifest declares no variables', async () => {
    const user = userEvent.setup();
    const electron = mockElectron();
    const props = renderModal();

    await addAndWait(user);

    const install = await screen.findByRole('button', { name: 'Install extension' });
    expect(screen.queryByRole('button', { name: /Next: configure/i })).toBeNull();
    expect(await screen.findByText(/0 required env vars/i)).toBeInTheDocument();

    await user.click(install);

    await waitFor(() => expect(electron.installBrxtBundle).toHaveBeenCalled());
    // Issue #56 Task 43 (DR-23): the registry id and download URL still reach
    // the install — now asserted where they are actually consumed.
    expect(electron.installBrxtBundle).toHaveBeenCalledWith(
      '/tmp/playwright.brxt',
      'playwrightagent',
      {
        registryId: 'playwright-agent',
        sourceUrl: 'https://example.test/playwright.brxt',
      }
    );
    await waitFor(() => expect(props.onInstalled).toHaveBeenCalled());
    expect(props.onClose).toHaveBeenCalled();
  });

  it('routes a manifest with required variables to a configuration form that gates Install', async () => {
    const user = userEvent.setup();
    const electron = mockElectron({
      validate: async () => ({
        manifest: manifestWith([envVar({ key: 'PLAYWRIGHT_TOKEN', required: true, secret: true })]),
        skillsPreview: [],
      }),
    });
    renderModal();

    await user.click(await screen.findByRole('button', { name: 'Add' }));
    await user.click(await screen.findByRole('button', { name: 'Next: configure' }));

    expect(await screen.findByText('Configure Playwright Agent')).toBeInTheDocument();
    const field = screen.getByLabelText(/PLAYWRIGHT_TOKEN/);
    // A secret is masked in the form it is typed into.
    expect(field).toHaveAttribute('type', 'password');
    expect(screen.getByRole('button', { name: 'Install extension' })).toBeDisabled();

    await user.type(field, 'a-token');
    expect(screen.getByRole('button', { name: 'Install extension' })).toBeEnabled();

    await user.click(screen.getByRole('button', { name: 'Install extension' }));
    await waitFor(() => expect(electron.installBrxtBundle).toHaveBeenCalled());
  });

  it('names optional-only configuration as optional and shows the fields without a disclosure', async () => {
    const user = userEvent.setup();
    mockElectron({
      validate: async () => ({
        manifest: manifestWith([envVar({ key: 'PLAYWRIGHT_HEADLESS', default: 'true' })]),
        skillsPreview: [],
      }),
    });
    renderModal();

    await user.click(await screen.findByRole('button', { name: 'Add' }));

    // Not "Next: configure": nothing here is required.
    const next = await screen.findByRole('button', { name: 'Next: optional settings' });
    await user.click(next);

    expect(await screen.findByLabelText('PLAYWRIGHT_HEADLESS')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Show 1 optional variable/i })).toBeNull();
    // Nothing is required, so Install is reachable immediately.
    expect(screen.getByRole('button', { name: 'Install extension' })).toBeEnabled();
  });

  it('offers Retry and Back to marketplace when validation fails, never a drop zone', async () => {
    const user = userEvent.setup();
    const electron = mockElectron({
      validate: vi
        .fn()
        .mockResolvedValueOnce({ error: 'manifest.json is not valid JSON' })
        .mockResolvedValue({ manifest: manifestWith([]), skillsPreview: [] }),
    });
    renderModal();

    await addAndWait(user);

    expect(await screen.findByText('manifest.json is not valid JSON')).toBeInTheDocument();
    expectNoLocalFileControls();
    expect(screen.getByRole('button', { name: 'Back to marketplace' })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await screen.findByRole('button', { name: 'Install extension' });
    expect(electron.validateBrxtBundle).toHaveBeenCalledTimes(2);
  });

  it('offers Retry when the download fails, and re-downloads on Retry', async () => {
    const user = userEvent.setup();
    const electron = mockElectron({
      download: vi
        .fn()
        .mockResolvedValueOnce({ error: 'Network unreachable' })
        .mockResolvedValue({ path: '/tmp/playwright.brxt' }),
    });
    renderModal();

    await addAndWait(user);

    expect(await screen.findByText('Network unreachable')).toBeInTheDocument();
    expectNoLocalFileControls();

    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await screen.findByRole('button', { name: 'Install extension' });
    expect(electron.downloadRegistryAsset).toHaveBeenCalledTimes(2);
  });

  it('returns to the marketplace list on Back, without installing and without closing it', async () => {
    const user = userEvent.setup();
    const electron = mockElectron();
    const props = renderModal();

    await addAndWait(user);
    await screen.findByRole('button', { name: 'Install extension' });

    await user.click(screen.getByRole('button', { name: 'Back to marketplace' }));

    await waitFor(() => expect(screen.getByText('Browse extensions')).toBeInTheDocument());
    expect(electron.installBrxtBundle).not.toHaveBeenCalled();
    expect(props.onClose).not.toHaveBeenCalled();
    expect(props.onInstalled).not.toHaveBeenCalled();
  });

  it('returns to the marketplace list on Escape, not to a file-drop step', async () => {
    const user = userEvent.setup();
    const props = renderModal();

    await addAndWait(user);
    await screen.findByRole('button', { name: 'Install extension' });

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.getByText('Browse extensions')).toBeInTheDocument());
    expect(props.onClose).not.toHaveBeenCalled();
  });
});

describe('BrowseExtensionsModal — installed rows (issue #116)', () => {
  it('offers Configure on an installed row and hands back the registry entry', async () => {
    const user = userEvent.setup();
    const onConfigureInstalled = vi.fn();
    renderModal({
      installedNames: new Set(['playwright agent']),
      onConfigureInstalled,
    });

    const row = (await screen.findByText('Playwright Agent')).closest('div.biorouter-modal-row');
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getByText('Installed')).toBeInTheDocument();

    await user.click(within(row as HTMLElement).getByRole('button', { name: 'Configure' }));
    expect(onConfigureInstalled).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'playwright-agent' })
    );
  });

  it('keeps the inert badge when the surface has nowhere to send the user', async () => {
    renderModal({ installedNames: new Set(['playwright agent']) });

    await screen.findByText('Installed');
    expect(screen.queryByRole('button', { name: 'Configure' })).toBeNull();
  });
});
