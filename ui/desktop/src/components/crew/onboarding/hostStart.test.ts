import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../../api/client.gen';
import { CrewHttpError } from '../crewApi';
import { CREW_DAEMON_OUTDATED, CREW_UNEXPECTED_RESPONSE, isStaleDaemon } from '../api/errors';
import {
  hostStartRunFrom,
  readHostRun,
  startHostRun,
  stopHostRun,
  type HostStartInput,
} from './hostStart';

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'host-start-test-proof' }),
}));

const INPUT: HostStartInput = {
  preparation_id: 'prep-1',
  workspace_name: 'lab',
  ssh_target: 'alice@hpc.ucsf.edu',
  port: null,
  identity_file: null,
  proxy_jump: null,
};

const RUNNING = {
  job_id: 'job-1',
  command: 'umask 077',
  state: 'running',
  output: 'starting\n',
};

/** Answer every request with `body` at `status`, and record what was sent. */
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
  client.setConfig({
    baseUrl: 'http://host-start.test',
    headers: { 'Content-Type': 'application/json' },
  });
  Object.defineProperty(window, 'electron', {
    configurable: true,
    writable: true,
    value: { getSecretKey: vi.fn().mockResolvedValue('host-start-secret') },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('Start it for me, on the wire (D-HOST)', () => {
  it('asks with proof that a person did, and names the setup and login, never a command', async () => {
    const fetchMock = answer(200, RUNNING);
    // Extra fields a caller might carry never ride along: the route refuses unknown fields.
    const run = await startHostRun({
      ...INPUT,
      command: 'rm -rf ~',
    } as HostStartInput & { command: string });

    const { url, init, headers } = sent(fetchMock);
    expect(url).toBe('http://host-start.test/crew/host/start');
    expect(init.method).toBe('POST');
    expect(headers.get('X-User-Action')).toBe('host-start-test-proof');
    expect(headers.get('X-Secret-Key')).toBe('host-start-secret');
    expect(JSON.parse(String(init.body))).toEqual(INPUT);
    expect(String(init.body)).not.toContain('rm -rf');
    expect(run).toEqual({
      jobId: 'job-1',
      command: 'umask 077',
      state: 'running',
      output: 'starting\n',
      result: null,
      error: null,
    });
  });

  it('reads and stops a run with the same proof, by its escaped id', async () => {
    const fetchMock = answer(200, {
      ...RUNNING,
      state: 'finished',
      result: { kind: 'found', text: 'brcrew1:abc' },
    });
    const run = await readHostRun('job/../1');
    expect(run.result).toEqual({ kind: 'found', text: 'brcrew1:abc' });
    const read = sent(fetchMock);
    expect(read.url).toBe('http://host-start.test/crew/host/start/job%2F..%2F1');
    expect(read.init.method).toBe('GET');
    expect(read.headers.get('X-User-Action')).toBe('host-start-test-proof');

    await stopHostRun('job-1');
    const stop = sent(fetchMock, 1);
    expect(stop.url).toBe('http://host-start.test/crew/host/start/job-1');
    expect(stop.init.method).toBe('DELETE');
    expect(stop.headers.get('X-User-Action')).toBe('host-start-test-proof');
  });

  it.each([
    [403, 'crew_user_action_required', 'Confirm this in Biorouter to continue.'],
    [
      409,
      'crew_host_setup_used',
      'This host setup already has a saved connection. Open it from Crew.',
    ],
    [
      409,
      'crew_host_setup_unknown',
      'This computer has no host setup with that ID. Start hosting again.',
    ],
    [400, 'crew_request_invalid', 'Type the server login as an SSH alias or user@host.'],
  ])('passes a %s %s refusal on with the daemon’s sentence', async (status, code, error) => {
    answer(status, { code, error });
    const failure = await startHostRun(INPUT).then(
      () => {
        throw new Error('expected a refusal');
      },
      (reason: unknown) => reason
    );
    expect(failure).toBeInstanceOf(CrewHttpError);
    expect(failure).toMatchObject({ status, code, message: error });
    expect(isStaleDaemon(failure)).toBe(false);
  });

  it('reads a daemon without the route as stale', async () => {
    answer(404, undefined);
    const failure = await startHostRun(INPUT).catch((reason: unknown) => reason);
    expect(isStaleDaemon(failure)).toBe(true);
  });
});

describe('hostStartRunFrom', () => {
  it('reads a failed run’s typed code and sentence, and a problem’s detail', () => {
    expect(
      hostStartRunFrom({
        ...RUNNING,
        state: 'failed',
        error: { code: 'crew_ssh_auth_required', message: 'Run them yourself.' },
      }).error
    ).toEqual({ code: 'crew_ssh_auth_required', message: 'Run them yourself.' });
    expect(
      hostStartRunFrom({
        ...RUNNING,
        state: 'finished',
        result: { kind: 'problem', problem: 'server_error', detail: 'locked' },
      }).result
    ).toEqual({ kind: 'problem', problem: 'server_error', detail: 'locked' });
  });

  it('never reads a body that is not a run as one', () => {
    for (const body of [
      { ...RUNNING, state: 'done' },
      { ...RUNNING, job_id: '' },
      { state: 'running', output: '' },
      { ...RUNNING, command: 7 },
    ]) {
      expect(() => hostStartRunFrom(body)).toThrow(
        expect.objectContaining({ code: CREW_UNEXPECTED_RESPONSE })
      );
    }
    // An older daemon answers with its web page, which parses as nothing.
    expect(() => hostStartRunFrom(null)).toThrow(
      expect.objectContaining({ code: CREW_DAEMON_OUTDATED })
    );
  });

  it('drops a result it does not recognise rather than guessing', () => {
    expect(
      hostStartRunFrom({ ...RUNNING, state: 'finished', result: { kind: 'shell', text: 'x' } })
        .result
    ).toBeNull();
    expect(
      hostStartRunFrom({ ...RUNNING, state: 'finished', result: { kind: 'found', text: '' } })
        .result
    ).toBeNull();
  });
});
