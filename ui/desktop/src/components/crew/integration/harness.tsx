/**
 * Test-only harness for the cross-area integration tests: a rich workspace whose every machine
 * identifier is shaped like the real thing (UUIDs, 64-hex keys, distinctive UIDs), a scripted
 * daemon behind the mocked wire helpers, and a render helper that mounts the real controller under
 * the real layout. Nothing in the app imports this file.
 *
 * The calling test file must mock, at its own top level (vi.mock is hoisted per file):
 *
 *   vi.mock('../crewApi', …)          crewHttp, crewRequest and observeCrew as vi.fn()s
 *   vi.mock('../../ConfigContext', …) useConfig for the model picker (keep the actual rest), with
 *                                     callbacks that stay the same across renders (`vi.hoisted`),
 *                                     as the real context's do; new ones each render never settle
 *   vi.mock('../CrewAuthentication')  the SSH terminal
 *
 * and call `installResizeObserverStub()`. This module then drives those mocks through the same
 * module instance.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import type { Mock } from 'vitest';
import { CREW_APP_OPTIONS } from '../CrewApp';
import {
  crewHttp,
  crewRequest,
  observeCrew,
  type Channel,
  type CrewMessage,
  type ObservedRun,
  type Snapshot,
} from '../crewApi';
import { CrewLayout } from '../layout/CrewLayout';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import { useCrewController } from '../state/useCrewController';
import type { CrewController } from '../state/types';

export const mocked = {
  crewHttp: crewHttp as unknown as Mock,
  crewRequest: crewRequest as unknown as Mock,
  observeCrew: observeCrew as unknown as Mock,
};

// ── Identifiers: each shaped like the real one, so a leak cannot hide ────────────────────────
export const ids = {
  workspace: 'a8f5c2d1-7b3e-4c9a-8d21-5f6e7a8b9c01',
  alice: '1f0e2d3c-4b5a-4968-8776-a5b4c3d2e1f0',
  bob: '2a1b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d',
  carol: '3b2c1d0e-9f8e-4d7c-8b6a-5f4e3d2c1b0a',
  team: '4c3d2e1f-0a9b-4c8d-9e7f-6a5b4c3d2e1f',
  otherTeam: '7f6a5b4c-3d2e-4f1a-8b9c-0d1e2f3a4b5c',
  general: '5d4e3f2a-1b0c-4d9e-8f7a-6b5c4d3e2f1a',
  methods: '6e5f4a3b-2c1d-4e0f-9a8b-7c6d5e4f3a2b',
  invitation: '8a7b6c5d-4e3f-4a2b-9c1d-0e2f3a4b5c6d',
  run: '9b8c7d6e-5f4a-4b3c-8d2e-1f0a9b8c7d6e',
  session: '0c9d8e7f-6a5b-4c4d-9e3f-2a1b0c9d8e7f',
  blob: 'b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e',
  device: 'c2d3e4f5-a6b7-4c8d-9e0f-1a2b3c4d5e6f',
  cluster: 'd3e4f5a6-b7c8-4d9e-8f0a-2b3c4d5e6f7a',
  messages: [
    'e4f5a6b7-c8d9-4e0f-9a1b-3c4d5e6f7a8b',
    'f5a6b7c8-d9e0-4f1a-8b2c-4d5e6f7a8b9c',
    'a6b7c8d9-e0f1-4a2b-9c3d-5e6f7a8b9c0d',
    'b7c8d9e0-f1a2-4b3c-8d4e-6f7a8b9c0d1e',
  ],
} as const;

/** 64-hex keys: the workspace's, this device's, and a file's SHA-256. */
export const keys = {
  workspace: '3f2a9c1e77b0d4e1'.repeat(4),
  device: '9e8d7c6b5a4f3e2d'.repeat(4),
  sha256: 'aa11bb22cc33dd44'.repeat(4),
} as const;

/** The people's UIDs on the server: distinctive, so a UID printed anywhere is found. */
export const uids = { alice: 70301, bob: 70302 } as const;

export const MACHINE_ID_PATTERNS: readonly RegExp[] = [
  /[0-9a-f]{8}-[0-9a-f]{4}-/i,
  /[0-9a-f]{64}/i,
  /\b7030[12]\b/,
];

export const connection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'alice@hpc.example.edu',
  port: 22,
  socket_path: '/tmp/crew-fixture/socket',
  owner_uid: uids.alice,
  workspace_id: ids.workspace,
  workspace_public_key: keys.workspace,
  public_key: keys.device,
  device_id: ids.device,
  cluster_connection_id: ids.cluster,
  mode: 'private' as const,
  policy_epoch: 1,
  institution_id: 'ucsf',
  status: 'connected' as const,
  remote_execution: false,
};

export const alice = {
  id: ids.alice,
  uid: uids.alice,
  username: 'alice',
  nickname: 'Alice Chen',
  display_name: 'Alice Chen',
  active: true,
};
export const bob = {
  id: ids.bob,
  uid: uids.bob,
  username: 'bob',
  nickname: 'Bob Lee',
  display_name: 'Bob Lee',
  active: true,
};
/** Removed from the workspace; still the author of a message. */
export const carol = {
  id: ids.carol,
  username: 'carol',
  display_name: 'Carol Diaz',
  active: false as const,
};

export const general: Channel = {
  id: ids.general,
  team_id: ids.team,
  name: 'general',
  created_by: ids.alice,
  owner_id: ids.alice,
  members: [ids.alice, ids.bob],
  archived: false,
  classification: 'restricted',
};
export const methods: Channel = {
  id: ids.methods,
  team_id: ids.team,
  name: 'methods',
  created_by: ids.alice,
  owner_id: ids.alice,
  members: [ids.alice],
  archived: false,
  classification: 'public_safe',
};

const now = Math.floor(Date.now() / 1000);

/** The rich workspace: a former member, an invitation, a waiting joiner, a running task. */
export function richSnapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    workspace: {
      id: ids.workspace,
      host_uid: uids.alice,
      mode: 'private',
      institution_id: 'ucsf',
      policy_epoch: 1,
      name: 'lab',
    },
    actor: alice,
    principals: [alice, bob],
    former_principals: [carol],
    teams: [
      {
        id: ids.team,
        name: 'Analysis Lab',
        created_by: ids.alice,
        members: [ids.alice, ids.bob],
        general_channel_id: ids.general,
      },
    ],
    channels: [general, methods],
    invitations: [
      {
        id: ids.invitation,
        kind: 'team',
        target_id: ids.otherTeam,
        principal_id: ids.alice,
        inviter_id: ids.bob,
        expires_at: now + 3600,
        target_name: 'Imaging Core',
        inviter: { username: 'bob', display_name: 'Bob Lee' },
      },
    ],
    pending_joins: [
      { username: 'dave', full_name: 'Dave Kim', created_at: now - 60, expires_at: now + 3600 },
    ],
    runs: [],
    read_positions: { [ids.general]: null },
    unread: { [ids.general]: 0 },
    ...overrides,
  };
}

export const ownedRun: ObservedRun = {
  run_id: ids.run,
  channel_id: ids.general,
  session_id: ids.session,
  status: 'running',
};

export function richMessages(): CrewMessage[] {
  const base = {
    channel_id: ids.general,
    restricted: false,
    source_channels: [ids.general],
    attachments: [] as string[],
  };
  return [
    {
      ...base,
      id: ids.messages[0],
      sequence: ids.messages[0],
      actor_id: ids.carol,
      body: 'Counts are in.',
      created_at: now - 600,
    },
    {
      ...base,
      id: ids.messages[1],
      sequence: ids.messages[1],
      actor_id: ids.bob,
      body: 'Here is the table.',
      created_at: now - 500,
      attachments: [ids.blob],
    },
    {
      ...base,
      id: ids.messages[2],
      sequence: ids.messages[2],
      actor_id: ids.alice,
      body: 'Plot next?',
      created_at: now - 400,
    },
    {
      ...base,
      id: ids.messages[3],
      sequence: ids.messages[3],
      actor_id: ids.alice,
      run_id: ids.run,
      body: 'Task: plot counts by sample',
      created_at: now - 300,
    },
  ];
}

/** What the scripted daemon answers. Tests change it between steps. */
export interface Daemon {
  connections: Record<string, unknown>[];
  snapshot: Snapshot;
  runs: ObservedRun[];
  messages: CrewMessage[];
  /** The mode and epoch the observer verified for the connection. */
  connectionMode: 'private' | 'public';
  /** When set, an observation hangs (sends nothing) until it is aborted. */
  hold: boolean;
  /** `crewHttp` answers checked first: return `undefined` to fall through to the defaults. */
  http?: (path: string, method: string, body: unknown) => unknown;
  /** `crewRequest` answers checked first: return `undefined` to fall through to the defaults. */
  request?: (method: string, params: Record<string, unknown>) => unknown;
}

export interface ScriptedDaemon {
  state: Daemon;
  /** Deliver a frame to the most recent observation, as the daemon would. */
  emit(frame: unknown): void;
  /** Answer a held observation now: stop holding and send it the current state and messages. */
  release(): void;
}

export function installDaemon(initial: Partial<Daemon> = {}): ScriptedDaemon {
  const state: Daemon = {
    connections: [connection],
    snapshot: richSnapshot(),
    runs: [],
    messages: [],
    connectionMode: 'private',
    hold: false,
    ...initial,
  };
  let receive: ((frame: unknown) => void) | null = null;
  let answerHeld: (() => void) | null = null;

  const answer = (
    connectionId: string,
    channelId: string | undefined,
    deliver: (frame: unknown) => void
  ) => {
    deliver({
      type: 'state',
      connection_id: connectionId,
      connection_mode: state.connectionMode,
      connection_policy_epoch: 1,
      connection_institution_id: connection.institution_id,
      snapshot: state.snapshot,
      runs: state.runs,
      cursor: null,
    });
    if (channelId) {
      const messages = state.messages.filter((message) => message.channel_id === channelId);
      deliver({
        type: 'messages',
        channel_id: channelId,
        messages,
        cursor: messages.length ? messages[messages.length - 1].sequence : null,
        reset: true,
      });
    }
  };

  mocked.crewHttp.mockImplementation(async (path: string, method = 'GET', body?: unknown) => {
    const answer = state.http?.(path, method, body);
    if (answer !== undefined) return answer;
    if (path === '/connections' && method === 'GET') return { connections: state.connections };
    if (path.startsWith('/transfers?')) return { transfers: [] };
    if (path === `/connections/${connection.id}/grants`) return { grants: [] };
    if (path === `/connections/${connection.id}/runs` && method === 'GET')
      return { runs: state.runs };
    if (path === `/connections/${connection.id}/join`) return { status: 'joined' };
    return {};
  });
  mocked.crewRequest.mockImplementation(
    async (_connection: string, method: string, params: Record<string, unknown> = {}) => {
      const answer = state.request?.(method, params);
      if (answer !== undefined) return answer;
      if (method === 'messages.history') return { messages: [], cursor: null };
      if (method === 'blob.status')
        return {
          id: params.blob_id,
          channel_id: ids.general,
          name: 'counts.csv',
          size: 56_320,
          sha256: keys.sha256,
          complete: true,
          media_type: 'text/csv',
        };
      return {};
    }
  );
  mocked.observeCrew.mockImplementation(
    async (
      connectionId: string,
      channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      deliver: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      receive = deliver;
      if (state.hold) {
        answerHeld = () => answer(connectionId, channelId, deliver);
        await new Promise<void>((resolve) =>
          signal.addEventListener('abort', () => resolve(), { once: true })
        );
        return 'terminal';
      }
      answer(connectionId, channelId, deliver);
      return 'terminal';
    }
  );

  return {
    state,
    emit(frame: unknown) {
      receive?.(frame);
    },
    release() {
      state.hold = false;
      const held = answerHeld;
      answerHeld = null;
      held?.();
    },
  };
}

let latest: CrewController | null = null;

/** The controller as of the last render, for driving what a DOM query cannot. */
export function currentCrew(): CrewController {
  if (!latest) throw new Error('No Crew controller has rendered yet.');
  return latest;
}

/** `CrewApp`, with the controller captured: the same options, the same layout. */
function CapturedCrewApp() {
  const controller = useCrewController(CREW_APP_OPTIONS);
  latest = controller;
  return (
    <CrewControllerProvider controller={controller}>
      <CrewLayout />
    </CrewControllerProvider>
  );
}

export function renderCrew(entry = '/crew') {
  latest = null;
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <CapturedCrewApp />
    </MemoryRouter>
  );
}

/** Wait until the selected channel's composer is on screen: the channel view is verified. */
export async function channelReady(channel = 'general') {
  return screen.findByRole('textbox', { name: `Message #${channel}` });
}
