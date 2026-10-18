import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ConfirmIntent } from '../state/types';
import { channelAfterDeleting, CrewConfirmation } from './confirmations';
import { confirmCopy } from './copy';
import {
  alice,
  bob,
  connection,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';

const toasts = vi.hoisted(() => ({ toastSuccess: vi.fn() }));
vi.mock('../../../toasts', () => toasts);

function renderConfirm(confirm: ConfirmIntent, options: Parameters<typeof renderWithCrew>[1] = {}) {
  const onClose = vi.fn();
  const view = renderWithCrew(<CrewConfirmation confirm={confirm} onClose={onClose} />, options);
  return { ...view, onClose };
}

afterEach(() => vi.clearAllMocks());

// The typed confirmation reads the input that opened it (`ui/inputTrail`): a key open puts focus on
// Cancel, any other open rests on the dialog box (V3-R6-2). Each focus case says which it models.
const keyOpen = () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
const pointerOpen = () => window.dispatchEvent(new Event('pointerdown'));

describe('CrewConfirmation', () => {
  it('removes a person only when their username is typed exactly, case included', async () => {
    keyOpen();
    const { crew, onClose } = renderConfirm({ action: 'remove-person', principalId: bob.id });
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removePerson.title('Bob Lee (@bob)', 'lab'),
    });
    await waitFor(() =>
      expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus()
    );
    const remove = within(dialog).getByRole('button', { name: 'Remove from lab' });
    const field = within(dialog).getByLabelText('Type bob to confirm');
    expect(remove).toBeDisabled();

    // The primitive's gate is case-folded, so "Bob" enables the button …
    fireEvent.change(field, { target: { value: 'Bob' } });
    expect(remove).toBeEnabled();
    fireEvent.click(remove);
    // … and the exact check on top of it refuses, sending nothing.
    expect(await within(dialog).findByText(confirmCopy.removePerson.mismatch)).toBeInTheDocument();
    expect(requestsFor(crew, 'enrollment.revoke')).toEqual([]);
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.change(field, { target: { value: 'bob' } });
    await act(async () => {
      fireEvent.click(remove);
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'enrollment.revoke')).toEqual([
        { principal_id: bob.id, expected_username: 'bob' },
      ])
    );
    expect(crew.request.mock.calls.find(([method]) => method === 'enrollment.revoke')?.[2]).toEqual(
      { mutation: true }
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('opens a typed confirmation on its box after a pointer press, never on Cancel or the action', async () => {
    pointerOpen();
    renderConfirm({ action: 'remove-person', principalId: bob.id });
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removePerson.title('Bob Lee (@bob)', 'lab'),
    });
    await waitFor(() => expect(dialog).toHaveFocus());
    for (const button of within(dialog).getAllByRole('button')) {
      expect(button).not.toHaveFocus();
    }
  });

  it('never confirms on Enter in the typed field', async () => {
    const { crew } = renderConfirm({ action: 'remove-person', principalId: bob.id });
    const field = await screen.findByLabelText('Type bob to confirm');
    fireEvent.change(field, { target: { value: 'bob' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(requestsFor(crew, 'enrollment.revoke')).toEqual([]);
  });

  it('asks for the workspace name before allowing Public', async () => {
    const { crew } = renderConfirm({ action: 'allow-workspace-public' });
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.allowWorkspacePublic.title('lab'),
    });
    const allow = within(dialog).getByRole('button', { name: 'Allow Public' });
    expect(allow).toBeDisabled();
    fireEvent.change(within(dialog).getByLabelText('Type lab to confirm'), {
      target: { value: 'lab' },
    });
    await act(async () => {
      fireEvent.click(allow);
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'policy.set')).toEqual([{ mode: 'public', institution_id: 'ucsf' }])
    );
  });

  it('makes the workspace Private for everyone with one confirmation, focusing Cancel', async () => {
    const snapshot = makeSnapshot();
    snapshot.workspace.mode = 'public';
    const { crew, onClose } = renderConfirm({ action: 'make-workspace-private' }, { snapshot });
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.makeWorkspacePrivate.title('lab'),
    });
    await waitFor(() =>
      expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus()
    );
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Make Private for everyone' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'policy.set')).toEqual([{ mode: 'private', institution_id: 'ucsf' }])
    );
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: confirmCopy.makeWorkspacePrivate.toast('lab'),
      })
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('sets the institution permanently with no phrase and no key that confirms', async () => {
    const snapshot = makeSnapshot();
    snapshot.workspace.institution_id = null;
    const { crew } = renderConfirm(
      { action: 'set-institution', institutionId: 'ucsf' },
      { snapshot }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.setInstitution.title('lab', 'ucsf'),
    });
    expect(within(dialog).queryByRole('textbox')).toBeNull();
    expect(
      within(dialog).getByText(confirmCopy.setInstitution.description('ucsf'))
    ).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Set ucsf permanently' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'policy.set')).toEqual([{ mode: 'private', institution_id: 'ucsf' }])
    );
  });

  it('makes a connection public with the whole record, then observes it afresh', async () => {
    const { crew } = renderConfirm({
      action: 'make-connection-public',
      connectionId: connection.id,
    });
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.makeConnectionPublic.title('lab'),
    });
    fireEvent.change(within(dialog).getByLabelText('Type lab to confirm'), {
      target: { value: 'lab' },
    });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Make public' }));
    });
    await waitFor(() =>
      expect(crew.updateConnection).toHaveBeenCalledWith(connection.id, {
        ...connectionUpdateBody(connection),
        mode: 'public',
      })
    );
    await waitFor(() => expect(crew.refresh).toHaveBeenCalled());
  });

  it('archives a channel and removes a channel member by name, with the expected username', async () => {
    const archive = renderConfirm({ action: 'archive-channel', channelId: 'channel-general' });
    const archiveDialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.archiveChannel.title('#general'),
    });
    await act(async () => {
      fireEvent.click(within(archiveDialog).getByRole('button', { name: 'Archive channel' }));
    });
    await waitFor(() =>
      expect(requestsFor(archive.crew, 'channel.archive')).toEqual([
        { channel_id: 'channel-general' },
      ])
    );
    archive.unmount();

    const remove = renderConfirm({
      action: 'remove-channel-member',
      channelId: 'channel-general',
      principalId: bob.id,
    });
    const removeDialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeChannelMember.title('Bob Lee (@bob)', '#general'),
    });
    // M11: the way back is Add people, not an invitation to accept.
    expect(removeDialog).toHaveTextContent(
      'They’ll lose access to its messages and files. You can add them again with Add people.'
    );
    expect(removeDialog).not.toHaveTextContent(/invite them/);
    await act(async () => {
      fireEvent.click(within(removeDialog).getByRole('button', { name: 'Remove' }));
    });
    await waitFor(() =>
      expect(requestsFor(remove.crew, 'membership.revoke')).toEqual([
        { channel_id: 'channel-general', principal_id: bob.id, expected_username: 'bob' },
      ])
    );
  });

  it('stops a task through the cancel route, with its own wording', async () => {
    const { crew, onClose } = renderConfirm({ action: 'stop-task', runId: 'run-7' });
    const dialog = await screen.findByRole('alertdialog', { name: 'Stop your agent?' });
    expect(within(dialog).getByRole('button', { name: 'Keep running' })).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Stop task' }));
    expect(crew.cancelRun).toHaveBeenCalledWith('run-7');
    expect(onClose).toHaveBeenCalled();
  });

  it('shows a refusal once, in the confirmation that sent it', async () => {
    const { onClose } = renderConfirm(
      { action: 'archive-channel', channelId: 'channel-general' },
      {
        request: () => {
          throw new Error('forbidden: current owner required');
        },
      }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.archiveChannel.title('#general'),
    });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Archive channel' }));
    });
    expect(await within(dialog).findAllByText('forbidden: current owner required')).toHaveLength(1);
    expect(onClose).not.toHaveBeenCalled();
  });

  it('says what removing a connection costs, and never that the workspace can simply be added again', async () => {
    // A host with another computer listed: whether it can still act as host is not known here.
    const snapshot = makeSnapshot({
      actor: {
        ...alice,
        devices: [{ fingerprint: 'DE1C E000 0000 0000' }, { fingerprint: 'AAAA BBBB CCCC DDDD' }],
      },
    });
    const { crew } = renderConfirm(
      { action: 'remove-connection', connectionId: connection.id },
      { snapshot }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    const description = confirmCopy.removeConnection.description;
    expect(within(dialog).getByText(description)).toBeInTheDocument();
    // The device key is deleted and the member stays enrolled, so the old invitation cannot
    // bring the workspace back: only the host adding this computer does.
    expect(description).not.toMatch(/add it again/i);
    expect(description).toMatch(/deletes this computer’s key/);
    expect(description).toMatch(/the host has to add this computer/);
    // A host removing it from their last computer ends the host controls for good.
    expect(description).toMatch(/If you host it and no other computer of yours still has it/);
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Remove' }));
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith(connection.id));
  });

  // F3: a removed computer is told "not in lab yet" when it joins again, because a plain
  // invitation cannot bring a member back; the confirmation names the one that can.
  it('tells a member which invitation brings the workspace back to this computer', async () => {
    const { crew } = renderConfirm(
      { action: 'remove-connection', connectionId: connection.id },
      { snapshot: makeSnapshot({ actor: bob }) }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    expect(dialog).toHaveTextContent(
      'To use lab on this computer again, ask the host to invite you with Add another device for @bob.'
    );
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Remove' }));
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith(connection.id));
  });

  // CLI-1 (desktop half): the CLI refuses to remove the host's only computer unless told to give
  // up the host controls; the desktop asks for the workspace's name first.
  it('asks the host for the workspace’s name before removing the only computer that can host it', async () => {
    const { crew } = renderConfirm({ action: 'remove-connection', connectionId: connection.id });
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    expect(dialog).toHaveTextContent(
      confirmCopy.removeConnection.onlyHostDescription('lab', 'alice')
    );
    const remove = within(dialog).getByRole('button', {
      name: confirmCopy.removeConnection.onlyHostConfirm,
    });
    expect(remove).toBeDisabled();
    fireEvent.click(remove);
    expect(crew.removeConnection).not.toHaveBeenCalled();
    fireEvent.change(within(dialog).getByRole('textbox'), { target: { value: 'lab' } });
    await act(async () => {
      fireEvent.click(remove);
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith(connection.id));
  });

  it('closes rather than confirm a removal it has no username for', async () => {
    const { onClose } = renderConfirm({ action: 'remove-person', principalId: 'person-unknown' });
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(screen.queryByRole('alertdialog')).toBeNull();
  });
});

describe('deleting channels, teams and workspaces (OW-4, OW-5)', () => {
  const twoChannels = () =>
    makeSnapshot({
      channels: [
        ...makeSnapshot().channels,
        {
          id: 'channel-methods',
          team_id: 'team-1',
          name: 'methods',
          created_by: alice.id,
          owner_id: alice.id,
          members: [alice.id, bob.id],
          archived: false,
          classification: 'restricted',
        },
      ],
    });

  it('names what a channel deletion removes, and sends the name the person saw', async () => {
    const { crew, onClose } = renderConfirm(
      { action: 'delete-channel', channelId: 'channel-methods' },
      { snapshot: twoChannels() }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.deleteChannel.title('#methods'),
    });
    // Pinned: "removed from the workspace", never "erased" (the journal keeps old records).
    expect(dialog).toHaveTextContent(
      'Removes #methods, its files and its messages from the workspace.'
    );
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Delete channel' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.delete')).toEqual([
        { channel_id: 'channel-methods', expected_name: 'methods' },
      ])
    );
    // Another channel is on screen: refreshed in place, nowhere else to go.
    expect(crew.selectChannel).not.toHaveBeenCalled();
    expect(crew.refresh).toHaveBeenCalled();
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: confirmCopy.deleteChannel.toast('#methods'),
      })
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('moves off the channel on screen before its deletion is observed', async () => {
    const { crew } = renderConfirm(
      { action: 'delete-channel', channelId: 'channel-methods' },
      { snapshot: twoChannels(), channelId: 'channel-methods' }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.deleteChannel.title('#methods'),
    });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Delete channel' }));
    });
    await waitFor(() => expect(crew.selectChannel).toHaveBeenCalledWith('channel-general'));
    expect(requestsFor(crew, 'channel.delete')).toHaveLength(1);
  });

  it('keeps a refused channel deletion in its dialog, once', async () => {
    const { onClose } = renderConfirm(
      { action: 'delete-channel', channelId: 'channel-methods' },
      {
        snapshot: twoChannels(),
        request: () => {
          throw new Error('name_changed: This was renamed since you confirmed.');
        },
      }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.deleteChannel.title('#methods'),
    });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Delete channel' }));
    });
    expect(await within(dialog).findAllByText(/This was renamed since you confirmed/)).toHaveLength(
      1
    );
    expect(onClose).not.toHaveBeenCalled();
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
  });

  it('deletes a team with every channel in it, and says how many went', async () => {
    const { crew, onClose } = renderConfirm(
      { action: 'delete-team', teamId: 'team-1' },
      {
        snapshot: twoChannels(),
        request: () => ({ deleted: true, team_id: 'team-1', channels: 3 }),
      }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.deleteTeam.title('Analysis Lab'),
    });
    expect(dialog).toHaveTextContent(/including channels you aren’t in/);
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Delete team' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'team.delete')).toEqual([
        { team_id: 'team-1', expected_name: 'Analysis Lab' },
      ])
    );
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: 'Deleted Analysis Lab and its 3 channels.',
      })
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('chooses where to go: the team’s #general, then its other channels, then any team', () => {
    const snapshot = twoChannels();
    expect(channelAfterDeleting(snapshot, { channelId: 'channel-methods' })).toBe(
      'channel-general'
    );
    expect(channelAfterDeleting(snapshot, { channelId: 'channel-general' })).toBe(
      'channel-methods'
    );
    expect(channelAfterDeleting(snapshot, { teamId: 'team-1' })).toBeNull();
    expect(channelAfterDeleting(null, { teamId: 'team-1' })).toBeNull();
  });

  it('deletes the workspace only once its name is typed, with the id and name it stores', async () => {
    pointerOpen();
    const { crew, onClose } = renderConfirm({
      action: 'delete-workspace',
      connectionId: connection.id,
    });
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.deleteWorkspace.title('lab'),
    });
    expect(dialog).toHaveTextContent(
      confirmCopy.deleteWorkspace.description('lab', 'hpc.example.edu')
    );
    const confirm = within(dialog).getByRole('button', { name: 'Delete workspace' });
    expect(confirm).toBeDisabled();
    fireEvent.change(within(dialog).getByLabelText('Type lab to confirm'), {
      target: { value: 'LAB' },
    });
    await act(async () => {
      fireEvent.click(confirm);
    });
    await waitFor(() =>
      expect(crew.deleteWorkspace).toHaveBeenCalledWith(connection.id, {
        workspace_id: connection.workspace_id,
        expected_name: 'lab',
      })
    );
    await waitFor(() => expect(toasts.toastSuccess).toHaveBeenCalledWith({ msg: 'Deleted lab.' }));
    expect(onClose).toHaveBeenCalled();
    // A deletion is no broker request the generic door could send.
    expect(requestsFor(crew, 'workspace.delete')).toEqual([]);
  });

  it('deletes a workspace from before names with expected_name null, typed as its connection (C19)', async () => {
    pointerOpen();
    // A workspace started without a name stores none: the broker compares null with that, so the
    // connection's name is only what the person types, never what is sent.
    const snapshot = makeSnapshot();
    delete (snapshot.workspace as { name?: string }).name;
    const { crew, onClose } = renderConfirm(
      { action: 'delete-workspace', connectionId: connection.id },
      { snapshot }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.deleteWorkspace.title('Fixture'),
    });
    fireEvent.change(within(dialog).getByLabelText('Type Fixture to confirm'), {
      target: { value: 'Fixture' },
    });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Delete workspace' }));
    });
    await waitFor(() =>
      expect(crew.deleteWorkspace).toHaveBeenCalledWith(connection.id, {
        workspace_id: connection.workspace_id,
        expected_name: null,
      })
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('removes a workspace its host deleted without the host or rejoin warnings (L6)', async () => {
    // The host's own last computer, which would otherwise be asked to type the name and told the
    // host controls end for good: the workspace is gone, so there are none to give up.
    const deleted = { ...connection, status: 'disconnected' as const, workspace_deleted: true };
    const { crew, onClose } = renderConfirm(
      { action: 'remove-connection', connectionId: connection.id },
      { connections: [deleted] }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    expect(dialog).toHaveTextContent(confirmCopy.removeConnection.deletedDescription('lab'));
    expect(dialog).not.toHaveTextContent(/host controls|the host has to add this computer/);
    expect(within(dialog).queryByLabelText('Type lab to confirm')).toBeNull();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Remove' }));
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith(connection.id));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  // G9: the main area said "The host deleted lab." from the ended view or the last connect, before
  // the saved record caught up, and the confirmation still warned that messages stay on the server
  // and that the host can add this computer back. It asks what the main area asks now.
  it.each([
    ['a view that ended workspace_deleted', { refreshErrorCode: 'workspace_deleted' }],
    [
      'a connect answered crew_workspace_deleted',
      {
        lastConnectFailure: {
          kind: 'unknown' as const,
          message: 'The host deleted this workspace on the server.',
          code: 'crew_workspace_deleted',
        },
      },
    ],
  ])('says only what goes from this computer after %s', async (_what, heard) => {
    // No verified view is left once the view ended, as on the deleted workspace's own screen, so
    // the workspace goes by its connection's name.
    const { crew } = renderConfirm(
      { action: 'remove-connection', connectionId: connection.id },
      { snapshot: null, ...heard }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('Fixture'),
    });
    expect(dialog).toHaveTextContent(confirmCopy.removeConnection.deletedDescription('Fixture'));
    expect(dialog).not.toHaveTextContent(
      /stay on the server|host has to add this computer|nobody can let people in/
    );
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Remove' }));
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith(connection.id));
  });

  it('keeps the generic warnings when nothing says the workspace was deleted', async () => {
    renderConfirm(
      { action: 'remove-connection', connectionId: connection.id },
      { snapshot: null, refreshErrorCode: 'forbidden' }
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('Fixture'),
    });
    expect(dialog).toHaveTextContent(confirmCopy.removeConnection.description);
  });

  it('reads another connection’s deletion from its saved record alone', async () => {
    // The selected connection's ended view is about the selected connection, not this one.
    const other = { ...connection, id: 'conn-2', name: 'other' };
    renderConfirm(
      { action: 'remove-connection', connectionId: other.id },
      { snapshot: null, connections: [connection, other], refreshErrorCode: 'workspace_deleted' }
    );
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).not.toHaveTextContent(/The host deleted/);
  });

  it('keeps a refused workspace deletion in its dialog', async () => {
    const { crew, onClose } = renderConfirm({
      action: 'delete-workspace',
      connectionId: connection.id,
    });
    crew.deleteWorkspace.mockRejectedValueOnce(
      new Error('forbidden: human host decision required')
    );
    const dialog = await screen.findByRole('alertdialog', {
      name: confirmCopy.deleteWorkspace.title('lab'),
    });
    fireEvent.change(within(dialog).getByLabelText('Type lab to confirm'), {
      target: { value: 'lab' },
    });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Delete workspace' }));
    });
    expect(await within(dialog).findAllByText(/human host decision required/)).toHaveLength(1);
    expect(onClose).not.toHaveBeenCalled();
  });
});
