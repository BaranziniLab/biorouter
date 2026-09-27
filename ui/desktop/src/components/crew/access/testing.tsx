import { render } from '@testing-library/react';
import type { ComponentType } from 'react';
import { MemoryRouter } from 'react-router-dom';
import type { Mock } from 'vitest';
import CrewView from '../CrewView';

/**
 * Fixtures and a harness for the access area's tests. Not a test file itself: each spec declares its
 * own `vi.mock('../crewApi', …)` (mocks are hoisted per file) and hands the mocked functions here.
 *
 * The harness renders the REAL controller (`CrewView` with an injected layout), so a test exercises
 * `grantSession`, `act`, the error slots and the pane intents exactly as the app does.
 */

export const connection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'alice@hpc',
  port: 22,
  socket_path: '/tmp/socket',
  owner_uid: 1000,
  workspace_id: 'workspace-1',
  workspace_public_key: 'workspace-key',
  public_key: 'device-key',
  device_id: 'device-1',
  cluster_connection_id: 'cluster-1',
  mode: 'private' as const,
  policy_epoch: 1,
  institution_id: 'ucsf',
  status: 'connected' as const,
  remote_execution: false,
};

export const actor = { id: 'person-1', uid: 1000, username: 'alice', nickname: 'Alice Chen' };

const channel = (id: string, name: string, extra: Record<string, unknown> = {}) => ({
  id,
  team_id: 'team-1',
  name,
  created_by: actor.id,
  owner_id: actor.id,
  members: [actor.id],
  archived: false,
  classification: 'restricted' as const,
  ...extra,
});

export const snapshot = {
  workspace: {
    id: 'workspace-1',
    host_uid: 1000,
    mode: 'private' as const,
    policy_epoch: 1,
    institution_id: 'ucsf',
    name: 'lab',
  },
  actor,
  principals: [actor],
  teams: [
    {
      id: 'team-1',
      name: 'Lab',
      created_by: actor.id,
      members: [actor.id],
      general_channel_id: 'channel-1',
    },
  ],
  channels: [
    channel('channel-1', 'general'),
    channel('channel-2', 'methods'),
    channel('channel-3', 'old-notes', { archived: true }),
  ],
  invitations: [],
  runs: [],
};

export interface RunFixture {
  run_id: string;
  channel_id: string;
  session_id: string;
  status: string;
  error?: string;
}

export function stateFrame(runs: RunFixture[] = []) {
  return {
    type: 'state' as const,
    connection_id: connection.id,
    connection_mode: 'private' as const,
    connection_policy_epoch: 1,
    connection_institution_id: 'ucsf',
    snapshot,
    runs,
    cursor: null,
  };
}

/** A grant row as the daemon lists it. Unix seconds for `expires_at`. */
export function grantRow(overrides: Record<string, unknown> = {}) {
  return {
    session_id: 'agent-1',
    run_id: 'run-1',
    connection_id: connection.id,
    channel_id: 'channel-1',
    source_channels: ['channel-1'],
    policy_epoch: 1,
    expired: false,
    kind: 'chat',
    session_name: 'Plot review',
    expires_at: Math.floor(Date.now() / 1000) + 3600,
    ...overrides,
  };
}

export interface CrewMocks {
  crewHttp: Mock;
  crewRequest: Mock;
  observeCrew: Mock;
}

export interface DaemonFixture {
  /** The grants `GET …/grants` answers with, read at request time. */
  grants: () => unknown[];
  runs?: RunFixture[];
  /** `POST …/sessions/{id}/revoke`; defaults to a confirmed revoke. */
  revoke?: (sessionId: string) => unknown;
  /** `POST …/sessions/{id}/grant`; defaults to success. */
  grant?: (sessionId: string, body: unknown) => unknown;
  /** `GET …/grants` failure, when set. */
  listFailure?: () => Error | null;
}

/** Route the mocked Crew API like a daemon holding `fixture`. */
export function installDaemon(mocks: CrewMocks, fixture: DaemonFixture): void {
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET', body?: unknown) => {
    if (path === '/connections' && method === 'GET') return { connections: [connection] };
    if (path === `/connections/${connection.id}/grants` && method === 'GET') {
      const failure = fixture.listFailure?.();
      if (failure) throw failure;
      return { grants: fixture.grants() };
    }
    const revoke = /^\/connections\/conn-1\/sessions\/([^/]+)\/revoke$/.exec(path);
    if (revoke && method === 'POST') {
      const sessionId = decodeURIComponent(revoke[1]);
      return fixture.revoke
        ? fixture.revoke(sessionId)
        : { revoked: true, remote_revocation_confirmed: true, session_id: sessionId };
    }
    const grant = /^\/connections\/conn-1\/sessions\/([^/]+)\/grant$/.exec(path);
    if (grant && method === 'POST') {
      const sessionId = decodeURIComponent(grant[1]);
      return fixture.grant
        ? fixture.grant(sessionId, body)
        : { run_id: 'run-new', session_id: sessionId };
    }
    if (/\/runs\/[^/]+\/cancel$/.test(path) && method === 'POST') return { cancelled: true };
    return {};
  });
  mocks.crewRequest.mockImplementation(async () => ({}));
  mocks.observeCrew.mockImplementation(
    async (
      _connectionId: string,
      channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      receive(stateFrame(fixture.runs ?? []));
      if (channelId)
        receive({
          type: 'messages',
          channel_id: channelId,
          messages: [],
          cursor: null,
          reset: true,
        });
      return 'terminal';
    }
  );
}

/** Render the real Crew controller with `layout`, at `entry`. */
export function renderWithController(layout: ComponentType, entry = '/crew?sessionId=agent-1') {
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <CrewView layout={layout} />
    </MemoryRouter>
  );
}

/** Every call the mocked `crewHttp` received for `path` with `method`. */
export function callsTo(mocks: CrewMocks, path: string, method: string): unknown[][] {
  return mocks.crewHttp.mock.calls.filter(
    ([calledPath, calledMethod = 'GET']) => calledPath === path && calledMethod === method
  );
}
