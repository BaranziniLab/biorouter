import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { ConnectionSettingsDialog } from './ConnectionSettingsDialog';
import { confirmCopy, connectionSettingsCopy } from './copy';
import { connection, installResizeObserverStub, renderWithCrew } from './dialogsTestHarness';

installResizeObserverStub();

const PLACEHOLDER = 'For example, ucsf or sdsc';

function renderSettings(overrides: Partial<typeof connection> = {}, onClose = vi.fn()) {
  const saved = { ...connection, ...overrides };
  const view = renderWithCrew(
    <ConnectionSettingsDialog connectionId={saved.id} onClose={onClose} />,
    {
      connections: [saved],
    }
  );
  return { ...view, saved, onClose };
}

function save() {
  fireEvent.click(screen.getByRole('button', { name: connectionSettingsCopy.save }));
}

afterEach(() => vi.restoreAllMocks());

describe('ConnectionSettingsDialog', () => {
  it('is titled as an edit, saves with Save connection, and focuses its first field', async () => {
    renderSettings();
    expect(await screen.findByRole('dialog', { name: 'Connection settings' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save connection' })).toHaveAttribute(
      'type',
      'submit'
    );
    await waitFor(() => expect(screen.getByLabelText('Connection name')).toHaveFocus());
  });

  it('is a real form: native required validation blocks the PATCH', async () => {
    const { crew } = renderSettings();
    const institution = await screen.findByPlaceholderText(PLACEHOLDER);
    expect(institution).toBeRequired();
    expect(institution.closest('form')).not.toHaveAttribute('novalidate');

    fireEvent.change(institution, { target: { value: '' } });
    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
    // Public hides the institution; it is not merely made optional.
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: /^Private/ }));

    const again = await screen.findByPlaceholderText(PLACEHOLDER);
    expect(again).toBeRequired();
    expect(again).toHaveValue('');
    save();
    expect(again).toBeInvalid();
    expect(crew.updateConnection).not.toHaveBeenCalled();
  });

  it('keeps the institution across mode toggles', async () => {
    renderSettings({ institution_id: null });
    fireEvent.change(await screen.findByPlaceholderText(PLACEHOLDER), {
      target: { value: 'sdsc' },
    });
    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
    fireEvent.click(screen.getByRole('radio', { name: /^Private/ }));
    expect(await screen.findByPlaceholderText(PLACEHOLDER)).toHaveValue('sdsc');
  });

  it('refuses an institution that is not a canonical ID, under the v-flag pattern browsers use', async () => {
    const { crew } = renderSettings();
    const institution = await screen.findByPlaceholderText(PLACEHOLDER);
    fireEvent.change(institution, { target: { value: 'UCSF' } });
    save();
    expect(institution).toBeInvalid();
    expect(await screen.findByText(connectionSettingsCopy.institutionPattern)).toBeInTheDocument();
    expect(crew.updateConnection).not.toHaveBeenCalled();
  });

  it('shows the institution helper only while the field is empty', async () => {
    renderSettings({ institution_id: null });
    const institution = await screen.findByPlaceholderText(PLACEHOLDER);
    expect(screen.getByText(connectionSettingsCopy.institutionHelper)).toBeInTheDocument();
    fireEvent.change(institution, { target: { value: 'ucsf' } });
    expect(screen.queryByText(connectionSettingsCopy.institutionHelper)).toBeNull();
  });

  it('asks for the typed workspace name before saving Private as Public, and Cancel sends nothing', async () => {
    const { crew } = renderSettings();
    fireEvent.click(await screen.findByRole('radio', { name: /^Public/ }));
    save();

    const confirm = await screen.findByRole('dialog', {
      name: confirmCopy.makeConnectionPublic.title('lab'),
    });
    expect(crew.updateConnection).not.toHaveBeenCalled();
    const makePublic = within(confirm).getByRole('button', { name: 'Make public' });
    expect(makePublic).toBeDisabled();
    await waitFor(() =>
      expect(within(confirm).getByRole('button', { name: 'Cancel' })).toHaveFocus()
    );

    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(
        screen.queryByRole('dialog', { name: confirmCopy.makeConnectionPublic.title('lab') })
      ).toBeNull()
    );
    expect(crew.updateConnection).not.toHaveBeenCalled();
    // Back in the settings, with the choice still made.
    expect(screen.getByRole('radio', { name: /^Public/ })).toBeChecked();
  });

  it('sends the whole record once the workspace name is typed', async () => {
    const { crew, saved, onClose } = renderSettings();
    fireEvent.click(await screen.findByRole('radio', { name: /^Public/ }));
    save();
    const confirm = await screen.findByRole('dialog', {
      name: confirmCopy.makeConnectionPublic.title('lab'),
    });
    fireEvent.change(within(confirm).getByLabelText('Type lab to confirm'), {
      target: { value: 'LAB' },
    });
    fireEvent.click(within(confirm).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(crew.updateConnection).toHaveBeenCalledTimes(1));
    expect(crew.updateConnection).toHaveBeenCalledWith(saved.id, {
      ...connectionUpdateBody(saved),
      port: 22,
      identity_file: undefined,
      proxy_jump: undefined,
      remote_root: undefined,
      remote_execution: false,
      mode: 'public',
      institution_id: 'ucsf',
    });
    // A privacy change is observed afresh.
    await waitFor(() => expect(crew.refresh).toHaveBeenCalled());
    expect(onClose).toHaveBeenCalled();
  });

  it('saves an ordinary edit straight away, with the full body', async () => {
    const { crew, saved, onClose } = renderSettings();
    fireEvent.change(await screen.findByLabelText('Connection name'), {
      target: { value: 'Imaging core' },
    });
    save();
    await waitFor(() => expect(crew.updateConnection).toHaveBeenCalledTimes(1));
    expect(crew.updateConnection.mock.calls[0][1]).toEqual({
      ...connectionUpdateBody(saved),
      name: 'Imaging core',
      port: 22,
      remote_execution: false,
      identity_file: undefined,
      proxy_jump: undefined,
      remote_root: undefined,
    });
    expect(crew.refresh).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it('shows a refused save in the dialog, once', async () => {
    const { crew } = renderSettings();
    crew.updateConnection.mockRejectedValueOnce(new Error('ssh_target is not valid'));
    save();
    expect(await screen.findAllByText('ssh_target is not valid')).toHaveLength(1);
    expect(screen.getByRole('alert')).toHaveTextContent('ssh_target is not valid');
  });

  it('opens Advanced by itself only when the record uses something inside it', async () => {
    renderSettings();
    await screen.findByLabelText('Connection name');
    expect(screen.queryByLabelText('Port')).toBeNull();
    expect(screen.getByText('Port 22 · your SSH settings')).toBeInTheDocument();
  });

  it('opens Advanced with the values when one is non-default', async () => {
    renderSettings({ port: 2222, proxy_jump: 'gateway.example.edu' });
    expect(await screen.findByLabelText('Port')).toHaveValue(2222);
    expect(screen.getByLabelText('Jump hosts')).toHaveValue('gateway.example.edu');
  });

  it('opens Advanced and focuses a hidden field the submit would refuse', async () => {
    const { crew } = renderSettings({ port: 2222 });
    fireEvent.change(await screen.findByLabelText('Port'), { target: { value: '70000' } });
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    await waitFor(() => expect(screen.queryByLabelText('Port')).toBeNull());

    save();
    const port = await screen.findByLabelText('Port');
    await waitFor(() => expect(port).toHaveFocus());
    expect(crew.updateConnection).not.toHaveBeenCalled();
  });

  it('keeps the agent-execution switch off until a remote folder is set', async () => {
    renderSettings({ port: 2222 });
    const execution = await screen.findByRole('switch', {
      name: /Let my agent run commands in this folder/,
    });
    expect(execution).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Remote work folder'), {
      target: { value: '/home/alice/project' },
    });
    expect(execution).toBeEnabled();
  });

  it('lists the pinned workspace identity read-only, with the fingerprint grouped', async () => {
    renderSettings({ port: 2222 });
    expect(await screen.findByText('9A2D B2E2 3F15 04CD')).toBeInTheDocument();
    for (const name of [
      'Copy workspace ID',
      'Copy fingerprint',
      'Copy socket path',
      'Copy host user ID',
      'Copy device ID',
      'Copy cluster ID',
    ])
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    expect(screen.queryByRole('textbox', { name: 'Workspace ID' })).toBeNull();
  });

  it('leaves an error another surface is showing alone when it opens a confirmation', async () => {
    const { crew } = renderSettings();
    act(() => crew.current().reportError('Crew updates stopped.', 'global'));
    fireEvent.click(
      await screen.findByRole('button', { name: connectionSettingsCopy.remove('lab') })
    );
    await screen.findByRole('dialog', { name: confirmCopy.removeConnection.title('lab') });
    expect(crew.current().error).toEqual({ message: 'Crew updates stopped.', source: 'global' });
  });

  it('confirms before removing the saved connection', async () => {
    const { crew } = renderSettings();
    fireEvent.click(
      await screen.findByRole('button', { name: connectionSettingsCopy.remove('lab') })
    );
    const confirm = await screen.findByRole('dialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    expect(crew.removeConnection).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(within(confirm).getByRole('button', { name: 'Remove' }));
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith('conn-1'));
  });
});
