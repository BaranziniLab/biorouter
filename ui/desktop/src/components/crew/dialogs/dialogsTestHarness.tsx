/**
 * TEST-ONLY: a Crew controller for the dialog tests, so each dialog can be rendered alone.
 *
 * It is not a mock of the controller's behavior where that behavior decides what a dialog shows:
 * errors and pending keys come from the real `useCrewActions` (so an error renders in exactly the
 * slot the real resolver picks), and dialog intents from the real `useCrewSurfaces` (so `mutate`
 * closes the dialog as the real one does). Everything that would reach the daemon is a spy.
 */
import { render, type RenderResult } from '@testing-library/react';
import { useRef, type ReactNode } from 'react';
import { afterAll, beforeAll, vi, type Mock } from 'vitest';
import type { CrewConnection, Snapshot } from '../crewApi';
import { useCrewActions } from '../state/crewActions';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import { useCrewSurfaces } from '../state/crewSurfaces';
import type { CrewController, DialogIntent } from '../state/types';

export const connection: CrewConnection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'alice@hpc.example.edu',
  port: 22,
  identity_file: undefined,
  proxy_jump: undefined,
  socket_path: '/tmp/crew-1000-abc/broker.sock',
  owner_uid: 1000,
  workspace_id: '11111111-2222-4333-8444-555555555555',
  workspace_public_key: 'ab'.repeat(32),
  public_key: 'cd'.repeat(32),
  device_id: 'device-1',
  cluster_connection_id: 'cluster-1',
  mode: 'private',
  policy_epoch: 1,
  institution_id: 'ucsf',
  status: 'connected',
  remote_execution: false,
};

export const alice = {
  id: 'person-alice',
  uid: 1000,
  username: 'alice',
  nickname: 'Alice Chen',
  display_name: 'Alice Chen',
};
export const bob = { id: 'person-bob', uid: 1001, username: 'bob', nickname: 'Bob Lee' };
export const carol = { id: 'person-carol', uid: 1002, username: 'carol', nickname: 'Carol Diaz' };
export const dan = { id: 'person-dan', uid: 1003, username: 'dan', nickname: 'Dan Wu' };

export function makeSnapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    workspace: {
      id: 'workspace-1',
      host_uid: 1000,
      mode: 'private',
      policy_epoch: 1,
      institution_id: 'ucsf',
      host_principal_id: alice.id,
      name: 'lab',
    },
    actor: alice,
    principals: [alice, bob, carol, dan],
    teams: [
      {
        id: 'team-1',
        name: 'Analysis Lab',
        created_by: alice.id,
        members: [alice.id, bob.id, carol.id],
        general_channel_id: 'channel-general',
      },
    ],
    channels: [
      {
        id: 'channel-general',
        team_id: 'team-1',
        name: 'general',
        created_by: alice.id,
        owner_id: alice.id,
        members: [alice.id, bob.id],
        archived: false,
        classification: 'restricted',
      },
    ],
    invitations: [],
    runs: [],
    ...overrides,
  };
}

export interface FakeCrewOptions {
  connections?: CrewConnection[];
  snapshot?: Snapshot | null;
  /** The broker's answer to `request`/`mutate`, by method. Throw to refuse. */
  request?: (method: string, params: Record<string, unknown>) => unknown;
  dialog?: DialogIntent | null;
}

export interface FakeCrew {
  request: Mock;
  updateConnection: Mock;
  removeConnection: Mock;
  refresh: Mock;
  cancelRun: Mock;
  selectTeam: Mock;
  selectChannel: Mock;
  addReference: Mock;
  /** The controller the last render provided. */
  current(): CrewController;
}

function FakeCrewProvider({
  options,
  spies,
  holder,
  children,
}: {
  options: FakeCrewOptions;
  spies: Omit<FakeCrew, 'current'>;
  holder: { current: CrewController | null };
  children: ReactNode;
}) {
  const actions = useCrewActions();
  const surfaces = useCrewSurfaces();
  const opened = useRef(false);
  if (!opened.current && options.dialog) {
    opened.current = true;
    surfaces.openDialog(options.dialog);
  }
  const connections = options.connections ?? [connection];
  const snapshot = options.snapshot === undefined ? makeSnapshot() : options.snapshot;
  const selected = connections[0] ?? null;
  const request = (method: string, params: Record<string, unknown> = {}, opts?: object) =>
    spies.request(method, params, opts);
  const controller = {
    connections,
    connectionId: selected?.id ?? '',
    connection: selected,
    connectionsState: 'loaded',
    snapshot,
    lastVerified: null,
    observedPrivacy: snapshot
      ? {
          connectionId: selected?.id ?? '',
          mode: selected?.mode ?? 'private',
          institutionId: selected?.institution_id ?? null,
          policyEpoch: 1,
        }
      : null,
    labels: null,
    teamId: 'team-1',
    channelId: 'channel-general',
    team: snapshot?.teams[0] ?? null,
    channel: snapshot?.channels[0] ?? null,
    act: actions.act,
    error: actions.error,
    errorSlotFor: actions.errorSlotFor,
    registerErrorSlot: actions.registerErrorSlot,
    reportError: actions.reportError,
    dismissError: actions.dismissError,
    isPending: actions.isPending,
    busy: actions.busy,
    request,
    mutate: async (method: string, params: Record<string, unknown>) => {
      const result = await request(method, params, { mutation: true });
      await spies.refresh();
      surfaces.resetSurfaces('mutated');
      return result;
    },
    updateConnection: spies.updateConnection,
    removeConnection: spies.removeConnection,
    refresh: spies.refresh,
    cancelRun: spies.cancelRun,
    selectTeam: spies.selectTeam,
    selectChannel: spies.selectChannel,
    addReference: spies.addReference,
    ui: surfaces.ui,
    openDialog: surfaces.openDialog,
    closeDialog: surfaces.closeDialog,
    openPane: surfaces.openPane,
    closePane: surfaces.closePane,
    subscribeSurfaceReset: surfaces.subscribeSurfaceReset,
    isHost: true,
  } as unknown as CrewController;
  holder.current = controller;
  return <CrewControllerProvider controller={controller}>{children}</CrewControllerProvider>;
}

/** Render `ui` under a fake controller. `ui` may be a function of the controller. */
export function renderWithCrew(
  ui: ReactNode | ((crew: CrewController) => ReactNode),
  options: FakeCrewOptions = {}
): RenderResult & { crew: FakeCrew } {
  const holder: { current: CrewController | null } = { current: null };
  const spies = {
    request: vi.fn(async (method: string, params: Record<string, unknown>) =>
      options.request ? options.request(method, params) : {}
    ),
    updateConnection: vi.fn(async (_id: string, body: object) => ({ ...connection, ...body })),
    removeConnection: vi.fn(async () => {}),
    refresh: vi.fn(async () => {}),
    cancelRun: vi.fn(async () => {}),
    selectTeam: vi.fn(),
    selectChannel: vi.fn(),
    addReference: vi.fn(),
  };
  const Consumer = () => <>{typeof ui === 'function' ? ui(holder.current!) : ui}</>;
  const result = render(
    <FakeCrewProvider options={options} spies={spies} holder={holder}>
      <Consumer />
    </FakeCrewProvider>
  );
  return { ...result, crew: { ...spies, current: () => holder.current! } };
}

/** The requests the fake broker received for `method`. */
export function requestsFor(crew: FakeCrew, method: string): Record<string, unknown>[] {
  return crew.request.mock.calls
    .filter(([called]) => called === method)
    .map(([, params]) => params as Record<string, unknown>);
}

/**
 * jsdom has no `ResizeObserver`, and Radix mounts one for a switch, a popover or a menu. Call at
 * the top of a spec file that renders one.
 */
export function installResizeObserverStub(): void {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  beforeAll(() => {
    vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  });
  afterAll(() => {
    vi.unstubAllGlobals();
  });
}
