import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import * as React from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { CrewHttpError } from '../crewApi';
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

/** The OS the preload reports, which the dialog judges a local path by (W2-UIW-13). */
function onPlatform(platform: 'win32' | 'darwin' | 'linux') {
  Object.defineProperty(window, 'electron', {
    value: { platform },
    configurable: true,
    writable: true,
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(window, 'electron');
});

describe('ConnectionSettingsDialog', () => {
  it('is titled as an edit, saves with Save connection, and focuses its first field', async () => {
    renderSettings();
    expect(await screen.findByRole('dialog', { name: 'Connection settings' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save connection' })).toHaveAttribute(
      'type',
      'submit'
    );
    await waitFor(() => expect(screen.getByLabelText('Connection name')).toHaveFocus());
    // QA Q2-30: a workspace name is not a word to correct.
    expect(screen.getByLabelText('Connection name')).toHaveAttribute('spellcheck', 'false');
  });

  it('opens with the caret at the end of the name, not the whole name selected (QA Q3-43)', async () => {
    renderSettings({ name: 'chen-lab' });
    const name = (await screen.findByLabelText('Connection name')) as HTMLInputElement;
    await waitFor(() => expect(name).toHaveFocus());
    // Selected, the first key typed replaced the whole name.
    expect(name.selectionStart).toBe('chen-lab'.length);
    expect(name.selectionEnd).toBe('chen-lab'.length);
  });

  it('keeps focus on the name, caret at the end, when a menu opened it with the pointer (QA Q4-33)', async () => {
    const saved = { ...connection, name: 'chen-lab' };
    function OpenFromMenu() {
      const [open, setOpen] = React.useState(false);
      return (
        <>
          <DropdownMenu>
            <DropdownMenuTrigger>Workspace</DropdownMenuTrigger>
            <DropdownMenuContent>
              <DropdownMenuItem onSelect={() => setOpen(true)}>
                Connection settings…
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          {open ? (
            <ConnectionSettingsDialog connectionId={saved.id} onClose={() => setOpen(false)} />
          ) : null}
        </>
      );
    }
    const user = userEvent.setup();
    renderWithCrew(<OpenFromMenu />, { connections: [saved] });
    await user.click(screen.getByRole('button', { name: 'Workspace' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Connection settings…' }));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    const name = (await screen.findByLabelText('Connection name')) as HTMLInputElement;
    await waitFor(() => expect(name).toHaveFocus());
    // The closing menu drops focus to <body> a moment later in the app (Carol R4-3): the dialog
    // takes it back, and the caret is still where the person expects it.
    act(() => name.blur());
    expect(document.activeElement).toBe(document.body);
    await waitFor(() => expect(name).toHaveFocus());
    expect(name.selectionStart).toBe('chen-lab'.length);
    expect(name.selectionEnd).toBe('chen-lab'.length);
  });

  it('keeps the saved login and says what the person’s SSH settings call its server (QA Q3-39)', async () => {
    renderSettings({
      ssh_target: 'crew_alice@52.33.141.141',
      server_label: 'lab-server',
    } as Partial<typeof connection>);
    const login = await screen.findByLabelText(connectionSettingsCopy.login);
    // The real target stays: it is what connects.
    expect(login).toHaveValue('crew_alice@52.33.141.141');
    expect(login).toHaveAccessibleDescription(connectionSettingsCopy.loginAlias('lab-server'));
    expect(connectionSettingsCopy.loginAlias('lab-server')).toBe(
      'Your SSH settings call this server lab-server.'
    );
    // The alias names the saved server, so it goes once the field names another.
    fireEvent.change(login, { target: { value: 'crew_alice@hpc.example.edu' } });
    expect(screen.queryByText(connectionSettingsCopy.loginAlias('lab-server'))).toBeNull();
  });

  it('adds no alias line when the label is only the host', async () => {
    renderSettings({ server_label: 'hpc.example.edu' } as Partial<typeof connection>);
    const login = await screen.findByLabelText(connectionSettingsCopy.login);
    expect(login).toHaveValue('alice@hpc.example.edu');
    expect(screen.queryByText(/Your SSH settings call this server/)).toBeNull();
    expect(login).not.toHaveAttribute('aria-describedby');
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

    const confirm = await screen.findByRole('alertdialog', {
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
        screen.queryByRole('alertdialog', { name: confirmCopy.makeConnectionPublic.title('lab') })
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
    const confirm = await screen.findByRole('alertdialog', {
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

  // DW-04: "Identity file must be an absolute path" rendered only as the dialog's one note, at the
  // end of the scroll body, 140px below the view; the field was not marked, and nothing moved.
  it('says a relative or ~ identity file under its field before sending, and marks it invalid', async () => {
    onPlatform('linux');
    const { crew } = renderSettings({ identity_file: '/home/alice/.ssh/id_ed25519' });
    const field = await screen.findByLabelText(connectionSettingsCopy.identityFile);
    fireEvent.change(field, { target: { value: '~/.ssh/id_ed25519' } });
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(field).toHaveAccessibleDescription(connectionSettingsCopy.identityFileAbsolute('posix'));
    expect(field).toBeInvalid();
    save();
    expect(crew.updateConnection).not.toHaveBeenCalled();
    fireEvent.change(field, { target: { value: '/home/alice/.ssh/id_rsa' } });
    expect(field).not.toHaveAttribute('aria-invalid');
    expect(field).toBeValid();
  });

  // W2-UIW-13: the Identity file is a path on THIS computer, and the daemon judges it by this
  // computer's OS (`Path::is_absolute`). A check by a leading / alone refused every Windows path,
  // and a Windows person with one saved saw the error on opening and could save nothing at all.
  it('takes a saved Windows identity file on Windows: no error, and other settings save', async () => {
    onPlatform('win32');
    const saved = 'C:\\Users\\alice\\.ssh\\id_ed25519';
    const { crew } = renderSettings({ identity_file: saved });
    const field = await screen.findByLabelText(connectionSettingsCopy.identityFile);
    expect(field).toHaveValue(saved);
    expect(field).not.toHaveAttribute('aria-invalid');
    expect(field).toBeValid();
    expect(field).not.toHaveAccessibleDescription();
    fireEvent.change(screen.getByLabelText('Connection name'), {
      target: { value: 'Imaging core' },
    });
    save();
    await waitFor(() => expect(crew.updateConnection).toHaveBeenCalledTimes(1));
    expect(crew.updateConnection.mock.calls[0][1]).toMatchObject({
      name: 'Imaging core',
      identity_file: saved,
    });
  });

  it('takes a typed Windows path on Windows, drive or share, and says a Windows shape otherwise', async () => {
    onPlatform('win32');
    const { crew } = renderSettings({ identity_file: 'C:\\Users\\alice\\.ssh\\id_ed25519' });
    const field = await screen.findByLabelText(connectionSettingsCopy.identityFile);
    expect(field).toHaveAttribute(
      'placeholder',
      connectionSettingsCopy.identityFilePlaceholder('windows')
    );
    for (const accepted of ['C:/Users/alice/.ssh/id_rsa', '\\\\fileserver\\home\\alice\\id']) {
      fireEvent.change(field, { target: { value: accepted } });
      expect(field).not.toHaveAttribute('aria-invalid');
      expect(field).toBeValid();
    }
    // Not absolute on Windows, so the daemon there would refuse them.
    for (const refused of ['/home/alice/.ssh/id_rsa', '~\\.ssh\\id_rsa', 'id_rsa']) {
      fireEvent.change(field, { target: { value: refused } });
      expect(field).toHaveAttribute('aria-invalid', 'true');
      expect(field).toHaveAccessibleDescription(
        connectionSettingsCopy.identityFileAbsolute('windows')
      );
      expect(field).toBeInvalid();
    }
    expect(connectionSettingsCopy.identityFileAbsolute('windows')).not.toMatch(/starting with \//);
    save();
    expect(crew.updateConnection).not.toHaveBeenCalled();
  });

  it('asks a Mac for a path starting with /, and never suggests a ~ path', async () => {
    onPlatform('darwin');
    renderSettings({ identity_file: '/Users/alice/.ssh/id_ed25519' });
    const field = await screen.findByLabelText(connectionSettingsCopy.identityFile);
    expect(field).toHaveAttribute('placeholder', '/Users/you/.ssh/id_ed25519');
    fireEvent.change(field, { target: { value: 'C:\\Users\\alice\\.ssh\\id_rsa' } });
    expect(field).toHaveAccessibleDescription(connectionSettingsCopy.identityFileAbsolute('mac'));
    for (const platform of ['windows', 'mac', 'posix', 'unknown'] as const) {
      expect(connectionSettingsCopy.identityFilePlaceholder(platform)).not.toMatch(/^~/);
    }
  });

  // What the daemon accepted when it was saved is never refused on opening: a rule this side gets
  // wrong must not lock a person out of every other setting.
  it('never refuses the saved identity file itself, only what is typed since', async () => {
    onPlatform('darwin');
    const saved = 'D:\\keys\\lab';
    const { crew } = renderSettings({ identity_file: saved });
    const field = await screen.findByLabelText(connectionSettingsCopy.identityFile);
    expect(field).not.toHaveAttribute('aria-invalid');
    expect(field).toBeValid();
    fireEvent.change(field, { target: { value: 'D:\\keys\\other' } });
    expect(field).toHaveAttribute('aria-invalid', 'true');
    fireEvent.change(field, { target: { value: ` ${saved} ` } });
    expect(field).not.toHaveAttribute('aria-invalid');
    save();
    await waitFor(() => expect(crew.updateConnection).toHaveBeenCalledTimes(1));
    expect(crew.updateConnection.mock.calls[0][1]).toMatchObject({ identity_file: saved });
  });

  it('opens Advanced and focuses a hidden identity file the save would send wrong', async () => {
    const { crew } = renderSettings({ identity_file: '/home/alice/.ssh/id_ed25519' });
    fireEvent.change(await screen.findByLabelText(connectionSettingsCopy.identityFile), {
      target: { value: 'id_ed25519' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    await waitFor(() =>
      expect(screen.queryByLabelText(connectionSettingsCopy.identityFile)).toBeNull()
    );
    save();
    const field = await screen.findByLabelText(connectionSettingsCopy.identityFile);
    await waitFor(() => expect(field).toHaveFocus());
    expect(crew.updateConnection).not.toHaveBeenCalled();
  });

  it('puts the daemon’s field refusal under its field, focused, instead of a note at the end', async () => {
    const { crew } = renderSettings();
    crew.updateConnection.mockRejectedValueOnce(
      new Error('Daemon returned 400: Invalid ProxyJump route')
    );
    save();
    const field = await screen.findByLabelText(connectionSettingsCopy.jumpHosts);
    await waitFor(() => expect(field).toHaveFocus());
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(field).toHaveAccessibleDescription(connectionSettingsCopy.jumpHostsInvalid);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.queryByText(/ProxyJump/)).toBeNull();
    // Advanced stays closed once the person closes it, refusal or not.
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    await waitFor(() =>
      expect(screen.queryByLabelText(connectionSettingsCopy.jumpHosts)).toBeNull()
    );
    await act(async () => {});
    expect(screen.queryByLabelText(connectionSettingsCopy.jumpHosts)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    // Editing the value takes the refusal away.
    const again = await screen.findByLabelText(connectionSettingsCopy.jumpHosts);
    fireEvent.change(again, { target: { value: 'gateway' } });
    await waitFor(() => expect(again).not.toHaveAttribute('aria-invalid'));
  });

  // SF-F4 (a): "Crew aliases have different institutions; use a separately verified cluster
  // connection" for a person with one connection.
  it('says an institution refusal under Institution, naming both institutions', async () => {
    const { crew } = renderSettings();
    crew.updateConnection.mockRejectedValueOnce(
      new Error(
        'Daemon returned 400: Crew aliases have different institutions; use a separately verified cluster connection'
      )
    );
    fireEvent.change(await screen.findByLabelText('Institution'), {
      target: { value: 'stanford' },
    });
    save();
    const text = connectionSettingsCopy.institutionMismatch('stanford', 'lab', 'ucsf');
    expect(text).toBe('This connection is for stanford, but lab belongs to ucsf. Use ucsf here.');
    const field = screen.getByLabelText('Institution');
    await waitFor(() => expect(field).toHaveAccessibleDescription(text));
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(screen.queryByText(/aliases|cluster/)).toBeNull();
  });

  it('keeps a coded institution refusal’s own sentence', async () => {
    const { crew } = renderSettings();
    const sentence = 'This connection is for stanford, but lab belongs to ucsf.';
    crew.updateConnection.mockRejectedValueOnce(
      new CrewHttpError(sentence, 400, 'crew_institution_mismatch')
    );
    save();
    await waitFor(() =>
      expect(screen.getByLabelText('Institution')).toHaveAccessibleDescription(sentence)
    );
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
    // HPC-N1: what a command there cannot do, said before the person turns it on.
    expect(execution).toHaveAccessibleDescription(connectionSettingsCopy.remoteExecutionHelp);
    expect(connectionSettingsCopy.remoteExecutionHelp).toMatch(/no network/);
    expect(connectionSettingsCopy.remoteExecutionHelp).toMatch(/sbatch/);
  });

  it('keeps Workspace details closed, even when Advanced opens by itself', async () => {
    // A remote folder opens Advanced (the normal setup for an agent that works there); the machine
    // IDs must not come with it (QA T-33).
    renderSettings({ remote_root: '/home/alice/project', remote_execution: true });
    expect(await screen.findByLabelText('Remote work folder')).toHaveValue('/home/alice/project');
    expect(screen.getByRole('button', { name: /^Workspace details/ })).toHaveAttribute(
      'aria-expanded',
      'false'
    );
    expect(screen.queryByRole('button', { name: 'Copy workspace ID' })).toBeNull();
    expect(document.body.textContent).not.toContain(connection.workspace_id);
  });

  it('shows the fingerprint and puts every machine ID behind a Copy button, never on screen', async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    renderSettings();
    fireEvent.click(await screen.findByRole('button', { name: /^Workspace details/ }));
    expect(await screen.findByText('9A2D B2E2 3F15 04CD')).toBeInTheDocument();
    for (const name of [
      'Copy workspace ID',
      'Copy fingerprint',
      'Copy socket path',
      'Copy device ID',
      'Copy cluster ID',
    ])
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    // Identity rule 6: the numeric host UID is not offered at all.
    expect(screen.queryByRole('button', { name: 'Copy host user ID' })).toBeNull();
    const dialog = screen.getByRole('dialog', { name: 'Connection settings' });
    for (const value of [
      connection.workspace_id,
      connection.socket_path,
      connection.device_id,
      connection.cluster_connection_id,
      connection.workspace_public_key,
    ])
      expect(dialog).not.toHaveTextContent(value);
    expect(screen.queryByRole('textbox', { name: 'Workspace ID' })).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy workspace ID' }));
    });
    expect(writeText).toHaveBeenCalledWith(connection.workspace_id);
    expect(await screen.findByText(connectionSettingsCopy.copied)).toBeInTheDocument();
    // The button keeps its name through the feedback.
    expect(screen.getByRole('button', { name: 'Copy workspace ID' })).toBeInTheDocument();
  });

  it('puts Remove on its own row, apart from Cancel and Save, which never wrap', async () => {
    renderSettings();
    const remove = await screen.findByRole('button', {
      name: connectionSettingsCopy.remove('lab'),
    });
    const save = screen.getByRole('button', { name: connectionSettingsCopy.save });
    const actions = save.parentElement!;
    expect(actions).toHaveClass('flex-nowrap');
    expect(actions).toContainElement(screen.getByRole('button', { name: 'Cancel' }));
    expect(actions).not.toContainElement(remove);
  });

  it('leaves an error another surface is showing alone when it opens a confirmation', async () => {
    const { crew } = renderSettings();
    act(() => crew.current().reportError('Crew updates stopped.', 'global'));
    fireEvent.click(
      await screen.findByRole('button', { name: connectionSettingsCopy.remove('lab') })
    );
    await screen.findByRole('alertdialog', { name: confirmCopy.removeConnection.title('lab') });
    expect(crew.current().error).toEqual({ message: 'Crew updates stopped.', source: 'global' });
  });

  it('confirms before removing the saved connection', async () => {
    // The harness's person hosts lab from this, the only computer of theirs it lists: removing it
    // ends the host controls, so the workspace's name is typed first (CLI-1).
    const { crew } = renderSettings();
    fireEvent.click(
      await screen.findByRole('button', { name: connectionSettingsCopy.remove('lab') })
    );
    const confirm = await screen.findByRole('alertdialog', {
      name: confirmCopy.removeConnection.title('lab'),
    });
    expect(crew.removeConnection).not.toHaveBeenCalled();
    const remove = within(confirm).getByRole('button', {
      name: confirmCopy.removeConnection.onlyHostConfirm,
    });
    expect(remove).toBeDisabled();
    fireEvent.change(within(confirm).getByRole('textbox'), { target: { value: 'lab' } });
    await act(async () => {
      fireEvent.click(remove);
    });
    await waitFor(() => expect(crew.removeConnection).toHaveBeenCalledWith('conn-1'));
  });
});
