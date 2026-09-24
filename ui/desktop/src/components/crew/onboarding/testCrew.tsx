import { render, type RenderResult } from '@testing-library/react';
import type { ReactElement } from 'react';
import { vi } from 'vitest';
import type { CrewConnection, Snapshot } from '../crewApi';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';

/**
 * Test support for the onboarding and sign-in areas: a controller whose every action is a spy,
 * and a render that can move the controller to a new state the way the real one would re-render.
 * Not a test file itself (it does not match `*.test.tsx`).
 */

export const WORKSPACE_KEY = 'a'.repeat(64);
export const DEVICE_KEY = 'b'.repeat(64);

export function fakeConnection(overrides: Partial<CrewConnection> = {}): CrewConnection {
  return {
    id: 'conn-1',
    name: 'lab',
    ssh_target: 'bob@hpc.ucsf.edu',
    port: 22,
    socket_path: '/tmp/crew-1000-abc/broker.sock',
    owner_uid: 1000,
    workspace_id: '11111111-2222-3333-4444-555555555555',
    workspace_public_key: WORKSPACE_KEY,
    public_key: DEVICE_KEY,
    device_id: 'device-1',
    cluster_connection_id: 'cluster-1',
    remote_execution: false,
    mode: 'private',
    institution_id: 'ucsf',
    policy_epoch: 1,
    status: 'connected',
    ...overrides,
  };
}

export function fakeSnapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    workspace: {
      id: 'workspace-1',
      host_uid: 1000,
      mode: 'private',
      institution_id: 'ucsf',
      policy_epoch: 1,
      host_principal_id: 'p-alice',
      name: 'lab',
    },
    actor: { id: 'p-bob', uid: 1001, username: 'bob', nickname: 'bob' },
    principals: [
      { id: 'p-alice', uid: 1000, username: 'alice', nickname: 'Alice Chen' },
      { id: 'p-bob', uid: 1001, username: 'bob', nickname: 'bob' },
    ],
    teams: [],
    channels: [],
    invitations: [],
    runs: [],
    ...overrides,
  };
}

export function makeCrew(overrides: Partial<CrewController> = {}): CrewController {
  const base: CrewController = {
    connections: [],
    connectionId: '',
    connection: null,
    connectionsState: 'loaded',
    selectConnection: vi.fn(),
    saveConnection: vi.fn(),
    updateConnection: vi.fn(),
    removeConnection: vi.fn(),
    prepareHostingDevice: vi.fn(),
    connect: vi.fn().mockResolvedValue(undefined),
    disconnect: vi.fn().mockResolvedValue(undefined),
    lastConnectFailure: null,
    reportConnectFailure: vi.fn(),

    snapshot: null,
    lastVerified: null,
    observedPrivacy: null,
    runs: [],
    messages: [],
    messagesLoaded: false,
    historyBefore: null,
    labels: null,
    refreshError: null,
    refresh: vi.fn().mockResolvedValue(undefined),
    loadOlder: vi.fn(),
    jumpToLatest: vi.fn(),

    teamId: '',
    channelId: '',
    team: null,
    channel: null,
    selectTeam: vi.fn(),
    selectChannel: vi.fn(),

    // The real `act` records a failure and resolves undefined; it never throws.
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
    request: vi.fn().mockResolvedValue({}) as CrewController['request'],
    mutate: vi.fn().mockResolvedValue({}) as CrewController['mutate'],
    markRead: vi.fn().mockResolvedValue(undefined),

    draft: { body: '', attachments: [], references: [] },
    setBody: vi.fn(),
    addAttachment: vi.fn(),
    removeAttachment: vi.fn(),
    addReference: vi.fn(),
    removeReference: vi.fn(),
    contextChannels: [],
    setContextChannels: vi.fn(),
    send: vi.fn().mockResolvedValue(undefined),
    clearBodyIfEquals: vi.fn(),

    startOwnedRun: vi.fn().mockResolvedValue(false),
    unknownRunDestination: null,
    inspectedPriorRun: false,
    setInspectedPriorRun: vi.fn(),
    cancelRun: vi.fn().mockResolvedValue(undefined),

    grantSessionId: null,
    grantSession: vi.fn().mockResolvedValue(undefined),

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

    status: null,
    screen: 'welcome',
    effectivePrivacy: null,
    isHost: false,
  };
  return { ...base, ...overrides };
}

export interface CrewRender extends RenderResult {
  /** The controller as it stands. */
  crew(): CrewController;
  /** Move the controller to a new state (spies are kept) and re-render. */
  update(patch: Partial<CrewController>): CrewController;
}

export function renderWithCrew(ui: ReactElement, crew: CrewController = makeCrew()): CrewRender {
  let current = crew;
  const wrap = (controller: CrewController) => (
    <CrewControllerProvider controller={controller}>{ui}</CrewControllerProvider>
  );
  const result = render(wrap(current));
  return {
    ...result,
    crew: () => current,
    update(patch) {
      current = { ...current, ...patch };
      result.rerender(wrap(current));
      return current;
    },
  };
}
