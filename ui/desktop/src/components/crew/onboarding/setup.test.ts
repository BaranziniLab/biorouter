import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../../api/client.gen';
import { CrewHttpError } from '../crewApi';
import { CREW_DAEMON_OUTDATED, CREW_UNEXPECTED_RESPONSE, isStaleDaemon } from '../api/errors';
import {
  brokerFileName,
  manualInstallCommands,
  MEMBER_SETUP_REQUEST_MS,
  plainStage,
  plainStages,
  readSetup,
  readSetupArtifact,
  requestMemberSetup,
  resetMemberSetupRequestsForTests,
  setupActive,
  setupArtifactFrom,
  setupRunFrom,
  startSetup,
  stopSetup,
  takeMemberSetupRequest,
  type SetupInput,
} from './setup';

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'setup-test-proof' }),
}));

const ARTIFACT = {
  version: '1.94.0',
  target: 'x86_64-unknown-linux-gnu',
  sha256: 'a'.repeat(64),
  size: 3556248,
  source: 'bundled',
};

const RUNNING = {
  job_id: 'job-1',
  intent: 'host',
  state: 'running',
  stage: 'probe',
  stages: [
    { name: 'preflight', state: 'done' },
    { name: 'probe', state: 'running', detail: null },
  ],
  server: { login: 'alice@hpc.ucsf.edu', user: 'alice', uid: 1000 },
  installed: null,
  artifact: ARTIFACT,
  outcome: null,
  error: null,
};

const HOST: SetupInput = {
  target: {
    kind: 'pending',
    preparation_id: 'prep-1',
    ssh_target: 'alice@hpc.ucsf.edu',
    port: null,
    identity_file: null,
    proxy_jump: null,
  },
  intent: 'host',
  workspace_name: 'lab',
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

function sent(fetchMock: ReturnType<typeof vi.fn>, call = 0) {
  const [url, init] = fetchMock.mock.calls[call] as [string, RequestInit];
  return { url, init, headers: new Headers(init.headers) };
}

beforeEach(() => {
  vi.restoreAllMocks();
  resetMemberSetupRequestsForTests();
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

describe('automatic setup, on the wire (OW-9)', () => {
  it('starts a host job with proof that a person asked, naming a target and never a command', async () => {
    const fetchMock = answer(200, RUNNING);
    const run = await startSetup({
      ...HOST,
      command: 'rm -rf ~',
    } as SetupInput & { command: string });

    const { url, init, headers } = sent(fetchMock);
    expect(url).toBe('http://setup.test/crew/setup');
    expect(init.method).toBe('POST');
    expect(headers.get('X-User-Action')).toBe('setup-test-proof');
    expect(headers.get('X-Secret-Key')).toBe('setup-secret');
    // Only what the route takes: an absent route choice is left out, never sent as null.
    expect(JSON.parse(String(init.body))).toEqual({
      target: { kind: 'pending', preparation_id: 'prep-1', ssh_target: 'alice@hpc.ucsf.edu' },
      intent: 'host',
      workspace_name: 'lab',
    });
    expect(String(init.body)).not.toContain('rm -rf');
    expect(run).toMatchObject({
      jobId: 'job-1',
      intent: 'host',
      state: 'running',
      stage: 'probe',
      stages: [
        { name: 'preflight', state: 'done', detail: null },
        { name: 'probe', state: 'running', detail: null },
      ],
      server: { login: 'alice@hpc.ucsf.edu', user: 'alice' },
      artifact: ARTIFACT,
      outcome: null,
      error: null,
    });
  });

  it('starts a member job on a saved connection, with replace when asked', async () => {
    const fetchMock = answer(200, { ...RUNNING, intent: 'member' });
    await startSetup({
      target: { kind: 'connection', connection_id: 'conn-1' },
      intent: 'member',
      replace_existing: true,
    });
    expect(JSON.parse(String(sent(fetchMock).init.body))).toEqual({
      target: { kind: 'connection', connection_id: 'conn-1' },
      intent: 'member',
      replace_existing: true,
    });
  });

  it('reads and stops a job with the same proof, by its escaped id', async () => {
    const fetchMock = answer(200, { ...RUNNING, state: 'finished', stage: 'done' });
    const run = await readSetup('job/../1');
    expect(run.state).toBe('finished');
    const read = sent(fetchMock);
    expect(read.url).toBe('http://setup.test/crew/setup/job%2F..%2F1');
    expect(read.init.method).toBe('GET');
    expect(read.headers.get('X-User-Action')).toBe('setup-test-proof');

    await stopSetup('job-1');
    const stop = sent(fetchMock, 1);
    expect(stop.url).toBe('http://setup.test/crew/setup/job-1');
    expect(stop.init.method).toBe('DELETE');
    expect(stop.headers.get('X-User-Action')).toBe('setup-test-proof');
  });

  it('reads the broker this build carries', async () => {
    const fetchMock = answer(200, ARTIFACT);
    await expect(readSetupArtifact()).resolves.toEqual(ARTIFACT);
    expect(sent(fetchMock).url).toBe('http://setup.test/crew/setup/artifact');
  });

  it.each([
    [409, 'crew_install_artifact_unavailable', 'No broker in this build.'],
    [409, 'crew_setup_busy', 'Two setups are running.'],
    [409, 'crew_host_setup_used', 'This host setup already has a saved connection.'],
    [403, 'crew_user_action_required', 'Confirm this in Biorouter to continue.'],
  ])('passes a %s %s refusal on with the daemon’s sentence', async (status, code, error) => {
    answer(status, { code, error });
    const failure = await startSetup(HOST).catch((reason: unknown) => reason);
    expect(failure).toBeInstanceOf(CrewHttpError);
    expect(failure).toMatchObject({ status, code, message: error });
    expect(isStaleDaemon(failure)).toBe(false);
  });

  it('reads a daemon without the routes as stale', async () => {
    answer(404, undefined);
    expect(isStaleDaemon(await startSetup(HOST).catch((reason: unknown) => reason))).toBe(true);
    expect(isStaleDaemon(await readSetupArtifact().catch((reason: unknown) => reason))).toBe(true);
  });
});

describe('setupRunFrom', () => {
  it('reads a host outcome: the start output, the command it ran, the folder and the draft', () => {
    const run = setupRunFrom({
      ...RUNNING,
      state: 'finished',
      stage: 'done',
      outcome: {
        installed_version: '1.94.0',
        reused: true,
        host: { kind: 'found', text: 'brcrew1:abc' },
        host_command: 'umask 077',
        connection_id: null,
        work_folder: '/home/alice/crew-work/lab',
        context_draft: '# hpc.ucsf.edu',
      },
    });
    expect(run.outcome).toEqual({
      installedVersion: '1.94.0',
      reused: true,
      host: { kind: 'found', text: 'brcrew1:abc' },
      hostCommand: 'umask 077',
      connectionId: null,
      workFolder: '/home/alice/crew-work/lab',
      remoteAccess: null,
      contextDraft: '# hpc.ucsf.edu',
    });
    expect(setupActive(run)).toBe(false);
    expect(plainStage(run)).toBe('ready');
  });

  it('reads a failed job’s typed code, sentence and output', () => {
    const run = setupRunFrom({
      ...RUNNING,
      state: 'failed',
      stage: 'install',
      error: { code: 'crew_install_no_space', message: 'No space.', detail: 'free_kib 10' },
      output: 'BRCREW_INSTALL fail no_space write',
    });
    expect(run.error).toEqual({
      code: 'crew_install_no_space',
      message: 'No space.',
      detail: 'free_kib 10',
    });
    expect(run.output).toBe('BRCREW_INSTALL fail no_space write');
  });

  it('never reads a body that is not a job as one', () => {
    for (const body of [
      { ...RUNNING, state: 'done' },
      { ...RUNNING, intent: 'reboot' },
      { ...RUNNING, job_id: '' },
      { ...RUNNING, stage: '' },
      { ...RUNNING, stages: 'probe' },
      { ...RUNNING, stages: [{ name: 'probe', state: 'maybe' }] },
    ]) {
      expect(() => setupRunFrom(body)).toThrow(
        expect.objectContaining({ code: CREW_UNEXPECTED_RESPONSE })
      );
    }
    // An older daemon answers with its web page, which parses as nothing.
    expect(() => setupRunFrom(null)).toThrow(
      expect.objectContaining({ code: CREW_DAEMON_OUTDATED })
    );
  });

  it('keeps an artifact only when its version and digest can become command text', () => {
    expect(setupArtifactFrom(ARTIFACT)).toEqual(ARTIFACT);
    expect(setupArtifactFrom({ ...ARTIFACT, sha256: 'A'.repeat(64) })?.sha256).toBe('a'.repeat(64));
    for (const bad of [
      { ...ARTIFACT, version: '1.94.0; rm -rf ~' },
      { ...ARTIFACT, sha256: 'a'.repeat(63) },
      { ...ARTIFACT, sha256: `${'a'.repeat(63)}'` },
      { ...ARTIFACT, size: 0 },
      { ...ARTIFACT, source: '' },
    ]) {
      expect(setupArtifactFrom(bad)).toBeNull();
    }
  });
});

describe('plain steps', () => {
  it('folds the daemon’s stages into the words a person reads', () => {
    const at = (stage: string, state = 'running') =>
      plainStage(setupRunFrom({ ...RUNNING, stage, state }));
    expect(at('check')).toBe('checking');
    expect(at('preflight')).toBe('checking');
    expect(at('check_installation')).toBe('checking');
    expect(at('install')).toBe('installing');
    // The work folder and the context draft close a job: its last step before Ready.
    expect(at('work_folder')).toBe('starting');
    expect(at('context')).toBe('starting');
    expect(plainStage(setupRunFrom({ ...RUNNING, intent: 'member', stage: 'work_folder' }))).toBe(
      'connecting'
    );
    expect(at('sign_in', 'needs_sign_in')).toBe('signing-in');
    expect(at('transfer')).toBe('installing');
    expect(at('verify_installed')).toBe('installing');
    expect(at('start')).toBe('starting');
    expect(at('connect')).toBe('connecting');
    // A stage this build does not know yet reads as checking, never as an error.
    expect(at('defragment')).toBe('checking');
    expect(plainStages('host')).toEqual([
      'checking',
      'signing-in',
      'installing',
      'starting',
      'ready',
    ]);
    expect(plainStages('member')).toEqual([
      'checking',
      'signing-in',
      'installing',
      'connecting',
      'ready',
    ]);
  });
});

describe('the manual install', () => {
  it('copies, checks and installs the exact broker, with no placeholder path', () => {
    const commands = manualInstallCommands(ARTIFACT, { ssh_target: 'alice@hpc.ucsf.edu' });
    expect(commands.split('\n')).toEqual([
      'scp ~/Downloads/biorouter-crew-1.94.0-linux-x86_64 alice@hpc.ucsf.edu:biorouter-crew-1.94.0-linux-x86_64',
      `echo '${'a'.repeat(64)}  biorouter-crew-1.94.0-linux-x86_64' | sha256sum -c -`,
      'mkdir -p "$HOME/.local/bin" && install -m 0755 biorouter-crew-1.94.0-linux-x86_64 "$HOME/.local/bin/biorouter-crew" && "$HOME/.local/bin/biorouter-crew" --version',
    ]);
    expect(brokerFileName('1.94.0')).toBe('biorouter-crew-1.94.0-linux-x86_64');
  });

  it('names the saved file and the route, quoted for a shell', () => {
    const commands = manualInstallCommands(
      ARTIFACT,
      { ssh_target: 'alice@hpc', port: 2222, proxy_jump: 'gw', identity_file: '~/.ssh/lab key' },
      "/Users/al ice/Downloads/it's here"
    );
    expect(commands.split('\n')[0]).toBe(
      `scp -P 2222 -J gw -i '~/.ssh/lab key' '/Users/al ice/Downloads/it'\\''s here' alice@hpc:biorouter-crew-1.94.0-linux-x86_64`
    );
  });
});

describe('a member setup Join asked for', () => {
  it('is taken once, and lapses', () => {
    requestMemberSetup('conn-1', 1000);
    expect(takeMemberSetupRequest('conn-2', 1000)).toBe(false);
    expect(takeMemberSetupRequest('conn-1', 1000 + MEMBER_SETUP_REQUEST_MS)).toBe(true);
    expect(takeMemberSetupRequest('conn-1', 1000)).toBe(false);
    requestMemberSetup('conn-1', 1000);
    expect(takeMemberSetupRequest('conn-1', 1001 + MEMBER_SETUP_REQUEST_MS)).toBe(false);
  });
});
