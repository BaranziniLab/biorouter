import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { CREW_INVITATION_INVALID } from '../api/errors';
import { joinCopy } from './copy';
import { readJoinContext, resetJoinContextForTests } from './joinContext';
import { JoinDialog, WORKSPACE_KEY_PATTERN } from './JoinDialog';
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

    // The controller has selected the saved connection and lists it: now connect, as the person.
    view.update({ connectionId: 'conn-new', connections: [saved], connection: saved });
    await waitFor(() => expect(crew.connect).toHaveBeenCalledWith({ userInitiated: true }));
    await waitFor(() => expect(crew.closeDialog).toHaveBeenCalled());
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
