import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { CREW_INVITATION_INVALID } from '../api/errors';
import type { CrewInvitationPreview } from '../api/join';
import { INSTITUTION_ID_PATTERN } from '../identity';
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
  savedConnectionIds: vi.fn(),
}));

vi.mock('../api/join', async () => {
  const actual = await vi.importActual<typeof import('../api/join')>('../api/join');
  return {
    ...actual,
    previewInvitation: mocks.previewInvitation,
    saveFromInvitation: mocks.saveFromInvitation,
    savedConnectionIds: mocks.savedConnectionIds,
  };
});

const LINE = 'brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ';
const MESSAGE = `Join lab on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste this whole message.\n${LINE}`;

/** What `previewInvitation` returns for an invitation that states its privacy. */
const PREVIEW: CrewInvitationPreview = {
  workspace_id: 'workspace-1',
  workspace_name: 'lab',
  workspace_public_key: WORKSPACE_KEY,
  workspace_key_fingerprint: '3f2a9c1e77b0d4e1' + '0'.repeat(48),
  fingerprint: '3F2A 9C1E 77B0 D4E1',
  host_username: 'alice',
  host_display_name: 'Alice Chen',
  workspace_mode: 'private',
  workspace_institution_id: 'ucsf',
  mode: 'private',
  institution_id: 'ucsf',
  ssh_host: 'hpc.ucsf.edu',
  ssh_port: 22,
  proxy_jump: null,
  invitee_username: 'bob',
  socket_path: '/tmp/crew-1000-abc/broker.sock',
  owner_uid: 1000,
  existing_connection_id: null,
  missing: [],
};

function renderDialog(overrides = {}) {
  const crew = makeCrew({ ui: { dialog: { kind: 'join' }, pane: null }, ...overrides });
  return renderWithCrew(<JoinDialog />, crew);
}

/** A 409 join refusal as `crewHttp` builds it from the daemon's body (`connection_id`). */
function refusal(message: string, code: string, connectionId: string) {
  return new CrewHttpError(message, 409, code, undefined, undefined, connectionId);
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
  // Nothing saved before the submit, unless a test says otherwise.
  mocks.savedConnectionIds.mockReset().mockResolvedValue([]);
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
    expect(institution).toHaveAttribute('pattern', INSTITUTION_ID_PATTERN);

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
    mocks.previewInvitation.mockResolvedValue({
      ...PREVIEW,
      workspace_institution_id: null,
      institution_id: null,
      missing: ['institution'],
    });
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

  it('sends the invitation route only its named overrides and applies the rest with the ordinary update', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    const saved = fakeConnection({ id: 'conn-new', status: 'disconnected', port: 2222 });
    mocks.saveFromInvitation.mockResolvedValue(saved);
    const updateConnection = vi.fn(async (_id: string, body: object) => ({ ...saved, ...body }));
    const view = renderDialog({ updateConnection });
    await paste();
    await screen.findByTestId('crew-join-summary');
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    const login = screen.getByLabelText(joinCopy.serverLogin);
    expect(login).toHaveAttribute('pattern', SSH_LOGIN_PATTERN);
    fireEvent.change(login, { target: { value: 'hpc' } });
    fireEvent.change(screen.getByLabelText(joinCopy.port), { target: { value: '2222' } });
    fireEvent.change(screen.getByLabelText(joinCopy.connectionName), {
      target: { value: 'UCSF lab' },
    });
    fireEvent.change(screen.getByLabelText(joinCopy.remoteFolder), {
      target: { value: '/work/lab' },
    });
    fireEvent.click(screen.getByRole('switch', { name: joinCopy.remoteExecution }));
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));

    // Port, identity file and jump host are what the route's contract names; nothing else rides it.
    await waitFor(() =>
      expect(mocks.saveFromInvitation).toHaveBeenCalledWith(MESSAGE, {
        mode: 'private',
        institution_id: 'ucsf',
        username: 'bob',
        advanced: { port: 2222 },
      })
    );
    const crew = view.crew();
    // The full body, pins unchanged, with the person's local choices applied over it.
    await waitFor(() =>
      expect(updateConnection).toHaveBeenCalledWith('conn-new', {
        name: 'UCSF lab',
        ssh_target: 'hpc',
        port: 2222,
        identity_file: saved.identity_file,
        proxy_jump: saved.proxy_jump,
        socket_path: saved.socket_path,
        owner_uid: saved.owner_uid,
        workspace_id: saved.workspace_id,
        workspace_public_key: saved.workspace_public_key,
        cluster_connection_id: saved.cluster_connection_id,
        remote_root: '/work/lab',
        remote_execution: true,
        mode: 'private',
        institution_id: 'ucsf',
      })
    );
    expect(updateConnection).toHaveBeenCalledOnce();
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

  it('never removes a connection that was already on this computer when its update fails', async () => {
    // The preview was read before the connection existed (another window saved it since), so the
    // dialog still offers Join; the daemon then answers the existing connection.
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    const existing = fakeConnection({ id: 'conn-old', status: 'connected' });
    mocks.savedConnectionIds.mockResolvedValue(['conn-old']);
    mocks.saveFromInvitation.mockResolvedValue(existing);
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

    const crew = view.crew();
    // Opened as it is: no update (which would disconnect it) and certainly no removal.
    await waitFor(() => expect(crew.selectConnection).toHaveBeenCalledWith('conn-old'));
    expect(updateConnection).not.toHaveBeenCalled();
    expect(removeConnection).not.toHaveBeenCalled();
    // What it remembers about its own join is left alone.
    expect(readJoinContext('conn-old')).not.toMatchObject({ joining: true });
  });

  it('neither updates nor removes a connection it cannot tell the save created', async () => {
    // The saved list can't be read, and the preview named no connection: the one the save returns
    // may have been saved by another window (or the CLI) meanwhile, possibly already joined.
    mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, existing_connection_id: null });
    mocks.savedConnectionIds.mockRejectedValue(new CrewHttpError('Crew request failed (500)', 500));
    const saved = fakeConnection({ id: 'conn-maybe', status: 'connected' });
    mocks.saveFromInvitation.mockResolvedValue(saved);
    const updateConnection = vi.fn(async (_id: string, body: object) => ({ ...saved, ...body }));
    const removeConnection = vi.fn().mockResolvedValue(undefined);
    const view = renderDialog({ updateConnection, removeConnection });
    await paste();
    await screen.findByTestId('crew-join-summary');
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    // A local choice that differs from what the save returned (`bob@hpc.ucsf.edu`).
    fireEvent.change(screen.getByLabelText(joinCopy.serverLogin), { target: { value: 'hpc' } });
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));

    const crew = view.crew();
    // Opened as it is: an update would rewrite its server login and disconnect it.
    await waitFor(() => expect(crew.selectConnection).toHaveBeenCalledWith('conn-maybe'));
    expect(updateConnection).not.toHaveBeenCalled();
    expect(removeConnection).not.toHaveBeenCalled();
  });

  it('leaves the join record of a connection the controller already lists when the saved list can’t be read', async () => {
    mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, existing_connection_id: null });
    mocks.savedConnectionIds.mockRejectedValue(new CrewHttpError('Crew request failed (500)', 500));
    const known = fakeConnection({ id: 'conn-known', status: 'connected' });
    mocks.saveFromInvitation.mockResolvedValue(known);
    const view = renderDialog({ connections: [known] });
    await paste();
    fireEvent.click(await screen.findByRole('button', { name: 'Join lab' }));

    const crew = view.crew();
    await waitFor(() => expect(crew.selectConnection).toHaveBeenCalledWith('conn-known'));
    expect(crew.updateConnection).not.toHaveBeenCalled();
    expect(crew.removeConnection).not.toHaveBeenCalled();
    expect(readJoinContext('conn-known')).not.toMatchObject({ joining: true });
  });

  it('offers the connection this computer already has instead of saving the invitation again', async () => {
    mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, existing_connection_id: 'conn-old' });
    const existing = fakeConnection({ id: 'conn-old', name: 'UCSF lab' });
    const view = renderDialog({ connections: [existing] });
    await paste();

    expect(await screen.findByTestId('crew-join-existing')).toHaveTextContent(
      joinCopy.existing('lab')
    );
    expect(screen.queryByRole('button', { name: 'Join lab' })).toBeNull();
    expect(screen.queryByTestId('crew-join-as')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: joinCopy.openExisting('UCSF lab') }));

    const crew = view.crew();
    await waitFor(() => expect(crew.selectConnection).toHaveBeenCalledWith('conn-old'));
    expect(crew.closeDialog).toHaveBeenCalled();
    expect(mocks.saveFromInvitation).not.toHaveBeenCalled();
    expect(crew.updateConnection).not.toHaveBeenCalled();
    expect(crew.removeConnection).not.toHaveBeenCalled();
  });

  it('offers to open the connection a refused paste concerns', async () => {
    mocks.previewInvitation.mockRejectedValue(
      refusal('Doesn’t match “UCSF lab”.', 'crew_invitation_conflict', 'conn-old')
    );
    const existing = fakeConnection({ id: 'conn-old', name: 'UCSF lab' });
    const view = renderDialog({ connections: [existing] });
    await paste();

    expect(await screen.findByText('Doesn’t match “UCSF lab”.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: joinCopy.openExisting('UCSF lab') }));
    await waitFor(() => expect(view.crew().selectConnection).toHaveBeenCalledWith('conn-old'));
  });

  it('offers to open the connection a refused save names', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    mocks.saveFromInvitation.mockRejectedValue(
      refusal('Already saved.', 'crew_connection_exists', 'conn-old')
    );
    const existing = fakeConnection({ id: 'conn-old', name: 'UCSF lab' });
    const view = renderDialog({
      connections: [existing],
      error: { message: 'Already saved.', source: 'dialog:join' },
      errorSlotFor: (source: string) => source === 'dialog:join',
    });
    await paste();
    fireEvent.click(await screen.findByRole('button', { name: 'Join lab' }));

    const open = await screen.findByRole('button', { name: joinCopy.openExisting('UCSF lab') });
    expect(screen.getByRole('alert')).toContainElement(open);
    fireEvent.click(open);
    await waitFor(() => expect(view.crew().selectConnection).toHaveBeenCalledWith('conn-old'));
  });

  it('offers to open a connection saved after the preview, named only by the save’s refusal', async () => {
    // The preview found nothing saved; another window saved the workspace before Join.
    mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, existing_connection_id: null });
    mocks.saveFromInvitation.mockRejectedValue(
      refusal('Already saved.', 'crew_connection_exists', 'conn-late')
    );
    const view = renderDialog({
      error: { message: 'Already saved.', source: 'dialog:join' },
      errorSlotFor: (source: string) => source === 'dialog:join',
    });
    await paste();
    fireEvent.click(await screen.findByRole('button', { name: 'Join lab' }));

    const open = await screen.findByRole('button', { name: joinCopy.openExisting('lab') });
    expect(screen.getByRole('alert')).toContainElement(open);
    fireEvent.click(open);
    const crew = view.crew();
    // Not listed yet: the list is reloaded before the connection the refusal named is opened.
    await waitFor(() => expect(crew.selectConnection).toHaveBeenCalledWith('conn-late'));
    expect(crew.refresh).toHaveBeenCalled();
    expect(crew.updateConnection).not.toHaveBeenCalled();
    expect(crew.removeConnection).not.toHaveBeenCalled();
  });

  it('never presents a defaulted Private as the workspace’s privacy, and saves only a choice', async () => {
    // An older status paste states no privacy; the daemon still plans a Private save.
    mocks.previewInvitation.mockResolvedValue({
      ...PREVIEW,
      workspace_mode: null,
      workspace_institution_id: null,
      mode: 'private',
      institution_id: null,
      missing: ['institution'],
    });
    mocks.saveFromInvitation.mockResolvedValue(fakeConnection({ id: 'conn-new' }));
    renderDialog();
    await paste();

    const stated = await screen.findByTestId('crew-join-workspace-privacy');
    expect(stated).toHaveTextContent(joinCopy.privacyUnstated);
    expect(stated).not.toHaveTextContent('Private');
    expect(screen.getByTestId('crew-join-as')).toHaveTextContent(joinCopy.privacyChoose);
    expect(screen.queryByTestId('crew-join-mismatch')).toBeNull();
    // The choice is open and nothing is chosen for the person.
    const radios = within(screen.getByTestId('crew-join-privacy-unchosen')).getAllByRole('radio');
    expect(radios).toHaveLength(2);
    for (const radio of radios) expect(radio).not.toBeChecked();
    const join = screen.getByRole('button', { name: 'Join lab' });
    expect(join).toBeDisabled();
    fireEvent.submit(join.closest('form') ?? document.body);
    expect(mocks.saveFromInvitation).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
    expect(screen.getByTestId('crew-join-as')).toHaveTextContent('You’ll join as');
    expect(screen.getByTestId('crew-join-as')).toHaveTextContent('Public');
    expect(screen.getByRole('radio', { name: /^Public/ })).toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));
    await waitFor(() =>
      expect(mocks.saveFromInvitation).toHaveBeenCalledWith(MESSAGE, {
        mode: 'public',
        institution_id: null,
        username: 'bob',
      })
    );
  });

  it('reads the privacy the invitation states, not the privacy saving would default to', async () => {
    // Contradictory on purpose: only `workspace_mode` is the workspace's word.
    mocks.previewInvitation.mockResolvedValue({
      ...PREVIEW,
      workspace_mode: 'public',
      workspace_institution_id: null,
      mode: 'private',
      institution_id: 'ucsf',
    });
    mocks.saveFromInvitation.mockResolvedValue(fakeConnection({ id: 'conn-new' }));
    renderDialog();
    await paste();

    const stated = await screen.findByTestId('crew-join-workspace-privacy');
    expect(stated).toHaveTextContent('Public');
    expect(stated).not.toHaveTextContent('Private');
    expect(screen.getByTestId('crew-join-as')).toHaveTextContent('Public');
    expect(screen.queryByTestId('crew-join-mismatch')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));
    await waitFor(() =>
      expect(mocks.saveFromInvitation).toHaveBeenCalledWith(MESSAGE, {
        mode: 'public',
        institution_id: null,
        username: 'bob',
      })
    );
  });

  it('asks for the server login up front when the invitation names no server', async () => {
    mocks.previewInvitation.mockResolvedValue({
      ...PREVIEW,
      ssh_host: null,
      missing: ['server'],
    });
    renderDialog();
    await paste();
    const login = await screen.findByLabelText(joinCopy.serverLogin);
    expect(login).toBeRequired();
    expect(screen.getByText(joinCopy.serverMissing)).toBeInTheDocument();
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
