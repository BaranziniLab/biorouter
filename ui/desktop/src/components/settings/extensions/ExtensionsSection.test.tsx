import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  extensionsList: [] as Array<Record<string, unknown>>,
  addExtension: vi.fn(async () => undefined),
  removeExtension: vi.fn(async () => undefined),
  getExtensions: vi.fn(async () => []),
  toggleExtensionDefault: vi.fn(async () => undefined),
  activateExtensionDefault: vi.fn(async () => undefined),
  deleteExtension: vi.fn(async () => undefined),
  success: vi.fn(),
  error: vi.fn(),
}));

// `useConfig` THROWS outside a ConfigProvider (`ConfigContext.tsx`, the
// `throw new Error('useConfig must be used within a ConfigProvider')`) and the
// context object itself is module-private (`const ConfigContext =
// createContext<ConfigContextType | undefined>(undefined);`, no `export`), so it
// cannot be wrapped — the component must be given a mocked hook. Same shape as
// the sibling `capabilities/CapabilitiesSection.test.tsx`.
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: mocks.extensionsList,
    addExtension: mocks.addExtension,
    removeExtension: mocks.removeExtension,
    getExtensions: mocks.getExtensions,
    // The default provider is read on mount; no provider here, so the cards judge nothing.
    read: vi.fn(async () => null),
    getProviders: vi.fn(async () => []),
  }),
  // Every card now carries a `PrivacyBadge` (issue #56 §13.5), and the badge
  // reads the master switch off this same context. It throws rather than
  // defaulting when a mock omits this — see the warning in `ui/PrivacyBadge.tsx`,
  // which is the deliberate trade against a prop nine call sites would have to
  // remember to pass.
  usePrivacyTiersEnabled: () => true,
}));

// `./index` re-exports the real extension-manager calls, which would hit the daemon.
vi.mock('./index', () => ({
  toggleExtensionDefault: mocks.toggleExtensionDefault,
  activateExtensionDefault: mocks.activateExtensionDefault,
  deleteExtension: mocks.deleteExtension,
}));

vi.mock('../../../toasts', () => ({
  toastService: { success: mocks.success, error: mocks.error },
}));

// DEFAULT export — `ExtensionsSection.tsx` is `export default function
// ExtensionsSection(...)`; there is no named export, so `import { ExtensionsSection }`
// is `undefined` and React throws "type is invalid" at render.
import ExtensionsSection from './ExtensionsSection';

// `ExtensionItem` names its Radix switch by the row's title (`aria-labelledby`), and
// `getFriendlyTitle` for a `platform` entry named `Workspace` is
// `formatExtensionName('Workspace')` = 'Workspace' (it is not in
// PLATFORM_EXTENSION_DISPLAY_NAMES).

/**
 * ⚠ These fixtures use the names the DAEMON sends, not the config keys.
 * `PlatformExtensionDef.name` is the extension's `EXTENSION_NAME` constant —
 * `"Workspace"` and `"Chat Recall"` (`crates/biorouter/src/agents/extension.rs`
 * + `workspace_extension.rs:44` / `chatrecall_extension.rs:14`) — which is what
 * `useConfig().extensionsList` carries. Verified off the live React props in
 * the dev GUI during Task 31's pass.
 *
 * They used to read `'workspace'` / `'chatrecall'`, and that is exactly why the
 * whole of decision 14 shipped inert: this suite was green while a real click
 * on the real switch suggested nothing, because neither the `name === 'workspace'`
 * arm nor the `find(e => e.name === 'chatrecall')` lookup could ever match.
 */
const workspaceEntry = (enabled: boolean) => ({
  type: 'platform',
  name: 'Workspace',
  description: 'Workspace Control',
  enabled,
});
const chatrecallEntry = (enabled: boolean) => ({
  type: 'platform',
  name: 'Chat Recall',
  description: 'Recall chats',
  enabled,
});

/**
 * ⚠ **The chatrecall-suggestion suite was retired here, not weakened** (#76).
 *
 * Decision 14's prompt fired when the user turned Workspace Control ON **from
 * the Extensions tab**. Workspace is now a built-in capability: it is filtered
 * out of this surface entirely (`ExtensionsSection.tsx` drops anything
 * `isCapabilityExtension`), and it ships enabled. So the trigger this suite
 * drove — `findByRole('switch', { name: 'Toggle Workspace extension' })` —
 * cannot exist here, and every test in it was asserting against a control that
 * no longer renders.
 *
 * What replaces it is the assertion that matters now: Workspace must NOT appear
 * on this screen. That is the actual requirement, and it is the one that would
 * regress if someone removed the capability key.
 *
 * ⚠ The suggestion itself still needs a home. Its toast lives in
 * `ExtensionsSection.tsx` on a branch that is now unreachable, and re-homing it
 * on the Capabilities toggle is tracked on #76 — it is a product question
 * (a default-on capability arguably has nothing to suggest at enable time)
 * rather than a mechanical move, so it is not being decided by a test edit.
 */
describe('ExtensionsSection — Workspace is a capability, not an extension', () => {
  it('does not render Workspace among the toggleable extensions', async () => {
    mocks.extensionsList = [workspaceEntry(true), chatrecallEntry(false)];
    render(<ExtensionsSection />);

    // The switch is named by the row's title now, so the query names the row. (`/Toggle
    // Workspace/` would pass vacuously: nothing is called that any more.)
    await waitFor(() =>
      expect(screen.queryByRole('switch', { name: /Workspace/i })).not.toBeInTheDocument()
    );
    expect(screen.queryByText('Workspace')).not.toBeInTheDocument();
  });
});

const customEntry = {
  type: 'stdio',
  name: 'medcp',
  description: 'Local clinical MCP',
  cmd: 'node',
  args: ['server.js'],
  enabled: true,
};

describe('ExtensionsSection — the groups and the row menu', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.extensionsList = [customEntry];
  });

  it('labels the groups in the caps style with a count and no dot', async () => {
    mocks.extensionsList = [customEntry, { ...customEntry, name: 'zzz', enabled: false }];
    render(<ExtensionsSection />);

    const enabled = await screen.findByRole('heading', { level: 2, name: /^Enabled 1$/ });
    expect(enabled).toHaveClass('text-caps');
    expect(screen.getByRole('heading', { level: 2, name: /^Available 1$/ })).toBeInTheDocument();
    // The status dots were a second way of saying what the switch already says.
    expect(document.querySelector('h2 > span.rounded-full')).toBeNull();
    // And it is not the old title-cased wording.
    expect(screen.queryByText(/Default Extensions/)).toBeNull();
  });

  it('says what a filter found nothing for, rather than leaving the page blank', async () => {
    render(<ExtensionsSection searchTerm="no-such-extension" />);

    expect(await screen.findByText('No matching extensions')).toBeInTheDocument();
    expect(screen.queryByText('No extensions yet')).toBeNull();
  });

  it('opens the configure dialog from Configure… and the removal question from Remove…', async () => {
    const user = userEvent.setup({ delay: null });
    render(<ExtensionsSection />);

    await user.click(await screen.findByRole('button', { name: 'More actions' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Configure…' }));
    expect(await screen.findByRole('dialog', { name: 'Update extension' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    await user.click(screen.getByRole('button', { name: 'More actions' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Remove…' }));
    // The dialog starts on the question, not on the form the person did not ask for.
    expect(await screen.findByRole('dialog', { name: 'Remove "medcp"?' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Confirm removal' })).toBeInTheDocument();
    // Cancel closes it, since there was no form to go back to.
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });
});
