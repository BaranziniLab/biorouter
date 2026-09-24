import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import {
  CREW_INVITATION_INVALID,
  CREW_JOIN_CODE_MISMATCH,
  CREW_UNEXPECTED_RESPONSE,
  isStaleDaemon,
} from './errors';
import {
  claimJoin,
  CREW_CONNECTION_EXISTS,
  CREW_INVITATION_CONFLICT,
  CREW_NOT_CONNECTED,
  getInvitation,
  groupDeviceCode,
  joinStatus,
  previewInvitation,
  refusalConnectionId,
  saveFromInvitation,
  savedConnectionIds,
} from './join';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

const LINE = 'brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ';
const MESSAGE = `Join lab on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste this whole message.\n${LINE}`;

// The daemon's InvitationSummary: the core preview flattened, plus the invitation's own route.
const summary = {
  source: 'invitation',
  workspace_id: 'workspace-1',
  workspace_name: 'lab',
  workspace_label: 'lab',
  workspace_public_key: 'a'.repeat(64),
  workspace_key_fingerprint: '3f2a9c1e77b0d4e1',
  fingerprint: '3F2A 9C1E 77B0 D4E1',
  socket_path: '/tmp/crew-1000-abc/broker.sock',
  owner_uid: 1000,
  host_username: 'alice',
  host_display_name: 'Alice Chen',
  workspace_mode: 'private',
  workspace_institution_id: 'ucsf',
  server: 'hpc.ucsf.edu',
  username: 'bob',
  ssh_target: 'bob@hpc.ucsf.edu',
  port: 22,
  mode: 'private',
  institution_id: 'ucsf',
  mode_differs: false,
  name: 'lab',
  existing_connection_id: null,
  missing: [],
  ssh_host: 'hpc.ucsf.edu',
  ssh_port: 22,
  proxy_jump: null,
  invitee_username: 'bob',
};

async function failureOf(promise: Promise<unknown>): Promise<CrewHttpError> {
  const failure: unknown = await promise.then(
    () => undefined,
    (error: unknown) => error
  );
  if (!(failure instanceof CrewHttpError))
    throw new Error(`expected a CrewHttpError, got ${String(failure)}`);
  return failure;
}

describe('previewInvitation', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('asks for a preview with the pasted text and the joiner overrides', async () => {
    mocks.crewHttp.mockResolvedValue(summary);
    const signal = new AbortController().signal;

    await previewInvitation(
      MESSAGE,
      { username: 'blee', mode: 'public', advanced: { port: 2222 } },
      signal
    );

    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/from-invitation',
      'POST',
      {
        username: 'blee',
        mode: 'public',
        advanced: { port: 2222 },
        invitation: MESSAGE,
        preview: true,
      },
      signal
    );
  });

  it('returns the display summary, bare or inside a preview envelope', async () => {
    const expected = {
      workspace_id: 'workspace-1',
      workspace_name: 'lab',
      workspace_public_key: 'a'.repeat(64),
      workspace_key_fingerprint: '3f2a9c1e77b0d4e1',
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
    mocks.crewHttp.mockResolvedValueOnce(summary);
    await expect(previewInvitation(LINE)).resolves.toEqual(expected);

    mocks.crewHttp.mockResolvedValueOnce({ preview: summary });
    await expect(previewInvitation(LINE)).resolves.toEqual(expected);
  });

  it('reads the fields an older status paste lacks as null', async () => {
    mocks.crewHttp.mockResolvedValue({
      workspace_id: 'workspace-1',
      workspace_public_key: 'a'.repeat(64),
      workspace_key_fingerprint: '3f2a9c1e77b0d4e1',
      mode: 'shared',
      ssh_port: -1,
      host_display_name: '   ',
    });

    await expect(previewInvitation('{"workspace_id":"workspace-1"}')).resolves.toEqual({
      workspace_id: 'workspace-1',
      workspace_name: null,
      workspace_public_key: 'a'.repeat(64),
      workspace_key_fingerprint: '3f2a9c1e77b0d4e1',
      fingerprint: null,
      host_username: null,
      host_display_name: null,
      workspace_mode: null,
      workspace_institution_id: null,
      mode: null,
      institution_id: null,
      ssh_host: null,
      ssh_port: null,
      proxy_jump: null,
      invitee_username: null,
      socket_path: null,
      owner_uid: null,
      existing_connection_id: null,
      missing: [],
    });
  });

  it('keeps what the invitation states apart from the privacy saving would default to', async () => {
    // A legacy status paste states no privacy; the daemon plans a Private save anyway.
    mocks.crewHttp.mockResolvedValue({
      ...summary,
      source: 'legacy_status',
      workspace_mode: null,
      workspace_institution_id: null,
      mode: 'private',
      institution_id: null,
    });
    const preview = await previewInvitation(LINE);
    expect(preview.workspace_mode).toBeNull();
    expect(preview.workspace_institution_id).toBeNull();
    expect(preview.mode).toBe('private');

    mocks.crewHttp.mockResolvedValue({
      ...summary,
      workspace_mode: 'public',
      workspace_institution_id: null,
      mode: 'private',
      institution_id: 'ucsf',
    });
    await expect(previewInvitation(LINE)).resolves.toMatchObject({
      workspace_mode: 'public',
      workspace_institution_id: null,
      mode: 'private',
      institution_id: 'ucsf',
    });
  });

  it('reads the connection this computer already has and what saving still needs', async () => {
    mocks.crewHttp.mockResolvedValue({
      ...summary,
      existing_connection_id: 'conn-7',
      missing: ['server', 'institution', 'server', 'robot', 3],
    });
    await expect(previewInvitation(LINE)).resolves.toMatchObject({
      existing_connection_id: 'conn-7',
      missing: ['server', 'institution'],
    });

    mocks.crewHttp.mockResolvedValue({ ...summary, existing_connection_id: '', missing: 'all' });
    await expect(previewInvitation(LINE)).resolves.toMatchObject({
      existing_connection_id: null,
      missing: [],
    });
  });

  it('refuses a summary with no workspace', async () => {
    mocks.crewHttp.mockResolvedValue({ workspace_name: 'lab' });
    expect((await failureOf(previewInvitation(LINE))).code).toBe(CREW_UNEXPECTED_RESPONSE);
  });

  it('passes an invalid-invitation refusal through for the dialog to word', async () => {
    const refusal = new CrewHttpError('Not an invitation', 400, CREW_INVITATION_INVALID);
    mocks.crewHttp.mockRejectedValue(refusal);
    await expect(previewInvitation('hello')).rejects.toBe(refusal);
  });

  it('recognizes a daemon that predates joining by invitation', async () => {
    // The path collides with PATCH/DELETE /crew/connections/{id} on an older daemon.
    mocks.crewHttp.mockRejectedValueOnce(new CrewHttpError('Crew request failed (405)', 405));
    expect(isStaleDaemon(await failureOf(previewInvitation(LINE)))).toBe(true);
    mocks.crewHttp.mockResolvedValueOnce(null);
    expect(isStaleDaemon(await failureOf(previewInvitation(LINE)))).toBe(true);
  });
});

describe('saveFromInvitation', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('saves without the preview flag and returns the saved connection', async () => {
    const connection = { id: 'conn-1', name: 'lab', status: 'disconnected' };
    mocks.crewHttp.mockResolvedValueOnce(connection);

    await expect(
      saveFromInvitation(MESSAGE, { mode: 'private', institution_id: 'ucsf' })
    ).resolves.toEqual(connection);
    expect(mocks.crewHttp).toHaveBeenCalledWith('/connections/from-invitation', 'POST', {
      mode: 'private',
      institution_id: 'ucsf',
      invitation: MESSAGE,
    });

    mocks.crewHttp.mockResolvedValueOnce({ connection });
    await expect(saveFromInvitation(MESSAGE)).resolves.toEqual(connection);
  });

  it('refuses an answer that is not a saved connection', async () => {
    mocks.crewHttp.mockResolvedValue({ saved: true });
    expect((await failureOf(saveFromInvitation(MESSAGE))).code).toBe(CREW_UNEXPECTED_RESPONSE);
  });
});

describe('refusalConnectionId', () => {
  it('names the connection a duplicate or conflicting join concerns', () => {
    const exists = Object.assign(new CrewHttpError('Already saved.', 409, CREW_CONNECTION_EXISTS), {
      body: { code: CREW_CONNECTION_EXISTS, connection_id: 'conn-7' },
    });
    expect(refusalConnectionId(exists)).toBe('conn-7');
    const conflict = Object.assign(
      new CrewHttpError('Different key.', 409, CREW_INVITATION_CONFLICT),
      { connection_id: 'conn-8' }
    );
    expect(refusalConnectionId(conflict, 'conn-1')).toBe('conn-8');
  });

  it('falls back to the preview’s connection when the error carries no id', () => {
    const exists = new CrewHttpError('Already saved.', 409, CREW_CONNECTION_EXISTS);
    expect(refusalConnectionId(exists, 'conn-7')).toBe('conn-7');
    expect(refusalConnectionId(exists)).toBeNull();
  });

  it('names nothing for any other failure', () => {
    const other = Object.assign(new CrewHttpError('No.', 409, CREW_NOT_CONNECTED), {
      body: { connection_id: 'conn-7' },
    });
    expect(refusalConnectionId(other, 'conn-7')).toBeNull();
    expect(refusalConnectionId(new Error('boom'), 'conn-7')).toBeNull();
  });
});

describe('savedConnectionIds', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('lists the ids the daemon has saved', async () => {
    mocks.crewHttp.mockResolvedValue({
      connections: [{ id: 'conn-1' }, { id: '' }, { name: 'no id' }, 'junk', { id: 'conn-2' }],
    });
    await expect(savedConnectionIds()).resolves.toEqual(['conn-1', 'conn-2']);
    expect(mocks.crewHttp).toHaveBeenCalledWith('/connections', 'GET', undefined, undefined);
  });

  it('refuses a list it cannot read rather than answering "none"', async () => {
    mocks.crewHttp.mockResolvedValue({ connections: 'many' });
    expect((await failureOf(savedConnectionIds())).code).toBe(CREW_UNEXPECTED_RESPONSE);
  });
});

describe('getInvitation', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('asks for the invitation with the invitee as a query parameter', async () => {
    mocks.crewHttp.mockResolvedValue({ message: MESSAGE, line: LINE });

    await expect(getInvitation('conn 1', '@bob lee')).resolves.toEqual({
      message: MESSAGE,
      line: LINE,
    });
    expect(mocks.crewHttp).toHaveBeenLastCalledWith(
      '/connections/conn%201/invitation?invitee=%40bob+lee',
      'GET',
      undefined,
      undefined
    );

    await getInvitation('conn-1');
    expect(mocks.crewHttp).toHaveBeenLastCalledWith(
      '/connections/conn-1/invitation',
      'GET',
      undefined,
      undefined
    );
  });

  it.each([
    ['a missing line', { message: MESSAGE }],
    ['a line that is not an invitation', { message: 'Join lab', line: 'Join lab' }],
    ['a message without its line', { message: 'Join lab on Crew.', line: LINE }],
  ])('refuses %s', async (_label, answer) => {
    mocks.crewHttp.mockResolvedValue(answer);
    expect((await failureOf(getInvitation('conn-1'))).code).toBe(CREW_UNEXPECTED_RESPONSE);
  });

  it('recognizes a daemon that predates invitations', async () => {
    mocks.crewHttp.mockRejectedValueOnce(new CrewHttpError('Crew request failed (404)', 404));
    expect(isStaleDaemon(await failureOf(getInvitation('conn-1')))).toBe(true);
    mocks.crewHttp.mockResolvedValueOnce(null);
    expect(isStaleDaemon(await failureOf(getInvitation('conn-1')))).toBe(true);
  });
});

describe('joinStatus', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('reads an invitation with this computer’s code in canonical form', async () => {
    const signal = new AbortController().signal;
    mocks.crewHttp.mockResolvedValue({
      status: 'invited',
      code: '7qk2-m9xa-3jtp-wz4d',
      inviter: { username: 'alice', display_name: 'Alice Chen' },
      workspace_name: 'lab',
      expires_at: 1_700_086_400,
      add_device: false,
    });

    await expect(joinStatus('conn-1', signal)).resolves.toEqual({
      status: 'invited',
      code: '7QK2M9XA3JTPWZ4D',
      inviter: { username: 'alice', display_name: 'Alice Chen' },
      workspace_name: 'lab',
      expires_at: 1_700_086_400,
      add_device: false,
    });
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn-1/join',
      'GET',
      undefined,
      signal
    );
  });

  it.each(['approved', 'not_invited', 'expired', 'joined', 'unsupported'])(
    'reads %s without a code',
    async (status) => {
      mocks.crewHttp.mockResolvedValue({ status, inviter: { username: 'alice' } });
      await expect(joinStatus('conn-1')).resolves.toEqual({
        status,
        inviter: { username: 'alice' },
      });
    }
  );

  it('reads a code mismatch with the code to send again', async () => {
    mocks.crewHttp.mockResolvedValue({ status: 'code_mismatch', code: '7QK2M9XA3JTPWZ4D' });
    await expect(joinStatus('conn-1')).resolves.toEqual({
      status: 'code_mismatch',
      code: '7QK2M9XA3JTPWZ4D',
    });
  });

  it.each([
    ['an unknown state', { status: 'pending' }],
    ['no state', {}],
    ['an invitation without a code', { status: 'invited' }],
    ['a mismatch without a code', { status: 'code_mismatch', code: null }],
    ['a code that is too short', { status: 'invited', code: '7QK2-M9XA' }],
    ['a code with letters Crockford never emits', { status: 'invited', code: '7QK2M9XA3JTPWZ4U' }],
    ['a code on another state that is malformed', { status: 'approved', code: 'not a code' }],
  ])('refuses %s', async (_label, answer) => {
    mocks.crewHttp.mockResolvedValue(answer);
    expect((await failureOf(joinStatus('conn-1'))).code).toBe(CREW_UNEXPECTED_RESPONSE);
  });

  it('leaves out an inviter it cannot name', async () => {
    mocks.crewHttp.mockResolvedValue({
      status: 'not_invited',
      inviter: { display_name: 'Alice Chen' },
      workspace_name: 7,
    });
    await expect(joinStatus('conn-1')).resolves.toEqual({ status: 'not_invited' });
  });

  it('recognizes a daemon that predates joining by invitation', async () => {
    mocks.crewHttp.mockRejectedValueOnce(new CrewHttpError('Crew request failed (404)', 404));
    expect(isStaleDaemon(await failureOf(joinStatus('conn-1')))).toBe(true);
    mocks.crewHttp.mockResolvedValueOnce(null);
    expect(isStaleDaemon(await failureOf(joinStatus('conn-1')))).toBe(true);
  });
});

describe('claimJoin', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('posts with no body and returns who invited this computer, as JoinClaimed says', async () => {
    mocks.crewHttp.mockResolvedValue({
      joined: true,
      status: 'joined',
      inviter: { username: 'alice', display_name: 'Alice Chen' },
      workspace_name: 'lab',
      add_device: true,
    });

    await expect(claimJoin('conn-1')).resolves.toEqual({
      joined: true,
      inviter: { username: 'alice', display_name: 'Alice Chen' },
      workspace_name: 'lab',
      add_device: true,
    });
    expect(mocks.crewHttp).toHaveBeenCalledWith('/connections/conn-1/join', 'POST');
    expect(mocks.crewHttp.mock.calls[0]).toHaveLength(2);
  });

  it('leaves out an inviter or workspace it cannot read', async () => {
    mocks.crewHttp.mockResolvedValue({
      joined: true,
      status: 'joined',
      inviter: { display_name: 'Alice Chen' },
      workspace_name: 9,
      add_device: 'yes',
      // Never sent by the daemon; not read either.
      principal: { username: 'mallory' },
    });
    await expect(claimJoin('conn-1')).resolves.toEqual({ joined: true });
  });

  it('accepts a status-shaped success', async () => {
    mocks.crewHttp.mockResolvedValue({ status: 'joined' });
    await expect(claimJoin('conn-1')).resolves.toEqual({ joined: true });
  });

  it.each([
    ['joined: false', { joined: false }],
    ['another status', { status: 'invited' }],
  ])('refuses %s as a join', async (_label, answer) => {
    mocks.crewHttp.mockResolvedValue(answer);
    expect((await failureOf(claimJoin('conn-1'))).code).toBe(CREW_UNEXPECTED_RESPONSE);
  });

  it('passes a typed refusal through', async () => {
    const refusal = new CrewHttpError('Code mismatch', 409, CREW_JOIN_CODE_MISMATCH);
    mocks.crewHttp.mockRejectedValue(refusal);
    await expect(claimJoin('conn-1')).rejects.toBe(refusal);
  });
});

describe('groupDeviceCode', () => {
  it('groups a code in fours for display', () => {
    expect(groupDeviceCode('7QK2M9XA3JTPWZ4D')).toBe('7QK2-M9XA-3JTP-WZ4D');
    expect(groupDeviceCode('')).toBe('');
  });
});
