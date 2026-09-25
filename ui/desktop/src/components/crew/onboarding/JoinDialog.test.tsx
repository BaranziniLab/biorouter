import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
  getProviders: vi.fn(),
  read: vi.fn(),
}));

// The configured providers publish the name "UCSF" for the ID `ucsf` (Q2-38).
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => ({ getProviders: mocks.getProviders, read: mocks.read }) };
});

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
  mocks.getProviders.mockReset().mockResolvedValue([
    {
      name: 'versa',
      is_configured: true,
      affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
    },
  ]);
  mocks.read.mockReset().mockResolvedValue(null);
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
    // Pinned at the top, so it grows downward as sections open instead of re-centring (Q2-26),
    // and carrying Crew's focused-field edge outside `.crew-app` (Q2-25).
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    expect(dialog).toHaveClass('crew-dialog');
    // One Cancel style across Crew's dialogs (Q2-26): secondary, not ghost.
    expect(screen.getByRole('button', { name: joinCopy.cancel })).toHaveClass(
      'bg-background-medium'
    );

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
    // The name a configured provider publishes for `ucsf`, not the lowercase ID (Q2-38).
    await waitFor(() =>
      expect(screen.getByTestId('crew-join-workspace-privacy')).toHaveTextContent('UCSF')
    );
    expect(screen.getByLabelText(joinCopy.username('hpc.ucsf.edu'))).toHaveValue('bob');
    expect(screen.getByRole('button', { name: 'Join lab' })).toBeEnabled();
  });

  it('folds the fingerprint away, uncopyable, and says it is not the code to send (Q2-04)', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();

    const summary = await screen.findByTestId('crew-join-summary');
    // Four groups of four is the join code's shape: nothing of it shows until asked for.
    expect(document.body.textContent).not.toContain('3F2A 9C1E 77B0 D4E1');
    expect(screen.queryByTestId('crew-join-fingerprint-helper')).toBeNull();
    const check = within(summary).getByRole('button', { name: joinCopy.fingerprintCheck });
    expect(check).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(check);

    const helper = screen.getByTestId('crew-join-fingerprint-helper');
    // "Hosted by Alice Chen (@alice)" named her in full; later sentences call her what the join
    // card does, not a bare handle (Q3-46).
    expect(helper).toHaveTextContent(
      'Fingerprint 3F2A 9C1E 77B0 D4E1. This isn’t the code you send; your code appears after you choose Join. To double-check the invitation, ask Alice to read theirs from Crew (their workspace menu shows it).'
    );
    // No Copy anywhere in the summary: the joiner never sends this.
    expect(within(summary).queryByRole('button', { name: /copy/i })).toBeNull();
    expect(helper).not.toHaveTextContent(/Check this matches/);
  });

  it('names the server by the person’s own SSH alias, and builds logins from the address (D-ALIAS)', async () => {
    mocks.previewInvitation.mockResolvedValue({
      ...PREVIEW,
      ssh_host: '52.33.141.141',
      server_label: 'lab-server',
    });
    renderDialog();
    await paste();

    await screen.findByTestId('crew-join-summary');
    expect(screen.getByTestId('crew-join-hosted-by')).toHaveTextContent(
      'Hosted by Alice Chen (@alice) on lab-server'
    );
    expect(screen.getByLabelText(joinCopy.username('lab-server'))).toHaveValue('bob');
    expect(screen.getByRole('button', { name: joinCopy.agentHeading('lab-server') })).toBeVisible();
    // The login the Advanced override replaces is written as it is saved: with the address.
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    expect(screen.getByLabelText(joinCopy.serverLogin)).toHaveAccessibleDescription(
      joinCopy.serverLoginHelper('bob@52.33.141.141')
    );
  });

  it('names the server by its address when the daemon found no alias', async () => {
    mocks.previewInvitation.mockResolvedValue({ ...PREVIEW, server_label: null });
    renderDialog();
    await paste();
    expect(await screen.findByTestId('crew-join-hosted-by')).toHaveTextContent('on hpc.ucsf.edu');
  });

  it('states the privacy the person joins with, and reveals the choice only on Change', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();

    const line = await screen.findByTestId('crew-join-as');
    // It names what the choice governs, the AI models (Q3-49: "Models" alone read as something
    // about the person), and the institution by its name (Q2-36, Q2-38).
    await waitFor(() =>
      expect(line).toHaveTextContent('AI models: private and UCSF-approved only·Change')
    );
    expect(line).not.toHaveTextContent(/join as/i);
    expect(screen.queryByRole('radio')).toBeNull();
    expect(screen.queryByTestId('crew-join-mismatch')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: joinCopy.change }));
    // Privacy is said once: the radio rows replace the line, and focus lands on the choice
    // rather than falling to the page when Change goes.
    expect(screen.queryByTestId('crew-join-as')).toBeNull();
    expect(screen.getByRole('radio', { name: /^Private/ })).toHaveFocus();
    const institution = screen.getByPlaceholderText('For example, ucsf or sdsc');
    expect(institution).toHaveValue('ucsf');
    expect(institution).toBeRequired();
    expect(institution).toHaveAttribute('pattern', INSTITUTION_ID_PATTERN);

    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
    // Public on a Private workspace states its consequence, in warning ink (Q2-36): only clauses
    // the daemon and broker enforce. A Private workspace still blocks public models, and a Public
    // connection limits nothing the person reads, so neither "can't use UCSF models" nor
    // "can't read Restricted channels" may appear.
    const note = screen.getByTestId('crew-join-public-consequence');
    expect(note).toHaveTextContent(
      'lab is Private, so nothing changes yet: your agent still uses only private and UCSF-approved models here. If Alice makes lab Public, public models could read its public-safe channels through your agent. Alice isn’t told what you chose.'
    );
    expect(note).not.toHaveTextContent(/Restricted/);
    expect(note).not.toHaveTextContent(/can’t use/);
    expect(screen.queryByTestId('crew-join-mismatch')).toBeNull();
    expect(screen.queryByPlaceholderText('For example, ucsf or sdsc')).toBeNull();

    // The institution survives a trip to Public and back.
    fireEvent.click(screen.getByRole('radio', { name: /^Private/ }));
    expect(screen.getByPlaceholderText('For example, ucsf or sdsc')).toHaveValue('ucsf');
    expect(screen.queryByTestId('crew-join-public-consequence')).toBeNull();
  });

  it('folds the privacy choice back to its line with Done, and hands focus to Change (Q2-36)', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();
    fireEvent.click(await screen.findByRole('button', { name: joinCopy.change }));
    fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));

    fireEvent.click(screen.getByRole('button', { name: joinCopy.privacyDone }));
    expect(screen.queryByRole('radio')).toBeNull();
    expect(screen.getByTestId('crew-join-as')).toHaveTextContent(
      joinCopy.privacyLine('public', null)
    );
    // Done left with the fields: focus is on the Change that replaced them, not on the page.
    expect(screen.getByRole('button', { name: joinCopy.change })).toHaveFocus();
    // The consequence stays said while the choice stands.
    expect(screen.getByTestId('crew-join-public-consequence')).toBeInTheDocument();
  });

  it('offers no Done while the line could not state the choice (a Private with no institution)', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();
    fireEvent.click(await screen.findByRole('button', { name: joinCopy.change }));
    fireEvent.change(screen.getByPlaceholderText('For example, ucsf or sdsc'), {
      target: { value: '' },
    });
    expect(screen.queryByRole('button', { name: joinCopy.privacyDone })).toBeNull();
    fireEvent.change(screen.getByPlaceholderText('For example, ucsf or sdsc'), {
      target: { value: 'UCSF' },
    });
    expect(screen.queryByRole('button', { name: joinCopy.privacyDone })).toBeNull();
    fireEvent.change(screen.getByPlaceholderText('For example, ucsf or sdsc'), {
      target: { value: 'ucsf' },
    });
    expect(screen.getByRole('button', { name: joinCopy.privacyDone })).toBeInTheDocument();
  });

  it('keeps focus on the invitation box while the menu that opened the dialog closes (Q2-27)', async () => {
    renderDialog();
    const box = screen.getByLabelText(joinCopy.invitation);
    expect(box).toHaveFocus();
    // The menu, closing a moment later, drops focus to the page.
    act(() => box.blur());
    expect(document.activeElement).toBe(document.body);
    await waitFor(() => expect(box).toHaveFocus());
    // Once the person acts, their focus is theirs: nothing is pulled back.
    fireEvent.keyDown(document.body, { key: 'Tab' });
    act(() => box.blur());
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(box).not.toHaveFocus();
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

  it('keeps the SSH settings behind Advanced with their defaults stated', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();
    await screen.findByTestId('crew-join-summary');
    // Folded, Advanced says what it holds in plain words; the port is one of its fields (Q3-49).
    expect(screen.getByRole('button', { name: 'Advanced' })).toHaveAccessibleDescription(
      'server connection details'
    );
    expect(document.body.textContent).not.toMatch(/Port 22|SSH settings/);
    expect(screen.queryByLabelText(joinCopy.identityFile)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    expect(screen.getByLabelText(joinCopy.identityFile)).toBeInTheDocument();
    expect(screen.getByLabelText(joinCopy.port)).toHaveAttribute('placeholder', '22');
    // The agent's permission is not an SSH setting: it is not in Advanced (Q2-37).
    expect(screen.queryByRole('switch', { name: joinCopy.remoteExecution })).toBeNull();
    expect(screen.queryByLabelText(joinCopy.remoteFolder)).toBeNull();
  });

  it('gives the agent’s permissions their own labelled row, off by default (Q2-37)', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();
    await screen.findByTestId('crew-join-summary');
    const row = screen.getByRole('button', { name: joinCopy.agentHeading('hpc.ucsf.edu') });
    // Folded, it still says what it holds: off.
    expect(row).toHaveAccessibleDescription('No work folder · agent commands off');
    fireEvent.click(row);
    const agent = screen.getByRole('switch', { name: joinCopy.remoteExecution });
    expect(agent).not.toBeChecked();
    expect(agent).toBeDisabled();
    // A switch that is off limits says why (T-42).
    expect(agent).toHaveAccessibleDescription(joinCopy.remoteExecutionNeedsFolder);
    // The folder in plain words: where it is and what it looks like, not "an absolute path"
    // (Q3-49).
    const folder = screen.getByLabelText(joinCopy.remoteFolder);
    expect(folder).toHaveAccessibleDescription(
      'Optional. A folder on hpc.ucsf.edu, starting with /. Your agent can read and write files there.'
    );
    expect(document.body.textContent).not.toMatch(/absolute path/);
    fireEvent.change(folder, {
      target: { value: '/work/lab' },
    });
    expect(agent).toBeEnabled();
    expect(agent).not.toHaveAccessibleDescription(joinCopy.remoteExecutionNeedsFolder);
    fireEvent.click(agent);
    fireEvent.click(row);
    expect(row).toHaveAccessibleDescription('/work/lab · agent commands on');
  });

  it('opens the agent row to report a work folder it would refuse', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    renderDialog();
    await paste();
    await screen.findByTestId('crew-join-summary');
    const row = screen.getByRole('button', { name: joinCopy.agentHeading('hpc.ucsf.edu') });
    fireEvent.click(row);
    fireEvent.change(screen.getByLabelText(joinCopy.remoteFolder), {
      target: { value: 'work/lab' },
    });
    fireEvent.click(row);
    fireEvent.click(screen.getByRole('button', { name: 'Join lab' }));
    const folder = await screen.findByLabelText(joinCopy.remoteFolder);
    expect(await screen.findByText(joinCopy.remoteFolderInvalid)).toBeInTheDocument();
    expect(folder).toHaveAttribute('aria-invalid', 'true');
    expect(mocks.saveFromInvitation).not.toHaveBeenCalled();
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
    fireEvent.click(screen.getByRole('button', { name: joinCopy.agentHeading('hpc.ucsf.edu') }));
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

    const pick = screen.getByRole('radio', { name: /^Public/ });
    pick.focus();
    fireEvent.click(pick);
    // The choice stays on screen, where it can still be changed; it states itself, once.
    expect(screen.getByRole('radio', { name: /^Public/ })).toBeChecked();
    expect(screen.getByRole('radio', { name: /^Private/ })).not.toBeChecked();
    expect(screen.queryByTestId('crew-join-as')).toBeNull();
    // The same rows, now chosen: the first pick keeps focus on the radio it landed on rather than
    // swapping the rows for new ones and dropping focus to the page.
    expect(pick).toBeChecked();
    expect(pick).toHaveFocus();
    expect(screen.queryByTestId('crew-join-privacy-unchosen')).toBeNull();
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
    expect(screen.getByTestId('crew-join-as')).toHaveTextContent(
      joinCopy.privacyLine('public', null)
    );
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
  describe('the institution a Private join needs', () => {
    const NO_INSTITUTION: CrewInvitationPreview = {
      ...PREVIEW,
      workspace_institution_id: null,
      institution_id: null,
      missing: ['institution'],
    };
    const PLACEHOLDER = 'For example, ucsf or sdsc';

    it('keeps the field while it is typed in, key by key, and saves every letter (P0-4)', async () => {
      mocks.previewInvitation.mockResolvedValue(NO_INSTITUTION);
      mocks.saveFromInvitation.mockResolvedValue(fakeConnection({ id: 'conn-new' }));
      const user = userEvent.setup();
      renderDialog();
      await paste();

      const field = await screen.findByPlaceholderText(PLACEHOLDER);
      await user.click(field);
      await user.keyboard('ucsf');
      // The same node, never unmounted after the first letter, still focused, holding it all.
      expect(screen.getByPlaceholderText(PLACEHOLDER)).toBe(field);
      expect(field).toHaveValue('ucsf');
      expect(field).toHaveFocus();
      expect(screen.getByRole('radio', { name: /^Private/ })).toBeChecked();
      // The invitation named no institution, so "Private for ucsf" differs from nothing.
      expect(screen.queryByTestId('crew-join-mismatch')).toBeNull();

      await user.click(screen.getByRole('button', { name: 'Join lab' }));
      await waitFor(() =>
        expect(mocks.saveFromInvitation).toHaveBeenCalledWith(MESSAGE, {
          mode: 'private',
          institution_id: 'ucsf',
          username: 'bob',
        })
      );
    });

    it('keeps the choice on screen after choosing Public, and the typed value for Private', async () => {
      mocks.previewInvitation.mockResolvedValue(NO_INSTITUTION);
      const user = userEvent.setup();
      renderDialog();
      await paste();
      await user.type(await screen.findByPlaceholderText(PLACEHOLDER), 'ucsf');

      await user.click(screen.getByRole('radio', { name: /^Public/ }));
      expect(screen.getByRole('radio', { name: /^Public/ })).toBeChecked();
      expect(screen.getByRole('radio', { name: /^Private/ })).toBeInTheDocument();
      expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();

      await user.click(screen.getByRole('radio', { name: /^Private/ }));
      expect(screen.getByPlaceholderText(PLACEHOLDER)).toHaveValue('ucsf');
    });

    it('keeps the choice on screen when Public is chosen before anything is typed', async () => {
      mocks.previewInvitation.mockResolvedValue(NO_INSTITUTION);
      renderDialog();
      await paste();
      await screen.findByPlaceholderText(PLACEHOLDER);
      fireEvent.click(screen.getByRole('radio', { name: /^Public/ }));
      expect(screen.getByRole('radio', { name: /^Public/ })).toBeChecked();
      expect(screen.getByRole('radio', { name: /^Private/ })).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: joinCopy.change })).toBeNull();
    });

    it('marks it required, says whom to ask, and answers a blank submit under the field (T-11)', async () => {
      mocks.previewInvitation.mockResolvedValue(NO_INSTITUTION);
      const user = userEvent.setup();
      renderDialog();
      await paste();

      const field = await screen.findByPlaceholderText(PLACEHOLDER);
      expect(field).toBeRequired();
      expect(field).toHaveAttribute('aria-required', 'true');
      // Visibly marked, beside the label but outside it: the field's name stays "Institution".
      expect(screen.getByLabelText(joinCopy.institution)).toBe(field);
      expect(screen.getByText(joinCopy.institution).parentElement).toHaveTextContent(
        joinCopy.required
      );
      // The invitation carried no institution, so the helper says whom to ask, not to guess.
      const ask = joinCopy.institutionUnknown('Alice', 'lab');
      expect(screen.getByText(ask)).toBeInTheDocument();
      expect(field).toHaveAccessibleDescription(ask);

      // No native bubble: the form checks itself and answers under the field.
      expect(field.closest('form')).toHaveAttribute('novalidate');
      await user.click(screen.getByRole('button', { name: 'Join lab' }));
      const message = await screen.findByText(joinCopy.institutionRequired);
      expect(message).toHaveClass('text-supporting', 'text-text-danger');
      expect(field).toHaveAttribute('aria-invalid', 'true');
      expect(field.getAttribute('aria-describedby')?.split(' ')).toContain(message.id);
      expect(field).toHaveFocus();
      expect(mocks.saveFromInvitation).not.toHaveBeenCalled();

      // A value the pattern refuses says what the pattern wants; a good one clears it.
      await user.type(field, 'UCSF');
      expect(await screen.findByText(joinCopy.institutionInvalid)).toBeInTheDocument();
      expect(screen.queryByText(joinCopy.institutionRequired)).toBeNull();
      await user.clear(field);
      await user.type(field, 'ucsf');
      await waitFor(() => expect(screen.queryByText(joinCopy.institutionInvalid)).toBeNull());
      expect(field).not.toHaveAttribute('aria-invalid');
    });

    it('prefills the institution the invitation states and keeps the plain helper', async () => {
      mocks.previewInvitation.mockResolvedValue(PREVIEW);
      renderDialog();
      await paste();
      fireEvent.click(await screen.findByRole('button', { name: joinCopy.change }));
      const field = screen.getByPlaceholderText(PLACEHOLDER);
      expect(field).toHaveValue('ucsf');
      expect(field).toHaveAccessibleDescription(joinCopy.institutionHelper);
    });
  });

  it('folds a read invitation to "Invitation read · Edit", keeping focus, and Edit brings it back', async () => {
    mocks.previewInvitation.mockResolvedValue(PREVIEW);
    const user = userEvent.setup();
    renderDialog();
    expect(screen.getByLabelText(joinCopy.invitation)).toHaveFocus();
    await paste();

    const read = await screen.findByTestId('crew-join-invitation-read');
    expect(read).toHaveTextContent(joinCopy.invitationRead);
    // No base64 wall above the summary.
    expect(screen.queryByLabelText(joinCopy.invitation)).toBeNull();
    expect(document.body.textContent).not.toContain('brcrew1:');
    const edit = screen.getByRole('button', { name: joinCopy.editInvitationLabel });
    expect(edit).toHaveFocus();

    await user.click(edit);
    const box = screen.getByLabelText(joinCopy.invitation);
    expect(box).toHaveValue(MESSAGE);
    expect(box).toHaveFocus();
    // Typing in the box never folds it away under the person, even when what they type parses.
    await user.type(box, ' Thanks!');
    await waitFor(() =>
      expect(mocks.previewInvitation).toHaveBeenLastCalledWith(
        `${MESSAGE} Thanks!`,
        {},
        expect.any(AbortSignal)
      )
    );
    await screen.findByTestId('crew-join-summary');
    expect(screen.getByLabelText(joinCopy.invitation)).toHaveFocus();
    expect(screen.queryByTestId('crew-join-invitation-read')).toBeNull();
  });
});
