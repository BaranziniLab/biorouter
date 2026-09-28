import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ConfirmIntent } from '../state/types';
import { CrewConfirmation } from './confirmations';
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

describe('CrewConfirmation', () => {
  it('removes a person only when their username is typed exactly, case included', async () => {
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
