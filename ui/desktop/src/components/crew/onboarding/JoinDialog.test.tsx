import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { CREW_INVITATION_INVALID } from '../api/errors';
import { joinCopy } from './copy';
import { readJoinContext, resetJoinContextForTests } from './joinContext';
import {
  JoinDialog,
  serverLoginInvalid,
  SSH_LOGIN_PATTERN,
  WORKSPACE_KEY_PATTERN,
} from './JoinDialog';
import { fakeConnection, makeCrew, renderWithCrew, WORKSPACE_KEY } from './testCrew';

const mocks = vi.hoisted(() => ({
  previewInvitation: vi.fn(),
  saveFromInvitation: vi.fn(),
}));

vi.mock('../api/join', async () => {
  const actual = await vi.importActual<typeof import('../api/join')>('../api/join');
  return {
    ...actual,
    previewInvitation: mocks.previewInvitation,
    saveFromInvitation: mocks.saveFromInvitation,
  };
});

const LINE = 'brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ';
const MESSAGE = `Join lab on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste this whole message.\n${LINE}`;

const PREVIEW = {
  workspace_id: 'workspace-1',
  workspace_name: 'lab',
  workspace_public_key: WORKSPACE_KEY,
  workspace_key_fingerprint: '3f2a9c1e77b0d4e1' + '0'.repeat(48),
  host_username: 'alice',
  host_display_name: 'Alice Chen',
  mode: 'private' as const,
  institution_id: 'ucsf',
  ssh_host: 'hpc.ucsf.edu',
  ssh_port: 22,
  proxy_jump: null,
  invitee_username: 'bob',
  socket_path: '/tmp/crew-1000-abc/broker.sock',
  owner_uid: 1000,
};

function renderDialog(overrides = {}) {
  const crew = makeCrew({ ui: { dialog: { kind: 'join' }, pane: null }, ...overrides });
  return renderWithCrew(<JoinDialog />, crew);
}

async function paste(text = MESSAGE) {
  fireEvent.change(screen.getByLabelText(joinCopy.invitation), { target: { value: text } });
}

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeEach(() => {
  // Radix's switch measures itself; jsdom has no ResizeObserver.
  vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  mocks.previewInvitation.mockReset();
  mocks.saveFromInvitation.mockReset();
  resetJoinContextForTests();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('JoinDialog', () => {
  it('opens on one field and asks the daemon to read the pasted invitation', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();

    const field = screen.getByLabelText(joinCopy.invitation);
    expect(field).toHaveFocus();
    expect(field).toBeRequired();
    expect(screen.getByRole('button', { name: joinCopy.submitFallback })).toBeDisabled();

    await paste();
    await waitFor(() =>
      expect(mocks.previewInvitation).toHaveBeenCalledWith(MESSAGE, {}, expect.any(AbortSignal))
    );
  });

  it('fills the summary from the preview: host, server, privacy and fingerprint', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();

    const summary = await screen.findByTestId('crew-join-summary');
    expect(within(summary).getByText('lab')).toBeInTheDocument();
    expect(screen.getByTestId('crew-join-hosted-by')).toHaveTextContent(
      'Hosted by Alice Chen (@alice) on hpc.ucsf.edu'
    );
    expect(screen.getByTestId('crew-join-workspace-privacy')).toHaveTextContent('Private');
    expect(screen.getByTestId('crew-join-workspace-privacy')).toHaveTextContent('ucsf');
    expect(within(summary).getByText('3F2A 9C1E 77B0 D4E1')).toBeInTheDocument();
    expect(screen.getByLabelText(joinCopy.username('hpc.ucsf.edu'))).toHaveValue('bob');
    expect(screen.getByRole('button', { name: 'Join lab' })).toBeEnabled();
  });

  it('states the privacy the person joins with, and reveals the choice only on Change', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();

    const line = await screen.findByTestId('crew-join-as');
    expect(line).toHaveTextContent('You’ll join as');
    expect(line).toHaveTextContent('Private');
    expect(line).toHaveTextContent('ucsf');
    expect(screen.queryByRole('radio')).toBeNull();
    expect(screen.queryByTestId('crew-join-mismatch')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: joinCopy.change }));
    const institution = screen.getByPlaceholderText('For example, ucsf or sdsc');
    expect(institution).toHaveValue('ucsf');
    expect(institution).toBeRequired();
    expect(institution).toHaveAttribute('pattern', '[a-z0-9][a-z0-9_-]{0,63}');

    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
    expect(screen.getByTestId('crew-join-mismatch')).toHaveTextContent(
      'lab is Private for ucsf. Your connection will be Public.'
    );
    expect(screen.queryByPlaceholderText('For example, ucsf or sdsc')).toBeNull();

    // The institution survives a trip to Public and back.
    fireEvent.click(screen.getByRole('radio', { name: /^Private/ }));
    expect(screen.getByPlaceholderText('For example, ucsf or sdsc')).toHaveValue('ucsf');
  });

  it('asks for the institution up front when the invitation names none', async () => {
    mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, institution_id: null });
    renderDialog();
    await paste();
    expect(await screen.findByPlaceholderText('For example, ucsf or sdsc')).toBeRequired();
  });

  it('says so when the paste is not an invitation', async () => {
    mocks.previewInvitation.mockRejectedValue(
      new CrewHttpError('not an invitation', 400, CREW_INVITATION_INVALID)
    );
    renderDialog();
    await paste('hello');
    expect(await screen.findByText(joinCopy.invalid)).toBeInTheDocument();
    expect(screen.getByLabelText(joinCopy.invitation)).toHaveAttribute('aria-invalid', 'true');
  });

  it('falls back to manual details, with the restart hint, on an older background service', async () => {
    mocks.previewInvitation.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
    renderDialog();
    await paste();

    expect(await screen.findByText(joinCopy.staleDaemon)).toBeInTheDocument();
    const key = screen.getByLabelText(joinCopy.workspaceKey);
    expect(key).toHaveAttribute('pattern', WORKSPACE_KEY_PATTERN);
    expect(key).toBeRequired();
    expect(screen.getByLabelText(joinCopy.socketPath)).toBeRequired();
    expect(screen.getByLabelText(joinCopy.workspaceId)).toBeRequired();
    expect(screen.getByLabelText(joinCopy.hostUserId)).toBeRequired();
    expect(screen.queryByLabelText(joinCopy.invitation)).toBeNull();
  });

  it('keeps the optional settings behind Advanced with their defaults stated', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();
    await screen.findByTestId('crew-join-summary');
    expect(screen.getByText('Port 22 · your SSH settings')).toBeInTheDocument();
    expect(screen.queryByLabelText(joinCopy.identityFile)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    expect(screen.getByLabelText(joinCopy.identityFile)).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: joinCopy.remoteExecution })).toBeDisabled();
  });

  it('saves the connection as the invitation pins it, then selects and connects it', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    const saved = fakeConnection({ id: 'conn-new', status: 'disconnected' });
    mocks.saveFromInvitation.mockResolvedValue(saved);
    const view = renderDialog();
    await paste();
    fireEvent.click(await screen.findByRole('button', { name: 'Join lab' }));

    await waitFor(() =>
      expect(mocks.saveFromInvitation).toHaveBeenCalledWith(MESSAGE, {
        mode: 'private',
        institution_id: 'ucsf',
        username: 'bob',
      })
    );
    const crew = view.crew();
    await waitFor(() => expect(crew.selectConnection).toHaveBeenCalledWith('conn-new'));
    expect(crew.refresh).toHaveBeenCalled();
    expect(readJoinContext('conn-new')).toMatchObject({
      hostUsername: 'alice',
      hostDisplayName: 'Alice Chen',
      workspaceName: 'lab',
      joining: true,
    });
    expect(crew.connect).not.toHaveBeenCalled();
    // No server login override: the saved connection is used as the invitation route wrote it.
    expect(crew.updateConnection).not.toHaveBeenCalled();

    // The controller has selected the saved connection and lists it: now connect, as the person.
    view.update({ connectionId: 'conn-new', connections: [saved], connection: saved });
    await waitFor(() => expect(crew.connect).toHaveBeenCalledWith({ userInitiated: true }));
    await waitFor(() => expect(crew.closeDialog).toHaveBeenCalled());
  });

  it('applies a server login override with the ordinary update, never through the invitation route', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    const saved = fakeConnection({ id: 'conn-new', status: 'disconnected' });
    mocks.saveFromInvitation.mockResolvedValue(saved);
    const updated = { ...saved, ssh_target: 'hpc' };
    const updateConnection = vi.fn().mockResolvedValue(updated);
    const view = renderDialog({ updateConnection });
    await paste();
    await screen.findByTestId('crew-join-summary');
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    const login = screen.getByLabelText(joinCopy.serverLogin);
    expect(login).toHaveAttribute('pattern', SSH_LOGIN_PATTERN);
    fireEvent.change(login, { target: { value: 'hpc' } });
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));

    // The invitation route gets only what its contract names; the override is not in `advanced`.
    await waitFor(() =>
      expect(mocks.saveFromInvitation).toHaveBeenCalledWith(MESSAGE, {
        mode: 'private',
        institution_id: 'ucsf',
        username: 'bob',
      })
    );
    const crew = view.crew();
    // The full body, pins unchanged, with only the server login replaced.
    await waitFor(() =>
      expect(updateConnection).toHaveBeenCalledWith('conn-new', {
        name: saved.name,
        ssh_target: 'hpc',
        port: saved.port,
        identity_file: saved.identity_file,
        proxy_jump: saved.proxy_jump,
        socket_path: saved.socket_path,
        owner_uid: saved.owner_uid,
        workspace_id: saved.workspace_id,
        workspace_public_key: saved.workspace_public_key,
        cluster_connection_id: saved.cluster_connection_id,
        remote_root: saved.remote_root,
        remote_execution: saved.remote_execution,
        mode: 'private',
        institution_id: 'ucsf',
      })
    );
    await waitFor(() => expect(crew.selectConnection).toHaveBeenCalledWith('conn-new'));
    expect(crew.removeConnection).not.toHaveBeenCalled();
  });

  it('removes the just-saved connection when the server login cannot be applied', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    const saved = fakeConnection({ id: 'conn-new', status: 'disconnected' });
    mocks.saveFromInvitation.mockResolvedValue(saved);
    const updateConnection = vi
      .fn()
      .mockRejectedValue(new CrewHttpError('Crew request failed (500)', 500));
    const removeConnection = vi.fn().mockResolvedValue(undefined);
    const view = renderDialog({ updateConnection, removeConnection });
    await paste();
    await screen.findByTestId('crew-join-summary');
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    fireEvent.change(screen.getByLabelText(joinCopy.serverLogin), { target: { value: 'hpc' } });
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));

    await waitFor(() => expect(removeConnection).toHaveBeenCalledWith('conn-new'));
    const crew = view.crew();
    // Nothing is selected or connected as someone the person did not choose; Join can be retried.
    await waitFor(() => expect(screen.getByRole('button', { name: 'Join lab' })).toBeEnabled());
    expect(crew.selectConnection).not.toHaveBeenCalled();
    expect(crew.connect).not.toHaveBeenCalled();
    expect(readJoinContext('conn-new')).not.toMatchObject({ joining: true });
  });

  it('refuses a server login the daemon would refuse, before anything is saved', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();
    await screen.findByTestId('crew-join-summary');
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    fireEvent.change(screen.getByLabelText(joinCopy.serverLogin), {
      target: { value: '-oProxyCommand=sh' },
    });
    // Close Advanced: the submit opens it again and reports the field.
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    expect(screen.queryByLabelText(joinCopy.serverLogin)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));
    const login = await screen.findByLabelText(joinCopy.serverLogin);
    expect(login).toHaveValue('-oProxyCommand=sh');
    expect((login as HTMLInputElement).validity.patternMismatch).toBe(true);
    expect(mocks.saveFromInvitation).not.toHaveBeenCalled();
  });

  it('matches the daemon’s SSH target rule in both the attribute and the check', () => {
    const attribute = new RegExp(`^(?:${SSH_LOGIN_PATTERN})$`, 'v');
    for (const [value, valid] of [
      ['hpc', true],
      ['bob@hpc.ucsf.edu', true],
      ['hpc-login:22', true],
      ['-oProxyCommand=sh', false],
      ['a b', false],
      ['a;b', false],
      ['$(id)', false],
    ] as const) {
      expect(attribute.test(value)).toBe(valid);
      expect(serverLoginInvalid(value)).toBe(!valid);
    }
    expect(serverLoginInvalid('')).toBe(false);
  });

  it('shows a save failure in the dialog, once', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog({
      error: { message: 'Crew request failed (500)', source: 'dialog:join' },
      errorSlotFor: (source: string) => source === 'dialog:join',
    });
    await paste();
    expect(await screen.findAllByText('Crew request failed (500)')).toHaveLength(1);
    expect(screen.getByRole('alert')).toHaveTextContent('Crew request failed (500)');
  });
});
