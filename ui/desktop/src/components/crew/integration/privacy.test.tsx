import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewConnection } from '../crewApi';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { installResizeObserverStub, workspaceAction } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  installDaemon,
  mocked,
  renderCrew,
  richSnapshot,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/** The workspace's own name: the phrase a typed confirmation asks for, and the switcher's name. */
const WORKSPACE = 'lab';

/** Every PATCH of the saved connection, by body. */
function patches(): unknown[] {
  return mocked.crewHttp.mock.calls
    .filter(([path, method]) => path === `/connections/${connection.id}` && method === 'PATCH')
    .map(([, , body]) => body);
}

function policySets(): unknown[] {
  return mocked.crewRequest.mock.calls
    .filter(([, method]) => method === 'policy.set')
    .map(([, , params]) => params);
}

/** A daemon that saves a PATCH and answers with the saved record, as the real one does. */
function savingDaemon(initial: Parameters<typeof installDaemon>[0] = {}): ScriptedDaemon {
  const daemon = installDaemon(initial);
  daemon.state.http = (path, method, body) =>
    path === `/connections/${connection.id}` && method === 'PATCH'
      ? { ...connection, ...(body as object) }
      : undefined;
  return daemon;
}

/** The typed confirmation for making the connection Public: an `alertdialog`, as every one is. */
async function makePublicConfirmation() {
  return screen.findByRole('alertdialog', { name: `Make your ${WORKSPACE} connection public?` });
}

/** Neither a confirmation nor the popover (a Radix `dialog`) is still open. */
function expectNothingOpen() {
  expect(screen.queryByRole('alertdialog')).toBeNull();
  expect(screen.queryByRole('dialog')).toBeNull();
}

/** The full record the daemon keeps (L18), with only the mode changed. */
function fullBody(mode: 'private' | 'public', from: CrewConnection = connection) {
  return { ...connectionUpdateBody(from), mode };
}

describe('privacy changes: exposing asks first, the reverse is one click', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('offers no downgrade from the status-row popover where the workspace is Private for everyone (Q3-54)', async () => {
    savingDaemon();
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    // Its own description said the models that can read the workspace "stay the same": a control
    // that changed nothing, offered first. Privacy… is the one action, and leads to the settings.
    await user.click(screen.getByRole('button', { name: /^Privacy: Private · ucsf/ }));
    const popover = await screen.findByRole('dialog', { name: /^Privacy: Private/ });
    expect(
      within(popover).queryByRole('button', { name: 'Make my connection public…' })
    ).toBeNull();
    expect(
      within(popover)
        .getAllByRole('button')
        .map((button) => button.textContent)
    ).toEqual(['Privacy…']);
    await user.click(within(popover).getByRole('button', { name: 'Privacy…' }));
    const settings = await screen.findByRole('dialog', { name: /settings/ });
    // Settings → Privacy follows the same rule (Q4-39): nothing there offers it either.
    expect(
      within(settings).queryByRole('button', { name: 'Make my connection public…' })
    ).toBeNull();
    expect(patches()).toEqual([]);
  });

  it('asks for the workspace name before the status-row popover makes the connection Public', async () => {
    // A workspace that allows Public: the one place the downgrade changes which models may read.
    savingDaemon({
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await user.click(screen.getByRole('button', { name: /^Privacy: Private · ucsf/ }));
    await user.click(await screen.findByRole('button', { name: 'Make my connection public…' }));
    let dialog = await makePublicConfirmation();
    expect(within(dialog).getByRole('button', { name: 'Make public' })).toBeDisabled();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(expectNothingOpen);
    expect(patches()).toEqual([]);

    await user.click(screen.getByRole('button', { name: /^Privacy: Private · ucsf/ }));
    await user.click(await screen.findByRole('button', { name: 'Make my connection public…' }));
    dialog = await makePublicConfirmation();
    await user.type(within(dialog).getByLabelText(`Type ${WORKSPACE} to confirm`), WORKSPACE);
    await user.click(within(dialog).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('public')]));
  });

  it('asks the same from the Privacy tab of Workspace settings', async () => {
    // A workspace that allows Public: the only one where Settings offers the downgrade (Q4-39).
    savingDaemon({
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Privacy…', /^lab/);
    const settings = await screen.findByRole('dialog', { name: /settings/ });
    await user.click(within(settings).getByRole('button', { name: 'Make my connection public…' }));
    let dialog = await makePublicConfirmation();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    // Cancel steps back to the settings it came from, having changed nothing.
    await waitFor(() =>
      expect(
        screen.queryByRole('alertdialog', { name: `Make your ${WORKSPACE} connection public?` })
      ).toBeNull()
    );
    expect(patches()).toEqual([]);

    await user.click(
      within(await screen.findByRole('dialog', { name: /settings/ })).getByRole('button', {
        name: 'Make my connection public…',
      })
    );
    dialog = await makePublicConfirmation();
    await user.type(within(dialog).getByLabelText(`Type ${WORKSPACE} to confirm`), WORKSPACE);
    await user.click(within(dialog).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('public')]));
  });

  it('asks the same when Connection settings is saved as Public, and sends the whole record', async () => {
    savingDaemon();
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Connection settings…', /^lab/);
    await user.click(await screen.findByRole('radio', { name: /^Public/ }));
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    let dialog = await makePublicConfirmation();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(patches()).toEqual([]);

    await user.click(await screen.findByRole('button', { name: 'Save connection' }));
    dialog = await makePublicConfirmation();
    await user.type(within(dialog).getByLabelText(`Type ${WORKSPACE} to confirm`), WORKSPACE);
    await user.click(within(dialog).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('public')]));
  });

  it('makes a Public connection Private in one click, with no confirmation', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    savingDaemon({
      connections: [publicConnection],
      connectionMode: 'public',
      snapshot: richSnapshot({
        workspace: { ...richSnapshot().workspace, mode: 'public' },
      }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await user.click(screen.getByRole('button', { name: /^Privacy: Public/ }));
    await user.click(await screen.findByRole('button', { name: 'Make private' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('private', publicConnection)]));
    expectNothingOpen();
  });

  it('confirms the permanent institution label before policy.set, from the note and the Privacy tab', async () => {
    installDaemon({
      snapshot: richSnapshot({
        workspace: { ...richSnapshot().workspace, institution_id: null },
      }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    // The host's note above the composer.
    await user.click(await screen.findByRole('button', { name: 'Set institution to ucsf…' }));
    let dialog = await screen.findByRole('alertdialog', {
      name: `Set ${WORKSPACE}’s institution to ucsf?`,
    });
    expect(policySets()).toEqual([]);
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(expectNothingOpen);
    expect(policySets()).toEqual([]);

    // Workspace settings → Privacy.
    await workspaceAction('Privacy…', /^lab/);
    await user.click(
      within(await screen.findByRole('dialog', { name: /settings/ })).getByRole('button', {
        name: 'Set institution to ucsf…',
      })
    );
    dialog = await screen.findByRole('alertdialog', {
      name: `Set ${WORKSPACE}’s institution to ucsf?`,
    });
    expect(policySets()).toEqual([]);
    await user.click(within(dialog).getByRole('button', { name: 'Set ucsf permanently' }));

    await waitFor(() =>
      expect(policySets()).toEqual([{ mode: 'private', institution_id: 'ucsf' }])
    );
  });
});

/**
 * SF2-N7: Workspace settings stays open through the reconnect its own privacy change causes. The
 * daemon ends that view with `policy_changed` and `clear: true`, which drops the last verified copy
 * as well, so until the view verifies again the dialog has no snapshot. Drawn from none, the people
 * directory named nobody as the host, and the host who had just pressed Make private lost the
 * Workspace row and read a member's sentences on the Privacy and People tabs.
 */
describe('Workspace settings through the reconnect its privacy change causes (SF2-N7)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const MEMBER_WORDS = [/Only the host/, /can invite new people/, /No one else has joined/];

  it('stays open for the host, says Checking… meanwhile and never a member’s words, then draws the host’s view again', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    const daemon = savingDaemon({
      connections: [publicConnection],
      connectionMode: 'public',
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Privacy…', /^lab/);
    const settings = await screen.findByRole('dialog', { name: `${WORKSPACE} settings` });
    // The host's own control on the Workspace row, before.
    expect(
      within(settings).getByRole('button', { name: 'Make Private for everyone…' })
    ).toBeInTheDocument();

    // The daemon saves the connection as Private, then holds the next view back while it connects
    // again, and ends the view the refresh opened with the policy change (`clear: true`).
    daemon.state.connections = [{ ...publicConnection, mode: 'private' }];
    daemon.state.connectionMode = 'private';
    daemon.state.hold = true;
    const before = mocked.observeCrew.mock.calls.length;
    await user.click(within(settings).getByRole('button', { name: 'Make private' }));
    await waitFor(() => expect(patches()).toEqual([fullBody('private', publicConnection)]));
    await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before));
    const ending = mocked.observeCrew.mock.calls.length;
    act(() =>
      daemon.emit({
        type: 'error',
        code: 'policy_changed',
        clear: true,
        error: 'Room observation ended.',
      })
    );

    // Nothing verified to draw: the dialog is still open, and says Checking… where the snapshot
    // decides, on every tab (they are all mounted), in no member's words.
    const checking = await screen.findByRole('dialog', { name: /settings$/ });
    for (const words of MEMBER_WORDS) expect(checking.textContent).not.toMatch(words);
    expect(checking.querySelector('.crew-settings-panels')).toHaveAttribute('aria-busy', 'true');
    expect(checking.querySelectorAll('[data-crew-settings-checking]').length).toBeGreaterThan(0);

    // It observes again by itself, still held: the same, and still open.
    await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(ending));
    const waiting = screen.getByRole('dialog', { name: /settings$/ });
    for (const words of MEMBER_WORDS) expect(waiting.textContent).not.toMatch(words);

    // The fresh view arrives: the host's view again, in the same dialog.
    act(() => daemon.release());
    const verified = await screen.findByRole('dialog', { name: `${WORKSPACE} settings` });
    await waitFor(() =>
      expect(
        within(verified).getByRole('button', { name: 'Make Private for everyone…' })
      ).toBeInTheDocument()
    );
    expect(verified.querySelector('[data-crew-settings-checking]')).toBeNull();
    expect(verified.querySelector('.crew-settings-panels')).not.toHaveAttribute('aria-busy');
    for (const words of MEMBER_WORDS) expect(verified.textContent).not.toMatch(words);
  });
});
