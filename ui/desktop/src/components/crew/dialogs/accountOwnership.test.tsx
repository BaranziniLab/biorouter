import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { deviceCopy } from '../account/copy';
import { needsRejoin, resetRejoinMarksForTests } from '../account/rejoinMarks';
import { ACCOUNT_IDENTITY_CAPABILITY, CrewHttpError } from '../crewApi';
import { CrewConfirmation } from './confirmations';
import { confirmCopy, inviteCopy, keysCopy } from './copy';
import {
  alice,
  bob,
  connection,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { InvitePeopleDialog } from './InvitePeopleDialog';
import { KeysDialog } from './KeysDialog';

/**
 * The dialogs' account-ownership half (docs/crew/design/account-ownership.md, "Desktop"): Keys'
 * Remove on every device row with AO-5's warning, the removal confirmation by `account_identity`,
 * and the invitation copy for a workspace that belongs to accounts.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
const toasts = vi.hoisted(() => ({ toastSuccess: vi.fn() }));
vi.mock('../../../toasts', () => toasts);

installResizeObserverStub();

const credentials = vi.fn();
const MINE = 'AAAA BBBB CCCC DDDD';
const OTHER = '1111 2222 3333 4444';

beforeEach(() => {
  resetRejoinMarksForTests();
  credentials.mockReset();
  credentials.mockResolvedValue({ backend: 'keyring', initialized: true, locked: false });
  (window as unknown as { electron: Record<string, unknown> }).electron = {
    ...(window as unknown as { electron?: Record<string, unknown> }).electron,
    crewCredentials: credentials,
  };
  mocks.crewHttp.mockResolvedValue({});
});
afterEach(() => vi.clearAllMocks());

function renderKeys(capabilities: string[] | null = [ACCOUNT_IDENTITY_CAPABILITY]) {
  return renderWithCrew(<KeysDialog onClose={vi.fn()} />, {
    capabilities,
    snapshot: makeSnapshot({
      actor: {
        ...alice,
        devices: [
          { fingerprint: OTHER, added_at: 1_700_000_000, added_via: 'account' },
          { fingerprint: MINE, added_at: 1_690_000_000, added_via: 'bootstrap', this_device: true },
        ],
      } as never,
    }),
  });
}

describe('Keys: removing a device (AO-5)', () => {
  it('offers Remove on every row, this computer included, and says how a computer was added', async () => {
    renderKeys();
    const devices = await screen.findByRole('region', { name: keysCopy.devices });
    expect(
      within(devices).getByRole('button', { name: deviceCopy.removeLabel(OTHER) })
    ).toBeVisible();
    expect(within(devices).getByRole('button', { name: deviceCopy.removeThisLabel })).toBeVisible();
    expect(within(devices).getByText(/by signing in to the server/)).toBeVisible();
  });

  it('asks first with the SSH warning, then revokes the device by fingerprint', async () => {
    const { crew } = renderKeys();
    const devices = await screen.findByRole('region', { name: keysCopy.devices });
    fireEvent.click(within(devices).getByRole('button', { name: deviceCopy.removeLabel(OTHER) }));
    const ask = within(devices).getByRole('group', { name: deviceCopy.confirm(OTHER) });
    expect(ask).toHaveTextContent(deviceCopy.sshWarning);
    expect(ask).toHaveTextContent(/authorized_keys/);
    expect(requestsFor(crew, 'device.revoke')).toEqual([]);
    await act(async () => {
      fireEvent.click(within(ask).getByRole('button', { name: deviceCopy.confirmAction }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'device.revoke')).toEqual([{ fingerprint: OTHER }])
    );
    expect(crew.request).toHaveBeenLastCalledWith(
      'device.revoke',
      { fingerprint: OTHER },
      { mutation: true }
    );
    expect(await screen.findByTestId('crew-device-removed')).toHaveTextContent(
      deviceCopy.removed(OTHER)
    );
    expect(needsRejoin(connection.id)).toBe(false);
  });

  it('marks the connection for Add this computer again when this computer is removed', async () => {
    const { crew } = renderKeys();
    const devices = await screen.findByRole('region', { name: keysCopy.devices });
    fireEvent.click(within(devices).getByRole('button', { name: deviceCopy.removeThisLabel }));
    const ask = within(devices).getByRole('group', { name: deviceCopy.confirmThis });
    expect(ask).toHaveTextContent(deviceCopy.thisFollows);
    await act(async () => {
      fireEvent.click(within(ask).getByRole('button', { name: deviceCopy.confirmAction }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'device.revoke')).toEqual([{ fingerprint: MINE }])
    );
    expect(needsRejoin(connection.id)).toBe(true);
    expect(await screen.findByTestId('crew-device-removed')).toHaveTextContent(
      deviceCopy.removedThis
    );
    // The view can no longer be read with this computer's key: nothing reads it again.
    expect(crew.refresh).not.toHaveBeenCalled();
  });

  it('keeps the device when the person says Keep, and offers nothing on an older server', async () => {
    const view = renderKeys();
    const devices = await screen.findByRole('region', { name: keysCopy.devices });
    fireEvent.click(within(devices).getByRole('button', { name: deviceCopy.removeLabel(OTHER) }));
    fireEvent.click(within(devices).getByRole('button', { name: deviceCopy.keep }));
    expect(requestsFor(view.crew, 'device.revoke')).toEqual([]);
    view.unmount();
    renderKeys(null);
    const older = await screen.findByRole('region', { name: keysCopy.devices });
    expect(within(older).queryByRole('button', { name: /^Remove/ })).toBeNull();
  });

  it('says why a revoke was refused, in the dialog', async () => {
    const view = renderWithCrew(<KeysDialog onClose={vi.fn()} />, {
      capabilities: [ACCOUNT_IDENTITY_CAPABILITY],
      request: (method) => {
        if (method === 'device.revoke')
          throw new CrewHttpError('That device is not one of yours.', 400, 'crew_request_refused');
        return {};
      },
      snapshot: makeSnapshot({
        actor: { ...alice, devices: [{ fingerprint: OTHER }, { fingerprint: MINE }] } as never,
      }),
    });
    const devices = await screen.findByRole('region', { name: keysCopy.devices });
    fireEvent.click(within(devices).getByRole('button', { name: deviceCopy.removeLabel(OTHER) }));
    await act(async () => {
      fireEvent.click(within(devices).getByRole('button', { name: deviceCopy.confirmAction }));
    });
    expect(await screen.findByText('That device is not one of yours.')).toBeInTheDocument();
    expect(view.container.querySelector('[data-testid="crew-device-removed"]')).toBeNull();
  });
});

describe('removing a connection by account_identity (AO-9, AO-11)', () => {
  it('drops the host-only guard on a workspace that belongs to accounts', async () => {
    const { crew } = renderWithCrew(
      <CrewConfirmation
        confirm={{ action: 'remove-connection', connectionId: connection.id }}
        onClose={vi.fn()}
      />,
      { connections: [{ ...connection, account_identity: true }] }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    expect(dialog).toHaveTextContent(
      'You can open lab again from any computer that signs in to hpc.example.edu as @alice.'
    );
    expect(dialog).toHaveTextContent(/can’t be given access through a new one/);
    expect(dialog).toHaveTextContent(/choose Add this computer again instead/);
    expect(within(dialog).queryByRole('textbox')).toBeNull();
    expect(
      within(dialog).queryByRole('button', { name: confirmCopy.removeConnection.onlyHostConfirm })
    ).toBeNull();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Remove' }));
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith(connection.id));
  });

  it('keeps the guard on an older server, saying that updating Crew first helps', async () => {
    renderWithCrew(
      <CrewConfirmation
        confirm={{ action: 'remove-connection', connectionId: connection.id }}
        onClose={vi.fn()}
      />,
      { connections: [{ ...connection, account_identity: false }] }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    expect(dialog).toHaveTextContent(
      confirmCopy.removeConnection.onlyHostUpdateFirst('alice@hpc.example.edu')
    );
    expect(within(dialog).getByRole('textbox')).toBeInTheDocument();
  });

  it('tells a person being removed that signing in no longer brings them back', async () => {
    renderWithCrew(
      <CrewConfirmation
        confirm={{ action: 'remove-person', principalId: bob.id }}
        onClose={vi.fn()}
      />
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removePerson.title('Bob Lee (@bob)', 'lab'),
    });
    expect(dialog).toHaveTextContent(
      /Signing in to the server as them no longer opens the workspace/
    );
  });
});

describe('inviting on a workspace that belongs to accounts', () => {
  function renderInvite(capabilities: string[] | null) {
    return renderWithCrew(<InvitePeopleDialog onClose={vi.fn()} />, {
      capabilities,
      request: (method) => {
        if (method !== 'enrollment.invite') return {};
        throw new CrewHttpError(
          'already_member: @bob is already a member. Choose Add device to add another computer for them.',
          400,
          'crew_request_refused'
        );
      },
    });
  }

  it('never turns Add another device on: a member’s new computer signs in by itself', async () => {
    renderInvite([ACCOUNT_IDENTITY_CAPABILITY]);
    fireEvent.change(await screen.findByLabelText('Username'), { target: { value: 'bob' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Invite' }));
    });
    expect(
      await screen.findByText(inviteCopy.refusal.alreadyMemberAccount('bob', 'lab'))
    ).toBeInTheDocument();
    expect(screen.queryByRole('switch')).toBeNull();
    expect(document.body.textContent).not.toMatch(/Add another device/);
  });

  it('keeps Add another device on an older server', async () => {
    renderInvite(null);
    fireEvent.change(await screen.findByLabelText('Username'), { target: { value: 'bob' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Invite' }));
    });
    expect(
      await screen.findByText(inviteCopy.refusal.alreadyMember('bob', 'lab'))
    ).toBeInTheDocument();
    expect(screen.getByRole('switch')).toBeChecked();
  });
});
