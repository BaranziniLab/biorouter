import { screen, waitFor, within } from '@testing-library/react';
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

  it('asks for the workspace name before the status-row popover makes the connection Public', async () => {
    savingDaemon();
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
    savingDaemon();
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
