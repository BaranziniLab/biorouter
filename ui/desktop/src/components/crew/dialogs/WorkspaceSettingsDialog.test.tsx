import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { confirmCopy, workspaceSettingsCopy as copy } from './copy';
import {
  alice,
  bob,
  connection,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { WorkspaceSettingsDialog } from './WorkspaceSettingsDialog';

installResizeObserverStub();

afterEach(() => vi.clearAllMocks());

function renderSettings(
  props: Partial<Parameters<typeof WorkspaceSettingsDialog>[0]> = {},
  options: Parameters<typeof renderWithCrew>[1] = {}
) {
  const onClose = vi.fn();
  const view = renderWithCrew(<WorkspaceSettingsDialog onClose={onClose} {...props} />, options);
  return { ...view, onClose };
}

describe('WorkspaceSettingsDialog', () => {
  it('opens on the requested tab, titled by the workspace, with Done', async () => {
    renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(within(dialog).getByRole('tab', { name: 'Privacy' })).toHaveAttribute(
      'aria-selected',
      'true'
    );
    await waitFor(() => expect(within(dialog).getByRole('tab', { name: 'Privacy' })).toHaveFocus());
    expect(within(dialog).getByRole('button', { name: 'Done' })).toBeInTheDocument();
    // Agent access is a slot: without content from the access area it is not offered.
    expect(within(dialog).queryByRole('tab', { name: 'Agent access' })).toBeNull();
  });

  it('renders the Agent access slot it is given', async () => {
    renderSettings({ tab: 'agent-access', agentAccess: <p>agent access fixture</p> });
    expect(await screen.findByText('agent access fixture')).toBeInTheDocument();
  });

  it('names the host and the server on General', async () => {
    renderSettings();
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent('Hosted by');
    expect(dialog).toHaveTextContent('Alice Chen (@alice)');
    expect(dialog).toHaveTextContent('hpc.example.edu');
    // Rename appears only where the broker speaks the S2 naming rules.
    expect(within(dialog).queryByRole('button', { name: 'Rename…' })).toBeNull();
  });

  it('offers Rename to the host when the broker projects name handles', async () => {
    const snapshot = makeSnapshot();
    snapshot.teams[0].handle = 'analysis-lab';
    const { crew } = renderSettings({}, { snapshot });
    fireEvent.click(await screen.findByRole('button', { name: 'Rename…' }));
    expect(crew.current().ui.dialog).toEqual({
      kind: 'rename',
      target: 'workspace',
      targetId: 'workspace-1',
    });
  });

  it('lists members by name with a row menu to copy and, for the host, remove', async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const { crew } = renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent('Bob Lee');
    expect(dialog).not.toHaveTextContent(bob.id);

    await user.click(within(dialog).getByRole('button', { name: 'Bob Lee (@bob) options' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Copy person ID' }));
    expect(writeText).toHaveBeenCalledWith(bob.id);

    await user.click(within(dialog).getByRole('button', { name: 'Bob Lee (@bob) options' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Remove from lab…' }));
    const confirm = await screen.findByRole('dialog', {
      name: confirmCopy.removePerson.title('Bob Lee (@bob)', 'lab'),
    });
    expect(within(confirm).getByLabelText('Type bob to confirm')).toBeInTheDocument();
    expect(requestsFor(crew, 'enrollment.revoke')).toEqual([]);
  });

  it('does not offer the host a way to remove themselves or the host', async () => {
    const user = userEvent.setup();
    renderSettings({ tab: 'people' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    await user.click(within(dialog).getByRole('button', { name: 'Alice Chen (@alice) options' }));
    expect(await screen.findByRole('menuitem', { name: 'Copy username' })).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: 'Remove from lab…' })).toBeNull();
  });

  it('shows waiting joiners to the host, with Let in and an inline cancel', async () => {
    const snapshot = makeSnapshot({
      pending_joins: [{ username: 'eve', full_name: 'Eve Park', mismatched_attempts: 1 }],
    });
    const { crew } = renderSettings({ tab: 'people' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent('@eve · Eve Park (name on the server account)');
    expect(within(dialog).getByText(copy.otherDevice('eve'))).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel @eve’s invitation' }));
    expect(within(dialog).getByText(copy.cancelInvitationConfirm('eve'))).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel invitation' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'enrollment.cancel')).toEqual([{ username: 'eve' }])
    );

    fireEvent.click(within(dialog).getByRole('button', { name: 'Let @eve in' }));
    expect(crew.current().ui.dialog).toEqual({ kind: 'let-in', username: 'eve' });
  });

  it('shows a member neither waiting joiners nor host controls', async () => {
    const snapshot = makeSnapshot({
      actor: bob,
      pending_joins: [{ username: 'eve' }],
    });
    renderSettings({ tab: 'privacy' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(within(dialog).queryByRole('button', { name: copy.allowPublic })).toBeNull();
    expect(within(dialog).getAllByText(copy.hostOnly).length).toBeGreaterThan(0);
    fireEvent.mouseDown(within(dialog).getByRole('tab', { name: 'People' }));
    fireEvent.click(within(dialog).getByRole('tab', { name: 'People' }));
    await waitFor(() => expect(dialog).toHaveTextContent('Members'));
    expect(dialog).not.toHaveTextContent('Waiting to join');
    expect(within(dialog).queryByRole('button', { name: copy.invite })).toBeNull();
  });

  it('asks for the workspace name before allowing Public, from the Privacy tab', async () => {
    const { crew } = renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent(copy.privateForEveryone);
    fireEvent.click(within(dialog).getByRole('button', { name: copy.allowPublic }));
    const confirm = await screen.findByRole('dialog', {
      name: confirmCopy.allowWorkspacePublic.title('lab'),
    });
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(
        screen.queryByRole('dialog', { name: confirmCopy.allowWorkspacePublic.title('lab') })
      ).toBeNull()
    );
    // Cancel returns to the settings, having sent nothing.
    expect(screen.getByRole('dialog', { name: 'lab settings' })).toBeInTheDocument();
    expect(requestsFor(crew, 'policy.set')).toEqual([]);
  });

  it('makes the connection public only through the typed confirmation', async () => {
    const { crew } = renderSettings({ tab: 'privacy' });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    fireEvent.click(within(dialog).getByRole('button', { name: copy.makePublic }));
    const confirm = await screen.findByRole('dialog', {
      name: confirmCopy.makeConnectionPublic.title('lab'),
    });
    expect(crew.updateConnection).not.toHaveBeenCalled();
    fireEvent.change(within(confirm).getByLabelText('Type lab to confirm'), {
      target: { value: 'lab' },
    });
    await act(async () => {
      fireEvent.click(within(confirm).getByRole('button', { name: 'Make public' }));
    });
    await waitFor(() =>
      expect(crew.updateConnection).toHaveBeenCalledWith(connection.id, {
        ...connectionUpdateBody(connection),
        mode: 'public',
      })
    );
  });

  it('makes a public connection private in one click, or asks for the institution first', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    const one = renderSettings({ tab: 'privacy' }, { connections: [publicConnection] });
    fireEvent.click(await screen.findByRole('button', { name: copy.makePrivate }));
    await waitFor(() =>
      expect(one.crew.updateConnection).toHaveBeenCalledWith(connection.id, {
        ...connectionUpdateBody(publicConnection),
        mode: 'private',
      })
    );
    one.unmount();

    const bare = { ...publicConnection, institution_id: null };
    const two = renderSettings({ tab: 'privacy' }, { connections: [bare] });
    fireEvent.click(await screen.findByRole('button', { name: copy.makePrivate }));
    const ask = await screen.findByRole('dialog', { name: 'Make your lab connection private' });
    expect(two.crew.updateConnection).not.toHaveBeenCalled();
    const institution = within(ask).getByPlaceholderText('For example, ucsf or sdsc');
    fireEvent.change(institution, { target: { value: 'sdsc' } });
    await act(async () => {
      fireEvent.click(within(ask).getByRole('button', { name: 'Make private' }));
    });
    await waitFor(() =>
      expect(two.crew.updateConnection).toHaveBeenCalledWith(connection.id, {
        ...connectionUpdateBody(bare),
        mode: 'private',
        institution_id: 'sdsc',
      })
    );
  });

  it('offers to set the workspace institution from the connection, with its permanent confirmation', async () => {
    const snapshot = makeSnapshot();
    snapshot.workspace.institution_id = null;
    const { crew } = renderSettings({ tab: 'privacy' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'lab settings' });
    expect(dialog).toHaveTextContent(copy.notSet);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Set institution to ucsf…' }));
    const confirm = await screen.findByRole('dialog', {
      name: confirmCopy.setInstitution.title('lab', 'ucsf'),
    });
    await act(async () => {
      fireEvent.click(within(confirm).getByRole('button', { name: 'Set ucsf permanently' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'policy.set')).toEqual([{ mode: 'private', institution_id: 'ucsf' }])
    );
  });

  it('tells the host to add an institution to the connection first', async () => {
    const snapshot = makeSnapshot();
    snapshot.workspace.institution_id = null;
    renderSettings(
      { tab: 'privacy' },
      { snapshot, connections: [{ ...connection, institution_id: null }] }
    );
    expect(await screen.findByText(copy.institutionNeedsConnection)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Set institution to/ })).toBeNull();
    expect(alice.id).toBe('person-alice');
  });
});
