import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { STALE_DAEMON_MESSAGE } from '../api/errors';
import { inviteCopy } from './copy';
import { installResizeObserverStub, renderWithCrew, requestsFor } from './dialogsTestHarness';
import { InvitePeopleDialog } from './InvitePeopleDialog';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

installResizeObserverStub();

const LINE = 'brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ';
const MESSAGE = `Join lab on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste this whole message.\n${LINE}`;

function renderInvite(refuse?: string) {
  const onClose = vi.fn();
  const view = renderWithCrew(<InvitePeopleDialog onClose={onClose} />, {
    request: (method, params) => {
      if (method !== 'enrollment.invite') return {};
      // What the daemon answers a broker refusal with: its text, as `CrewHttpError.message`.
      if (refuse) throw new CrewHttpError(refuse, 400, 'crew_request_refused');
      if ('uid' in params) return { invitation: 'token-secret-value' };
      return {
        username: 'bob',
        full_name: 'Bob Lee',
        add_device: params.add_device === true,
        join_id: 'join-1',
        expires_at: 1,
      };
    },
  });
  return { ...view, onClose };
}

async function invite(username: string) {
  fireEvent.change(await screen.findByLabelText('Username'), { target: { value: username } });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Invite' }));
  });
}

beforeEach(() => {
  mocks.crewHttp.mockImplementation(async (path: string) =>
    path.startsWith('/connections/conn-1/invitation') ? { message: MESSAGE, line: LINE } : {}
  );
});
afterEach(() => vi.clearAllMocks());

describe('InvitePeopleDialog', () => {
  it('has one visible field, focused, with the @ as an adornment', async () => {
    renderInvite();
    expect(await screen.findByRole('dialog', { name: 'Invite people to lab' })).toBeInTheDocument();
    const username = screen.getByLabelText('Username');
    await waitFor(() => expect(username).toHaveFocus());
    expect(username).toBeRequired();
    expect(screen.queryByLabelText(inviteCopy.legacy.joinRequest)).toBeNull();
  });

  it('invites by username and renders the broker’s answer, then the message to send', async () => {
    const { crew } = renderInvite();
    await invite('@bob');
    expect(requestsFor(crew, 'enrollment.invite')).toEqual([{ username: 'bob' }]);

    const dialog = await screen.findByRole('dialog', { name: 'Invite people to lab' });
    expect(
      within(dialog).getByText((_, node) =>
        Boolean(
          node?.tagName === 'P' &&
          node.textContent === '@bob · Bob Lee (name on the server account) · invited'
        )
      )
    ).toBeInTheDocument();
    expect(within(dialog).getByText(inviteCopy.sendInvitation('Bob'))).toBeInTheDocument();
    expect(
      await within(dialog).findByRole('button', { name: 'Copy invitation message' })
    ).toBeInTheDocument();
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn-1/invitation?invitee=bob',
      'GET',
      undefined,
      undefined
    );
    expect(dialog).toHaveTextContent(LINE);
    expect(
      within(dialog).getByRole('button', { name: inviteCopy.installed('bob', 'hpc.example.edu') })
    ).toBeInTheDocument();
    expect(within(dialog).getByText(inviteCopy.nextStep('Bob'))).toBeInTheDocument();
    await waitFor(() => expect(within(dialog).getByRole('button', { name: 'Done' })).toHaveFocus());
  });

  it('says a stale background service plainly when the message cannot be built', async () => {
    mocks.crewHttp.mockRejectedValue(new CrewHttpError('', 404));
    renderInvite();
    await invite('bob');
    expect(await screen.findByText(STALE_DAEMON_MESSAGE)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
  });

  it('offers the account’s exact spelling rather than accepting a near miss', async () => {
    renderInvite('identity_ambiguous: This server spells the account @bob. Invite @bob.');
    await invite('Bob');
    expect(await screen.findByText(inviteCopy.refusal.canonical('bob'))).toBeInTheDocument();
    expect(screen.getByLabelText('Username')).toHaveAttribute('aria-invalid', 'true');
  });

  it('reads an older daemon’s refusal envelope the same way, and never shows its JSON', async () => {
    const view = renderInvite(
      `Crew broker refused request: ${JSON.stringify({
        code: 'identity_ambiguous',
        message: 'identity_ambiguous: @Al is an alias on this server. Invite @alice.',
      })}`
    );
    await invite('Al');
    expect(await screen.findByText(inviteCopy.refusal.canonical('alice'))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('Crew broker refused request');
    view.unmount();

    renderInvite(
      `Crew broker refused request: ${JSON.stringify({
        code: 'identity_conflict',
        message:
          'identity_conflict: Another account on this server is already invited as @bob. Cancel that invitation first.',
      })}`
    );
    await invite('Bob');
    expect(
      await screen.findByText(
        'Another account on this server is already invited as @bob. Cancel that invitation first.'
      )
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('{');
  });

  it('says there is no such account, and offers Add device for an existing member', async () => {
    const missing = renderInvite(
      'unknown_account: There is no account @zed on this server. Check the spelling.'
    );
    await invite('zed');
    expect(await screen.findByText(inviteCopy.refusal.noAccount('zed'))).toBeInTheDocument();
    missing.unmount();

    const member = renderInvite(
      'already_member: @bob is already a member. Choose Add device to add another computer for them.'
    );
    await invite('bob');
    expect(
      await screen.findByText(inviteCopy.refusal.alreadyMember('bob', 'lab'))
    ).toBeInTheDocument();
    const addDevice = screen.getByRole('switch', { name: inviteCopy.addDevice('bob') });
    fireEvent.click(addDevice);
    member.crew.request.mockClear();
    await invite('bob');
    expect(requestsFor(member.crew, 'enrollment.invite')).toEqual([
      { username: 'bob', add_device: true },
    ]);
  });

  it('keeps the older token path under its own disclosure, and shows the token masked', async () => {
    const { crew } = renderInvite();
    fireEvent.click(await screen.findByRole('button', { name: inviteCopy.legacy.toggle }));
    const request = await screen.findByLabelText(inviteCopy.legacy.joinRequest);
    fireEvent.change(request, { target: { value: 'Crew join request\nUsername: @dana\nno key' } });
    expect(request).toBeInvalid();

    const key = 'ef'.repeat(32);
    fireEvent.change(request, {
      target: { value: `Crew join request\nUsername: @dana\nDevice key: ${key}` },
    });
    expect(request).toBeValid();
    expect(screen.getByText('id -u dana')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(inviteCopy.legacy.userId('hpc.example.edu')), {
      target: { value: '1050' },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: inviteCopy.legacy.submit }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'enrollment.invite')).toEqual([{ uid: 1050, public_key: key }])
    );
    expect(
      await screen.findByRole('button', { name: 'Copy invitation token' })
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('token-secret-value');
    expect(screen.getByText(inviteCopy.legacy.sendToken('@dana'))).toBeInTheDocument();
  });
});
