/**
 * Test fixtures for the sidebar's own suites: a complete `CrewController` stand-in and a rich
 * snapshot. Tests only — nothing in the app imports this file.
 *
 * The controller is a plain object of values and `vi.fn()`s, not the real hook: the sidebar reads
 * the controller and asks it to act, so each suite can set exactly the state it is about and
 * assert exactly the request the sidebar made. `act` runs its function the way the real one does
 * (records a failure instead of throwing), so a click reaches the wire call behind it.
 */
import { render, type RenderResult } from '@testing-library/react';
import type { ReactElement } from 'react';
import { vi } from 'vitest';
import type { CrewConnection, Snapshot } from '../crewApi';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';

export const alice = {
  id: 'person-alice-0000',
  uid: 1000,
  username: 'alice',
  nickname: 'Alice Chen',
};
export const bob = {
  id: 'person-bob-0000',
  uid: 1001,
  username: 'bob',
  nickname: 'Bob Lee',
};

export const connection: CrewConnection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'alice@hpc.ucsf.edu',
  port: 22,
  socket_path: '/tmp/socket',
  owner_uid: 1000,
  workspace_id: 'workspace-1',
  workspace_public_key: 'workspace-key',
  public_key: 'device-key',
  device_id: 'device-1',
  cluster_connection_id: 'cluster-1',
  mode: 'private',
  policy_epoch: 1,
  institution_id: 'ucsf',
  status: 'connected',
  remote_execution: false,
};

export const secondConnection: CrewConnection = {
  ...connection,
  id: 'conn-2',
  name: 'Imaging core',
  ssh_target: 'alice@imaging.ucsf.edu',
  status: 'disconnected',
};

const channel = (
  id: string,
  teamId: string,
  name: string,
  extra: Partial<Snapshot['channels'][number]> = {}
): Snapshot['channels'][number] => ({
  id,
  team_id: teamId,
  name,
  created_by: alice.id,
  owner_id: alice.id,
  members: [alice.id, bob.id],
  archived: false,
  classification: 'restricted',
  ...extra,
});

export const TEAM_LAB = 'team-lab-0000';
export const TEAM_SC = 'team-sc-0000';

export function makeSnapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    workspace: {
      id: 'workspace-1',
      host_uid: 1000,
      mode: 'private',
      institution_id: 'ucsf',
      policy_epoch: 1,
    },
    actor: alice,
    principals: [alice, bob],
    teams: [
      {
        id: TEAM_LAB,
        name: 'Analysis Lab',
        created_by: alice.id,
        members: [alice.id, bob.id],
        general_channel_id: 'chan-general',
      },
      {
        id: TEAM_SC,
        name: 'single-cell',
        created_by: alice.id,
        members: [alice.id],
        general_channel_id: 'chan-intro',
      },
    ],
    channels: [
      channel('chan-general', TEAM_LAB, 'general'),
      channel('chan-methods', TEAM_LAB, 'methods'),
      channel('chan-raw', TEAM_LAB, 'raw-data'),
      channel('chan-old', TEAM_LAB, 'old-notes', { archived: true }),
      channel('chan-intro', TEAM_SC, 'intro'),
    ],
    invitations: [],
    runs: [],
    unread: { 'chan-raw': 3 },
    ...overrides,
  };
}

export type ControllerOverrides = Partial<CrewController>;

/** A verified, connected, private controller for `alice`, the host, reading `#methods`. */
export function makeController(overrides: ControllerOverrides = {}): CrewController {
  const snapshot = 'snapshot' in overrides ? (overrides.snapshot ?? null) : makeSnapshot();
  const base: CrewController = {
    connections: [connection],
    connectionId: connection.id,
    connection,
    connectionsState: 'loaded',
    selectConnection: vi.fn(),
    saveConnection: vi.fn(),
    updateConnection: vi.fn(async () => connection),
    removeConnection: vi.fn(async () => {}),
    prepareHostingDevice: vi.fn(),
    connect: vi.fn(async () => {}),
    disconnect: vi.fn(async () => {}),
    lastConnectFailure: null,
    reportConnectFailure: vi.fn(),

    snapshot,
    lastVerified: null,
    observedPrivacy: snapshot
      ? { connectionId: connection.id, mode: 'private', institutionId: 'ucsf', policyEpoch: 1 }
      : null,
    runs: [],
    messages: [],
    messagesLoaded: true,
    historyBefore: null,
    labels: null,
    refreshError: null,
    refresh: vi.fn(async () => {}),
    loadOlder: vi.fn(),
    jumpToLatest: vi.fn(),

    teamId: TEAM_LAB,
    channelId: 'chan-methods',
    team: null,
    channel: null,
    selectTeam: vi.fn(),
    selectChannel: vi.fn(),

    act: vi.fn(async (_source, _key, fn) => {
      try {
        return await fn();
      } catch {
        return undefined;
      }
    }) as CrewController['act'],
    error: null,
    errorSlotFor: vi.fn(() => false),
    registerErrorSlot: vi.fn(() => () => {}),
    reportError: vi.fn(),
    dismissError: vi.fn(),
    isPending: vi.fn(() => false),
    busy: false,
    request: vi.fn(async () => undefined) as CrewController['request'],
    mutate: vi.fn(async () => undefined) as CrewController['mutate'],
    markRead: vi.fn(async () => {}),

    draft: { body: '', attachments: [], references: [] },
    setBody: vi.fn(),
    addAttachment: vi.fn(),
    removeAttachment: vi.fn(),
    addReference: vi.fn(),
    removeReference: vi.fn(),
    contextChannels: [],
    setContextChannels: vi.fn(),
    send: vi.fn(async () => {}),
    clearBodyIfEquals: vi.fn(),

    startOwnedRun: vi.fn(async () => true),
    unknownRunDestination: null,
    inspectedPriorRun: false,
    setInspectedPriorRun: vi.fn(),
    cancelRun: vi.fn(async () => {}),

    grantSessionId: null,
    grantSession: vi.fn(async () => {}),

    signIn: { open: false, reason: null },
    openSignIn: vi.fn(),
    closeSignIn: vi.fn(),
    onSignedIn: vi.fn(),

    ui: { dialog: null, pane: null },
    openDialog: vi.fn(),
    closeDialog: vi.fn(),
    openPane: vi.fn(),
    closePane: vi.fn(),
    subscribeSurfaceReset: vi.fn(() => () => {}),

    joinStatus: null,
    setJoinStatus: vi.fn(),

    status: snapshot ? 'connected' : 'checking',
    screen: snapshot ? 'channel' : 'checking',
    effectivePrivacy: snapshot ? 'private' : null,
    isHost: true,
  };
  return { ...base, ...overrides };
}

export function renderWithCrew(
  ui: ReactElement,
  controller: CrewController = makeController()
): RenderResult & { controller: CrewController; update(next: CrewController): void } {
  const view = render(
    <CrewControllerProvider controller={controller}>{ui}</CrewControllerProvider>
  );
  return {
    ...view,
    controller,
    update: (next: CrewController) =>
      view.rerender(<CrewControllerProvider controller={next}>{ui}</CrewControllerProvider>),
  };
}
