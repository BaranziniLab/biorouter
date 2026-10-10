import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BrxtManifest } from '../types/brxt';

/**
 * `BrxtInstallModal` reaches for the live config on mount; nothing here installs
 * anything, so the context is stubbed down to the one member the component
 * closes over.
 */
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ addExtension: vi.fn() }),
  // `PrivacyBadge` reads the master switch off this same context and fails loudly
  // if the mock omits it — see the warning in `ui/PrivacyBadge.tsx`.
  usePrivacyTiersEnabled: () => true,
}));

import { BrxtInstallModal } from './BrxtInstallModal';
import { BRXT_INSTALL_COPY as COPY } from './brxtInstallCopy';

const MANIFEST: BrxtManifest = {
  name: 'playwrightagent',
  display_name: 'Playwright Agent',
  description: 'Browser automation via Playwright.',
  version: '0.1.0',
  entry_point: 'main.py',
  repository: 'https://github.com/BaranziniLab/PlaywrightAgent',
  tools_count: 23,
  env_vars: [],
};

afterEach(() => {
  cleanup();
  delete (window as unknown as { electron?: unknown }).electron;
});

describe('BrxtInstallModal — issue #56 §13.5', () => {
  it('the brxt install modal says the resulting badge out loud', () => {
    // The plan wrote `render(<BrxtInstallModal manifest={{ name: 'anything' }} />)`.
    // There is no `manifest` prop — the component derives the manifest from the
    // dropped file — so the real props are passed instead. The assertions are
    // the plan's, unchanged: the disclosure has to be on screen BEFORE a file is
    // chosen, because "always Public" is a property of the install route, not of
    // whatever bundle the user is about to pick.
    render(<BrxtInstallModal onClose={() => {}} onInstalled={() => {}} />);
    expect(screen.getByText(/always Public/i)).toBeInTheDocument();
    expect(
      screen.getByText(/including commercial models hosted outside your institution/i)
    ).toBeInTheDocument();
  });

  /**
   * ⚠ **This modal is not only the file-drop route.** `BrowseExtensionsModal`
   * downloads a marketplace `.brxt` and then renders THIS component — so the
   * browse row can show a Private badge, the install confirmation say "always
   * Public", and Settings badge it Private afterwards. Three screens, two
   * answers, one install.
   *
   * The task's own Step 3 names the other half of the same contradiction: a
   * hand-installed extension NAMED `ucsfomopagent` inherits the private badge,
   * "fail-closed, and fine" — which is only fine if the modal does not promise
   * the opposite first.
   *
   * The disclosure is therefore about the RESULT and not about the route, and
   * once a manifest is in hand the result is knowable: `classifyExtension` is
   * the same union every other privacy surface in the app reads.
   */
  it('says Private when that is the badge the install will actually produce', async () => {
    (window as unknown as { electron: unknown }).electron = {
      validateBrxtBundle: vi.fn(async () => ({
        manifest: {
          name: 'ucsfomopagent',
          display_name: 'UCSF OMOP',
          description: 'OMOP clinical database',
          version: '1.0.0',
          entry_point: 'main.py',
          repository: 'https://github.com/BaranziniLab/UCSFOMOPAgent',
          env_vars: [],
        },
        skillsPreview: [],
      })),
    };

    render(
      <BrxtInstallModal
        onClose={() => {}}
        onInstalled={() => {}}
        preloadedFilePath="/tmp/ucsfomopagent.brxt"
      />
    );

    expect(await screen.findByText(/only private models/i)).toBeInTheDocument();
    expect(screen.queryByText(/always Public/i)).toBeNull();
  });

  /**
   * Issue #116. The marketplace row's badge is the only tier available while
   * the bundle is still downloading, and the installer opens on top of that
   * row — so it must not answer the question differently from the screen the
   * user just clicked.
   */
  it('carries the marketplace row badge through the download, before any manifest exists', () => {
    render(
      <BrxtInstallModal
        onClose={() => {}}
        onInstalled={() => {}}
        origin={{
          kind: 'marketplace',
          registrySource: { registryId: 'ucsfomopagent-1.0.0' },
          entry: { name: 'UCSF OMOP', privacyTier: 'private' },
          downloading: true,
          onRetry: () => {},
        }}
      />
    );

    expect(screen.getByText(/only private models/i)).toBeInTheDocument();
    expect(screen.queryByText(/always Public/i)).toBeNull();
  });
});

/**
 * Issue #116. The local route is the half that MUST keep its file controls, and
 * these assertions are the reason the marketplace suite's absence-checks are
 * worth anything: the same strings are found here and refused there.
 */
describe('BrxtInstallModal — the local-file route (issue #116)', () => {
  it('keeps drag/drop and the file picker when no origin is given', () => {
    render(<BrxtInstallModal onClose={() => {}} onInstalled={() => {}} />);

    expect(screen.getByText(COPY.addTitle)).toBeInTheDocument();
    // One line, and the zone IS the control: no second "or click to browse" line, no extra
    // "Browse file…" button beside it (Add skill's zone, Knowledge's recipe).
    expect(screen.getByText(COPY.dropZone)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: COPY.dropZoneName })).toBeInTheDocument();
    expect(COPY.dropZoneName).toContain(COPY.dropZone);
    expect(screen.queryByText('or click to browse')).toBeNull();
    expect(screen.queryByRole('button', { name: /Browse file/ })).toBeNull();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Back to marketplace' })).toBeNull();
  });

  /**
   * ⚠ A preloaded path is NOT a marketplace install. Finder hands a
   * double-clicked `.brxt` to the app over IPC and `ExtensionsView` preloads
   * it, so deriving the mode from the path — the obvious shortcut — would strip
   * the file controls off the one route that needs them and leave the user with
   * no way to correct a mis-picked file.
   */
  it('stays on the local route for an IPC-preloaded file, offering to replace it', async () => {
    (window as unknown as { electron: unknown }).electron = {
      validateBrxtBundle: vi.fn(async () => ({ manifest: MANIFEST, skillsPreview: [] })),
    };

    render(
      <BrxtInstallModal
        onClose={() => {}}
        onInstalled={() => {}}
        preloadedFilePath="/tmp/playwright.brxt"
      />
    );

    expect(await screen.findByText(COPY.detectedFromBundle)).toBeInTheDocument();
    expect(screen.getByText(COPY.addTitle)).toBeInTheDocument();
    expect(screen.getByText(COPY.dropZoneAnother)).toBeInTheDocument();
    // The button's name contains the line on screen, whichever line that is (label in name).
    const zone = screen.getByRole('button', { name: COPY.dropZoneAnotherName });
    expect(zone).toHaveAccessibleName(expect.stringContaining(COPY.dropZoneAnother));
  });

  it('labels the local primary action by what it will actually do', async () => {
    const install = vi.fn(async () => ({ installDir: '/ext/playwrightagent' }));
    (window as unknown as { electron: unknown }).electron = {
      validateBrxtBundle: vi.fn(async () => ({ manifest: MANIFEST, skillsPreview: [] })),
      installBrxtBundle: install,
    };

    render(
      <BrxtInstallModal
        onClose={() => {}}
        onInstalled={() => {}}
        preloadedFilePath="/tmp/playwright.brxt"
      />
    );

    // Zero env vars: the button installs, so it says so.
    const button = await screen.findByRole('button', { name: 'Install extension' });
    expect(screen.queryByRole('button', { name: /Next: configure/i })).toBeNull();

    await userEvent.setup().click(button);
    await waitFor(() => expect(install).toHaveBeenCalled());
    // The local route records no registry provenance (DR-23): there is none.
    expect(install).toHaveBeenCalledWith('/tmp/playwright.brxt', 'playwrightagent', undefined);
  });
});

/**
 * The dialog's chrome and its flat sections (spec 3.11): the dialog is the only box, the drop
 * zone is one line and a button, the type roles are the app's, and the tier line (section 13.5)
 * stays on screen as text.
 */
describe('BrxtInstallModal — the dialog', () => {
  const ENV = (overrides: Record<string, unknown>) => ({
    key: 'TOKEN',
    required: true,
    auto_propagate: false,
    description: 'Your API token',
    secret: true,
    ...overrides,
  });

  const withManifest = (env: unknown[]) => {
    (window as unknown as { electron: unknown }).electron = {
      validateBrxtBundle: vi.fn(async () => ({
        manifest: { ...MANIFEST, env_vars: env },
        skillsPreview: [
          { slug: 'pw-skill', name: 'pw-skill', description: 'Drives the browser' },
          { slug: 'pw-two', name: 'pw-two', description: 'Another' },
        ],
      })),
    };
  };

  const open = (props: Partial<React.ComponentProps<typeof BrxtInstallModal>> = {}) =>
    render(
      <BrxtInstallModal
        onClose={() => {}}
        onInstalled={() => {}}
        preloadedFilePath="/tmp/playwright.brxt"
        {...props}
      />
    );

  it('is a 480px dialog anchored at the top, with Cancel quiet and the tier line in the page', async () => {
    withManifest([]);
    open();
    const dialog = await screen.findByRole('dialog', { name: COPY.addTitle });

    expect(dialog).toHaveAttribute('data-size', 'md');
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    expect(screen.getByRole('button', { name: 'Cancel' }).className).toMatch(
      /bg-background-medium/
    );
    // §13.5, as text above the Install button, not in a panel and not in help.
    expect(screen.getByText(COPY.tierPublicFile)).toBeVisible();
    expect(dialog.querySelector('.biorouter-modal-panel')).toBeNull();
  });

  it('summarises the bundle flat: facts, the description, and the skills it carries', async () => {
    withManifest([ENV({})]);
    open();

    expect(await screen.findByText(COPY.detectedFromBundle)).toBeVisible();
    const facts = screen.getByText(/^v0\.1\.0/);
    expect(facts).toHaveTextContent('v0.1.0 · 23 tools · 2 skills · 1 required env var');
    expect(screen.getByText('Browser automation via Playwright.')).toBeVisible();
    expect(screen.getByText(COPY.skillsIncluded)).toBeVisible();
    expect(screen.getByText('pw-skill')).toBeVisible();
  });

  it('opens the native picker from the drop zone and reads the file it returns', async () => {
    const user = userEvent.setup({ delay: null });
    const validate = vi.fn(async () => ({ manifest: MANIFEST, skillsPreview: [] }));
    const picker = vi.fn(async () => '/tmp/picked.brxt');
    (window as unknown as { electron: unknown }).electron = {
      validateBrxtBundle: validate,
      openBrxtFilePicker: picker,
    };
    render(<BrxtInstallModal onClose={() => {}} onInstalled={() => {}} />);

    await user.click(screen.getByRole('button', { name: COPY.dropZoneName }));

    expect(picker).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(validate).toHaveBeenCalledWith('/tmp/picked.brxt'));
    expect(await screen.findByText(COPY.detectedFromBundle)).toBeInTheDocument();
  });

  it('keeps the drop zone focusable while it reads a file, so first focus never lands on the dialog box', async () => {
    // Finder hands over a file and the dialog opens already reading it. A disabled drop zone
    // cannot hold focus, which left first focus on the dialog's own box, drawn with the
    // browser's default blue ring.
    const user = userEvent.setup({ delay: null });
    const validate = vi.fn(() => new Promise<never>(() => {}));
    const picker = vi.fn(async () => '/tmp/other.brxt');
    (window as unknown as { electron: unknown }).electron = {
      validateBrxtBundle: validate,
      openBrxtFilePicker: picker,
    };
    render(
      <BrxtInstallModal
        onClose={() => {}}
        onInstalled={() => {}}
        preloadedFilePath="/tmp/playwright.brxt"
      />
    );

    const zone = await screen.findByRole('button', { name: COPY.reading });
    expect(zone).not.toBeDisabled();
    expect(zone).toHaveAttribute('aria-disabled', 'true');
    await waitFor(() => expect(zone).toHaveFocus());
    expect(screen.getByRole('dialog')).not.toHaveFocus();

    // It stays on screen without taking a second file while the first is being read.
    await user.click(zone);
    expect(picker).not.toHaveBeenCalled();
    expect(validate).toHaveBeenCalledTimes(1);
  });

  it('labels a required variable with a mark that is not spoken, and masks a secret', async () => {
    const user = userEvent.setup({ delay: null });
    withManifest([ENV({ key: 'TEST_KEY' })]);
    open();

    await user.click(await screen.findByRole('button', { name: 'Next: configure' }));

    const field = await screen.findByLabelText(/TEST_KEY/);
    expect(field).toHaveAttribute('type', 'password');
    expect(field).toHaveAttribute('aria-required', 'true');
    // The hint is the manifest's own words, in the empty field.
    expect(field).toHaveAttribute('placeholder', 'Your API token');
    const label = document.querySelector('label[for="brxt-env-TEST_KEY"]') as HTMLElement;
    expect(label).toHaveTextContent('TEST_KEY *');
    expect(label.querySelector('[aria-hidden="true"]')).toHaveTextContent('*');
  });

  it('says which value is missing when Install is pressed, instead of sitting grey', async () => {
    const user = userEvent.setup({ delay: null });
    const install = vi.fn(async () => ({ installDir: '/tmp/x' }));
    withManifest([ENV({ key: 'TEST_KEY' })]);
    (window.electron as unknown as { installBrxtBundle: unknown }).installBrxtBundle = install;
    open();
    await user.click(await screen.findByRole('button', { name: 'Next: configure' }));

    const field = await screen.findByLabelText(/TEST_KEY/);
    const installButton = screen.getByRole('button', { name: COPY.installExtension });
    expect(installButton).toBeEnabled();
    expect(screen.queryByText(COPY.valueRequired)).toBeNull();

    await user.click(installButton);

    expect(await screen.findByText(COPY.valueRequired)).toBeVisible();
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(field).toHaveAccessibleDescription(COPY.valueRequired);
    await waitFor(() => expect(field).toHaveFocus());
    expect(install).not.toHaveBeenCalled();

    // The message follows what is typed.
    await user.type(field, 'abc');
    expect(screen.queryByText(COPY.valueRequired)).toBeNull();
  });

  it('keeps optional variables behind a disclosure when some are required', async () => {
    const user = userEvent.setup({ delay: null });
    withManifest([ENV({}), ENV({ key: 'OPTIONAL_KEY', required: false, secret: false })]);
    open();
    await user.click(await screen.findByRole('button', { name: 'Next: configure' }));

    expect(screen.queryByLabelText('OPTIONAL_KEY')).toBeNull();
    const toggle = screen.getByRole('button', { name: COPY.optionalVariables(1) });
    expect(toggle).toHaveAccessibleDescription(COPY.prefilled);
    await user.click(toggle);
    expect(await screen.findByLabelText('OPTIONAL_KEY')).toBeInTheDocument();
  });

  it('shows optional-only settings open, with the reassurance as help on their title', async () => {
    const user = userEvent.setup({ delay: null });
    withManifest([ENV({ key: 'HEADLESS', required: false, secret: false, default: 'true' })]);
    open();
    await user.click(await screen.findByRole('button', { name: 'Next: optional settings' }));

    expect(await screen.findByLabelText('HEADLESS')).toBeVisible();
    expect(screen.getByRole('heading', { name: COPY.optionalSettings })).toBeVisible();
    const help = screen.getByRole('button', { name: `About ${COPY.optionalSettings}` });
    expect(help).toHaveAccessibleDescription(COPY.optionalHelp('Playwright Agent', 1));
    // Nothing is required, so Install is reachable at once, and no paragraph carries the help.
    expect(screen.getByRole('button', { name: COPY.installExtension })).toBeEnabled();
    expect(screen.queryByText(/pre-filled with defaults — install/)).toBeNull();
  });

  it('names the publisher and version under the title, and says "from the marketplace" once', async () => {
    withManifest([]);
    open({
      preloadedFilePath: '/tmp/cdw.brxt',
      origin: {
        kind: 'marketplace',
        registrySource: { registryId: 'cdwagent' },
        entry: { name: 'CDWAgent', organization: 'BaranziniLab · UCSF', privacyTier: 'private' },
        downloading: false,
        onRetry: () => {},
      },
    });

    const dialog = await screen.findByRole('dialog', {
      name: COPY.installTitle('Playwright Agent'),
    });
    expect(dialog).toHaveAccessibleDescription('BaranziniLab · UCSF · v0.1.0');
    // The summary's first line is the one statement of where it came from.
    expect(await screen.findAllByText(COPY.fromMarketplace)).toHaveLength(1);
  });

  it('says what it is waiting for in one status line with the shared spinner', () => {
    render(
      <BrxtInstallModal
        onClose={() => {}}
        onInstalled={() => {}}
        origin={{
          kind: 'marketplace',
          registrySource: { registryId: 'playwright-agent' },
          entry: { name: 'Playwright Agent' },
          downloading: true,
          onRetry: () => {},
        }}
      />
    );

    const status = screen.getByRole('status');
    expect(status).toHaveTextContent(COPY.downloading('Playwright Agent'));
    expect(status.querySelector('.br-spinner')).not.toBeNull();
    expect(document.querySelector('.animate-pulse')).toBeNull();
  });

  it('has no dash in a sentence it says', () => {
    const sentences = [
      COPY.tierPrivate,
      COPY.tierPublicMarketplace,
      COPY.tierPublicFile,
      COPY.noLocalPath,
      COPY.registerFailed,
      COPY.optionalHelp('X', 3),
    ];
    for (const text of sentences) expect(text).not.toMatch(/[\u2013\u2014]/);
  });
});
