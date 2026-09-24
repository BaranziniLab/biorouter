import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../../api/client.gen';
import { observeCrew, type CrewObservation } from '../crewApi';

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'projection-test-proof' }),
}));

// The `labels`, `capabilities`, `former_principals`, `pending_joins`, `actor.devices` and run
// `started_at` projections on a state frame (S1a/S3a). crewApi.observation.test.ts covers the
// framing and the projections beside a messages frame.

const encoder = new TextEncoder();

function streamOf(...frames: unknown[]): Response {
  const text = frames.map((frame) => `${JSON.stringify(frame)}\n`).join('');
  return {
    ok: true,
    status: 200,
    headers: new Headers({ 'content-type': 'application/x-ndjson' }),
    body: new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoder.encode(text));
        controller.close();
      },
    }),
  } as unknown as Response;
}

function state(snapshot: Record<string, unknown> = {}, frame: Record<string, unknown> = {}) {
  return {
    type: 'state',
    connection_id: 'connection-1',
    connection_mode: 'private',
    connection_policy_epoch: 2,
    connection_institution_id: 'ucsf',
    snapshot: {
      actor: { id: 'actor-1', uid: 1, username: 'alice', nickname: 'alice' },
      workspace: { id: 'workspace-1', host_uid: 1, mode: 'private', policy_epoch: 3 },
      principals: [],
      invitations: [],
      runs: [],
      channels: [],
      teams: [],
      ...snapshot,
    },
    runs: [],
    ...frame,
  };
}

async function observedState(frame: unknown): Promise<Extract<CrewObservation, { type: 'state' }>> {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(streamOf(frame, { type: 'reconnect', cursor: null }))
  );
  const received: CrewObservation[] = [];
  await expect(
    observeCrew('connection-1', undefined, null, new AbortController().signal, (observed) =>
      received.push(observed)
    )
  ).resolves.toBe('reconnect');
  const first = received[0];
  if (first?.type !== 'state') throw new Error('expected a state frame');
  return first;
}

describe('observation state projections', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    client.setConfig({ baseUrl: 'http://crew-projections.test', headers: {} });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { getSecretKey: vi.fn().mockResolvedValue('projection-secret') },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('passes a well-formed labels map through', async () => {
    const labels = {
      'actor-1': { full: 'Alice Chen (@alice)', short: 'Alice Chen', collides: false },
      'principal-2': { full: 'Sam Park (@sam)', short: 'Sam Park (@sam)', collides: true },
    };

    const observed = await observedState(state({}, { labels }));

    expect(observed.labels).toEqual(labels);
  });

  it('leaves labels out when the daemon sent none', async () => {
    const observed = await observedState(state());
    expect(observed).not.toHaveProperty('labels');
    const nulled = await observedState(state({}, { labels: null }));
    expect(nulled).not.toHaveProperty('labels');
  });

  it('keeps only the labels it can render, without failing the observation', async () => {
    const observed = await observedState(
      state(
        {},
        {
          labels: {
            good: { full: 'Bob Lee (@bob)', short: 'Bob Lee', collides: false },
            'no-short': { full: 'Carol (@carol)', collides: false },
            'blank-full': { full: '  ', short: 'x', collides: false },
            'string-flag': { full: 'Dee (@dee)', short: 'Dee', collides: 'yes' },
            scalar: 'Eve',
            extra: { full: 'Fay (@fay)', short: 'Fay', collides: false, id: 'leaked' },
          },
        }
      )
    );

    expect(observed.labels).toEqual({
      good: { full: 'Bob Lee (@bob)', short: 'Bob Lee', collides: false },
      extra: { full: 'Fay (@fay)', short: 'Fay', collides: false },
    });
  });

  it('drops a labels value that is not a map', async () => {
    for (const labels of [['Alice'], 'Alice', 7]) {
      const observed = await observedState(state({}, { labels }));
      expect(observed).not.toHaveProperty('labels');
    }
  });

  it('never lets a "__proto__" label reach the prototype', async () => {
    const line = JSON.stringify(state()).replace(
      /}$/,
      ',"labels":{"__proto__":{"full":"Mallory (@m)","short":"Mallory","collides":false}}}'
    );
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        headers: new Headers({ 'content-type': 'application/x-ndjson' }),
        body: new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(encoder.encode(`${line}\n{"type":"reconnect","cursor":null}\n`));
            controller.close();
          },
        }),
      })
    );
    const received: CrewObservation[] = [];
    await observeCrew('connection-1', undefined, null, new AbortController().signal, (frame) =>
      received.push(frame)
    );

    const first = received[0];
    if (first?.type !== 'state') throw new Error('expected a state frame');
    expect(Object.getPrototypeOf(first.labels)).toBe(Object.prototype);
    expect(Object.prototype.hasOwnProperty.call(first.labels, '__proto__')).toBe(true);
    expect(({} as Record<string, unknown>).full).toBeUndefined();
  });

  it('passes the S1a and S3a snapshot projections through', async () => {
    const snapshot = {
      actor: {
        id: 'actor-1',
        uid: 1,
        username: 'alice',
        nickname: 'Alice Chen',
        display_name: 'Alice Chen',
        active: true,
        devices: [{ fingerprint: '3F2A 9C1E 77B0 D4E1', added_at: 1, added_via: 'bootstrap' }],
      },
      workspace: {
        id: 'workspace-1',
        host_uid: 1,
        mode: 'private',
        policy_epoch: 3,
        host_principal_id: 'actor-1',
        name: 'lab',
      },
      former_principals: [
        { id: 'former-1', username: 'bob', display_name: 'Bob Lee', active: false },
      ],
      pending_joins: [
        {
          username: 'carol',
          full_name: 'Carol Diaz',
          add_device: false,
          approved: false,
          created_at: 1,
          expires_at: 2,
          mismatched_attempts: 1,
        },
      ],
    };

    const observed = await observedState(state(snapshot));

    expect(observed.snapshot.actor.devices).toEqual(snapshot.actor.devices);
    expect(observed.snapshot.workspace.host_principal_id).toBe('actor-1');
    expect(observed.snapshot.workspace.name).toBe('lab');
    expect(observed.snapshot.former_principals).toEqual(snapshot.former_principals);
    expect(observed.snapshot.pending_joins).toEqual(snapshot.pending_joins);
  });

  it('drops malformed display projections instead of failing the observation', async () => {
    const observed = await observedState(
      state({
        actor: {
          id: 'actor-1',
          uid: 1,
          username: 'alice',
          nickname: 'alice',
          devices: [{ fingerprint: 'AAAA BBBB' }, { fingerprint: 7 }, null],
        },
        former_principals: [{ id: 'former-1', username: 'bob' }, { id: 'former-2' }, 'carol'],
        pending_joins: { username: 'dee' },
      })
    );

    expect(observed.snapshot.actor.devices).toEqual([{ fingerprint: 'AAAA BBBB' }]);
    expect(observed.snapshot.former_principals).toEqual([{ id: 'former-1', username: 'bob' }]);
    expect(observed.snapshot).not.toHaveProperty('pending_joins');

    const nulled = await observedState(
      state({
        former_principals: null,
        pending_joins: null,
        actor: { id: 'actor-1', uid: 1, username: 'alice', nickname: 'alice', devices: null },
      })
    );
    expect(nulled.snapshot).not.toHaveProperty('former_principals');
    expect(nulled.snapshot).not.toHaveProperty('pending_joins');
    expect(nulled.snapshot.actor).not.toHaveProperty('devices');
  });

  it('passes the broker’s capabilities through, keeping only words', async () => {
    const observed = await observedState(
      state({}, { capabilities: ['unique_names_v1', 7, '', 'join_v1'] })
    );
    expect(observed.capabilities).toEqual(['unique_names_v1', 'join_v1']);
    expect(await observedState(state())).not.toHaveProperty('capabilities');
    expect(await observedState(state({}, { capabilities: 'unique_names_v1' }))).not.toHaveProperty(
      'capabilities'
    );
  });

  it('keeps a run’s start time only when it is one', async () => {
    const run = { run_id: 'run-1', channel_id: 'c', session_id: 's', status: 'completed' };
    const observed = await observedState(
      state(
        {},
        {
          runs: [
            { ...run, started_at: 1_790_000_000_000 },
            { ...run, run_id: 'run-2', started_at: -1 },
            { ...run, run_id: 'run-3', started_at: '1790000000000' },
            { ...run, run_id: 'run-4' },
          ],
        }
      )
    );
    expect(observed.runs.map((item) => item.started_at)).toEqual([
      1_790_000_000_000,
      undefined,
      undefined,
      undefined,
    ]);
    expect(observed.runs.map((item) => item.run_id)).toEqual(['run-1', 'run-2', 'run-3', 'run-4']);
    expect(observed.runs[1]).not.toHaveProperty('started_at');
  });

  it('keeps a pending join’s expired flag only when it is a flag', async () => {
    const observed = await observedState(
      state({
        pending_joins: [
          { username: 'carol', expired: true },
          { username: 'dee', expired: 'yes' },
          { username: 'erin' },
        ],
      })
    );
    expect(observed.snapshot.pending_joins).toEqual([
      { username: 'carol', expired: true },
      { username: 'dee' },
      { username: 'erin' },
    ]);
  });

  it('still refuses a state frame whose required parts are malformed', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          streamOf(state({ principals: 'none' }, { labels: { a: { full: 'A', short: 'A' } } }))
        )
    );
    await expect(
      observeCrew('connection-1', undefined, null, new AbortController().signal, vi.fn())
    ).rejects.toThrow('invalid Crew observation');
  });
});
