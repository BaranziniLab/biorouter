import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../../api/client.gen';
import {
  listSetupJobs,
  plainStage,
  plainStagesFor,
  resetSetupStartedHereForTests,
  setupRunFrom,
  setupStartedHere,
  setupWorkspacesFrom,
  startSetup,
  versionOlder,
} from './setup';

/**
 * Automatic setup's account-ownership half (AO-6, AO-8), on the wire: the server target, the
 * restart and start choices, the workspaces a job found, what it restarted, the jobs started
 * elsewhere, and the two new plain steps.
 */

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'setup-test-proof' }),
}));

const RUNNING = {
  job_id: 'job-1',
  intent: 'check',
  state: 'running',
  stage: 'check',
  stages: [{ name: 'check', state: 'running' }],
  server: { login: 'wanjun@leo.ucsf.edu', user: 'wanjun' },
  outcome: null,
  error: null,
};

const HOSTED = {
  workspace_id: '11111111-2222-4333-8444-555555555555',
  name: 'baranzini-lab',
  role: 'host',
  host_uid: 1000,
  host_username: 'wanjun',
  socket_path: '/tmp/crew-1000-abc/broker.sock',
  workspace_key_fingerprint: 'abcd',
  mode: 'private',
  institution_id: 'ucsf',
  state: 'running',
  version: '1.94.1',
  binary_current: false,
  state_dir: '/home/wanjun/.local/share/biorouter-crew/baranzini-lab',
  found_by: 'home',
  invitation: 'brcrew1:abc',
  others_connected: 2,
  saved_connection_id: null,
  this_computer: 'not_saved',
};

function answer(status: number, body: unknown) {
  const fetchMock = vi.fn().mockImplementation(
    async () =>
      new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
      })
  );
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

function sentBody(fetchMock: ReturnType<typeof vi.fn>, call = 0): unknown {
  const [, init] = fetchMock.mock.calls[call] as [string, RequestInit];
  return JSON.parse(String(init.body));
}

beforeEach(() => {
  vi.restoreAllMocks();
  resetSetupStartedHereForTests();
  client.setConfig({
    baseUrl: 'http://setup.test',
    headers: { 'Content-Type': 'application/json' },
  });
  Object.defineProperty(window, 'electron', {
    configurable: true,
    writable: true,
    value: { getSecretKey: vi.fn().mockResolvedValue('setup-secret') },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the server target and its choices (AO-8)', () => {
  it('checks a server login with the device it prepared, leaving out what was not chosen', async () => {
    const fetchMock = answer(200, RUNNING);
    const run = await startSetup({
      target: {
        kind: 'server',
        ssh_target: 'wanjun@leo.ucsf.edu',
        preparation_id: 'prep-1',
        port: null,
        identity_file: null,
        proxy_jump: 'bastion',
      },
      intent: 'check',
    });
    expect(sentBody(fetchMock)).toEqual({
      target: {
        kind: 'server',
        ssh_target: 'wanjun@leo.ucsf.edu',
        proxy_jump: 'bastion',
        preparation_id: 'prep-1',
      },
      intent: 'check',
    });
    // This window started it: the notice for jobs started elsewhere leaves it out.
    expect(setupStartedHere(run.jobId)).toBe(true);
    expect(setupStartedHere('job-2')).toBe(false);
  });

  it('sends restart and start choices only when made', async () => {
    const fetchMock = answer(200, { ...RUNNING, intent: 'install' });
    await startSetup({
      target: { kind: 'server', ssh_target: 'wanjun@leo.ucsf.edu' },
      intent: 'install',
      restart_workspaces: true,
      start_workspaces: [HOSTED.workspace_id],
    });
    expect(sentBody(fetchMock)).toEqual({
      target: { kind: 'server', ssh_target: 'wanjun@leo.ucsf.edu' },
      intent: 'install',
      restart_workspaces: true,
      start_workspaces: [HOSTED.workspace_id],
    });
    await startSetup({
      target: { kind: 'server', ssh_target: 'wanjun@leo.ucsf.edu' },
      intent: 'install',
      restart_workspaces: false,
      start_workspaces: [],
    });
    expect(sentBody(fetchMock, 1)).toEqual({
      target: { kind: 'server', ssh_target: 'wanjun@leo.ucsf.edu' },
      intent: 'install',
      restart_workspaces: false,
    });
  });
});

describe('what a job found and did (AO-6, AO-8)', () => {
  it('reads each workspace, and leaves out one it cannot', () => {
    const found = setupWorkspacesFrom([
      HOSTED,
      { ...HOSTED, workspace_id: '' },
      { ...HOSTED, role: 'admin' },
      { ...HOSTED, state: 'sleeping' },
      'not an entry',
      {
        ...HOSTED,
        workspace_id: '22222222-2222-4333-8444-555555555555',
        role: 'member',
        host_username: 'gbellucci',
        version: '1.94.1; rm -rf ~',
        invitation: 'javascript:alert(1)',
        this_computer: 'elsewhere',
      },
    ]);
    expect(found).toHaveLength(2);
    expect(found[0]).toEqual({
      workspaceId: HOSTED.workspace_id,
      name: 'baranzini-lab',
      role: 'host',
      state: 'running',
      hostUsername: 'wanjun',
      fingerprint: 'abcd',
      mode: 'private',
      institutionId: 'ucsf',
      version: '1.94.1',
      binaryCurrent: false,
      othersConnected: 2,
      invitation: 'brcrew1:abc',
      savedConnectionId: null,
      thisComputer: 'not_saved',
      error: null,
    });
    // A version that is not one, and a line that is not an invitation, are never kept.
    expect(found[1]).toMatchObject({
      role: 'member',
      version: null,
      invitation: null,
      thisComputer: 'not_saved',
    });
    expect(setupWorkspacesFrom(null)).toEqual([]);
  });

  it('reads the workspaces, the note, and what the job restarted, left outdated or failed', () => {
    const run = setupRunFrom({
      ...RUNNING,
      state: 'finished',
      stage: 'done',
      workspaces: [HOSTED],
      workspaces_note: 'timed out',
      outcome: {
        reused: true,
        previous_kept: false,
        restarted: [
          { workspace_id: HOSTED.workspace_id, name: 'baranzini-lab', from_version: '1.94.1' },
          { name: 'no id' },
        ],
        outdated: [
          {
            workspace_id: HOSTED.workspace_id,
            reason: 'others_connected',
            others_connected: 2,
            running_version: '1.94.1',
            installed_version: '1.95.0',
          },
          { workspace_id: HOSTED.workspace_id, reason: 'because' },
        ],
        restart_failed: [
          {
            workspace_id: HOSTED.workspace_id,
            code: 'stop_timeout',
            message: 'It did not stop in time.',
            command: 'biorouter-crew start --workspace-id 1111',
          },
          { workspace_id: HOSTED.workspace_id, code: 'stop_timeout' },
        ],
      },
    });
    expect(run.workspaces).toHaveLength(1);
    expect(run.workspacesNote).toBe('timed out');
    expect(run.outcome?.restarted).toEqual([
      {
        workspaceId: HOSTED.workspace_id,
        name: 'baranzini-lab',
        fromVersion: '1.94.1',
        toVersion: null,
      },
    ]);
    expect(run.outcome?.outdated).toEqual([
      {
        workspaceId: HOSTED.workspace_id,
        name: null,
        runningVersion: '1.94.1',
        installedVersion: '1.95.0',
        othersConnected: 2,
        reason: 'others_connected',
      },
    ]);
    expect(run.outcome?.restartFailed).toEqual([
      {
        workspaceId: HOSTED.workspace_id,
        name: null,
        code: 'stop_timeout',
        message: 'It did not stop in time.',
        command: 'biorouter-crew start --workspace-id 1111',
      },
    ]);
  });

  it('lists the jobs the daemon knows, and reads anything that is not a list as none', async () => {
    let fetchMock = answer(200, [
      { ...RUNNING, state: 'needs_sign_in' },
      { not: 'a job' },
      { ...RUNNING, job_id: 'job-2', state: 'finished', stage: 'done' },
    ]);
    const jobs = await listSetupJobs();
    expect(jobs.map((job) => [job.jobId, job.state])).toEqual([
      ['job-1', 'needs_sign_in'],
      ['job-2', 'finished'],
    ]);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://setup.test/crew/setup');
    expect(init.method).toBe('GET');
    fetchMock = answer(200, { jobs: [] });
    expect(await listSetupJobs()).toEqual([]);
  });
});

describe('the plain steps of a job (AO-8)', () => {
  const run = (intent: string, stages: [string, string][], stage = stages[0][0]) =>
    setupRunFrom({
      ...RUNNING,
      intent,
      stage,
      stages: stages.map(([name, state]) => ({ name, state })),
    });

  it('names Looking for workspaces and Updating running workspaces', () => {
    expect(plainStage(run('install', [['workspaces', 'running']]))).toBe('finding');
    expect(plainStage(run('install', [['restart', 'running']]))).toBe('updating');
  });

  it('a check looks for workspaces and installs nothing', () => {
    expect(plainStagesFor('check', null)).toEqual(['checking', 'signing-in', 'finding', 'ready']);
  });

  it('an install leaves out a step it skipped', () => {
    expect(
      plainStagesFor(
        'install',
        run('install', [
          ['install', 'done'],
          ['workspaces', 'done'],
          ['restart', 'skipped'],
        ])
      )
    ).toEqual(['checking', 'signing-in', 'installing', 'finding', 'ready']);
  });

  it('a host or member job adds the steps only when its daemon reports them', () => {
    expect(plainStagesFor('host', run('host', [['check', 'running']]))).toEqual([
      'checking',
      'signing-in',
      'installing',
      'starting',
      'ready',
    ]);
    expect(
      plainStagesFor(
        'member',
        run('member', [
          ['check', 'done'],
          ['workspaces', 'running'],
          ['restart', 'pending'],
        ])
      )
    ).toEqual([
      'checking',
      'signing-in',
      'installing',
      'finding',
      'updating',
      'connecting',
      'ready',
    ]);
  });
});

describe('versionOlder', () => {
  it('compares major, minor and patch, and never calls an unreadable version older', () => {
    expect(versionOlder('1.94.1', '1.94.2')).toBe(true);
    expect(versionOlder('1.94.2', '1.94.2')).toBe(false);
    expect(versionOlder('1.95.0', '1.94.9')).toBe(false);
    expect(versionOlder('1.9.0', '1.10.0')).toBe(true);
    expect(versionOlder('1.94.1-dev', '1.94.2')).toBe(true);
    expect(versionOlder(null, '1.94.2')).toBe(false);
    expect(versionOlder('latest', '1.94.2')).toBe(false);
  });
});
