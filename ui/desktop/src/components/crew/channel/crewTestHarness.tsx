/**
 * Test-only harness for the channel and pane areas: fixtures, an observer that answers with a
 * verified state, and a render helper that mounts the REAL controller (`CrewView`) around a small
 * layout. Nothing in the app imports this file.
 *
 * The calling test file must mock `../crewApi` (replacing `crewHttp`, `crewRequest` and
 * `observeCrew` with `vi.fn()`s, exactly as the Crew regression tests do); this module then drives
 * those mocks through the same module instance.
 */
import { render } from '@testing-library/react';
import type { ComponentType, ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import type { Mock } from 'vitest';
import CrewView from '../CrewView';
import { crewHttp, crewRequest, observeCrew, type ObservedRun } from '../crewApi';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController, CrewControllerOptions } from '../state/types';

export const connection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'alice@hpc.example.edu',
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

export const alice = {
  id: '6f1c2a3b-0000-4000-8000-00000000a11c',
  uid: 1000,
  username: 'alice',
  nickname: 'Alice Chen',
};
export const bob = {
  id: '6f1c2a3b-0000-4000-8000-0000000000b0',
  uid: 1001,
  username: 'bob',
  nickname: 'Bob Lee',
};
export const carol = {
  id: '6f1c2a3b-0000-4000-8000-00000000ca01',
  uid: 1002,
  username: 'carol',
  nickname: 'Carol Diaz',
};

export const general = {
  id: '6f1c2a3b-0000-4000-8000-0000000c0001',
  team_id: '6f1c2a3b-0000-4000-8000-0000000fee01',
  name: 'general',
  created_by: alice.id,
  owner_id: alice.id,
  members: [alice.id, bob.id],
  archived: false,
  classification: 'restricted' as const,
};
export const methods = {
  id: '6f1c2a3b-0000-4000-8000-0000000c0002',
  team_id: general.team_id,
  name: 'methods',
  created_by: alice.id,
  owner_id: alice.id,
  members: [alice.id],
  archived: false,
  classification: 'public_safe' as const,
};
export const oldNotes = {
  id: '6f1c2a3b-0000-4000-8000-0000000c0003',
  team_id: general.team_id,
  name: 'old-notes',
  created_by: alice.id,
  owner_id: alice.id,
  members: [alice.id],
  archived: true,
  classification: 'restricted' as const,
};
export const team = {
  id: general.team_id,
  name: 'Analysis Lab',
  created_by: alice.id,
  members: [alice.id, bob.id, carol.id],
  general_channel_id: general.id,
};

export type FixtureSnapshot = {
  workspace: {
    id: string;
    host_uid: number;
    mode: 'private' | 'public';
    policy_epoch: number;
    institution_id?: string | null;
    name?: string | null;
  };
  actor: Record<string, unknown> & { id: string };
  principals: Record<string, unknown>[];
  teams: Record<string, unknown>[];
  channels: Record<string, unknown>[];
  invitations: Record<string, unknown>[];
  runs: unknown[];
  [key: string]: unknown;
};

/** A workspace named "lab", hosted by Alice, seen by `actor` (default Alice). */
export function makeSnapshot(overrides: Partial<FixtureSnapshot> = {}): FixtureSnapshot {
  return {
    workspace: {
      id: 'workspace-1',
      host_uid: 1000,
      mode: 'private',
      policy_epoch: 1,
      institution_id: 'ucsf',
      name: 'lab',
    },
    actor: alice,
    principals: [alice, bob, carol],
    teams: [team],
    channels: [general, methods, oldNotes],
    invitations: [],
    runs: [],
    ...overrides,
  };
}

export interface ObserverOptions {
  snapshot?: FixtureSnapshot;
  runs?: ObservedRun[];
  messages?: Record<string, unknown>[];
}

export function stateFrame(options: ObserverOptions = {}) {
  const snapshot = options.snapshot ?? makeSnapshot();
  return {
    type: 'state' as const,
    connection_id: connection.id,
    connection_mode: snapshot.workspace.mode,
    connection_policy_epoch: 1,
    connection_institution_id: connection.institution_id,
    snapshot,
    runs: options.runs ?? [],
    cursor: null,
  };
}

export const message = (sequence: string, body = `message ${sequence}`) => ({
  id: `message-${sequence}`,
  sequence,
  channel_id: general.id,
  actor_id: bob.id,
  body,
  created_at: 1_700_000_000,
  restricted: false,
  source_channels: [general.id],
  attachments: [],
});

const mocked = {
  crewHttp: crewHttp as unknown as Mock,
  crewRequest: crewRequest as unknown as Mock,
  observeCrew: observeCrew as unknown as Mock,
};
export { mocked };

/** Answer every observation with a verified state and the channel's messages, then end. */
export function installObserver(options: ObserverOptions | (() => ObserverOptions) = {}) {
  mocked.observeCrew.mockImplementation(
    async (
      _connectionId: string,
      channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      receive: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      const current = typeof options === 'function' ? options() : options;
      receive(stateFrame(current));
      if (channelId) {
        const messages = current.messages ?? [];
        receive({
          type: 'messages',
          channel_id: channelId,
          messages,
          cursor: messages.length ? messages[messages.length - 1].sequence : null,
          reset: true,
        });
      }
      return 'terminal';
    }
  );
}

/** The daemon's usual answers: one saved connection, no runs, no transfers. */
export function installDaemon(connections: Record<string, unknown>[] = [connection]) {
  mocked.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (path === '/connections') return { connections };
    if (path.startsWith('/transfers?')) return { transfers: [] };
    if (path === `/connections/${connection.id}/runs` && method === 'GET') return { runs: [] };
    if (path === `/connections/${connection.id}/runs` && method === 'POST')
      return { run_id: 'run-1', session_id: 'session-1' };
    return {};
  });
  mocked.crewRequest.mockImplementation(async (_id: string, method: string) => {
    if (method === 'messages.history') return { messages: [], cursor: null };
    return {};
  });
}

let latest: CrewController | null = null;

/** The controller as of the last render, for assertions a DOM query cannot make. */
export function currentCrew(): CrewController {
  if (!latest) throw new Error('No Crew controller has rendered yet.');
  return latest;
}

function Capture({ children }: { children?: ReactNode }) {
  latest = useCrew();
  return <>{children}</>;
}

/**
 * Mount the real controller with `Layout` inside it. The controller runs with the new layout's
 * options (the last verified view is kept during a refresh).
 */
export function renderCrew(
  Layout: ComponentType,
  {
    entry = '/crew',
    options = { keepLastVerifiedView: true, autoOpenSignIn: true },
  }: { entry?: string; options?: CrewControllerOptions } = {}
) {
  latest = null;
  function Wrapped() {
    return (
      <Capture>
        <Layout />
      </Capture>
    );
  }
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <CrewView layout={Wrapped} controllerOptions={options} />
    </MemoryRouter>
  );
}
