import type { ReactNode } from 'react';
import type { Channel, Snapshot } from '../crewApi';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import type { CrewController, ErrorSource } from '../state/types';

/**
 * A stand-in `CrewController` for the composer and files tests: a verified `#general` in a
 * private connection, with every action a no-op the test can replace. Test-only: nothing in
 * the app imports it.
 */
export const testChannel: Channel = {
  id: 'channel-1',
  team_id: 'team-1',
  name: 'general',
  created_by: 'person-1',
  owner_id: 'person-1',
  members: ['person-1'],
  archived: false,
  classification: 'restricted',
};

export const testSnapshot: Snapshot = {
  workspace: { id: 'workspace-1', host_uid: 1000, mode: 'private', policy_epoch: 1 },
  actor: { id: 'person-1', uid: 1000, username: 'alice', nickname: 'Alice' } as Snapshot['actor'],
  principals: [],
  teams: [],
  channels: [testChannel],
  invitations: [],
  runs: [],
};

const noop = () => undefined;
const resolved = async () => undefined;

export function crewTestController(overrides: Partial<CrewController> = {}): CrewController {
  const base: CrewController = {
    connections: [],
    connectionId: 'connection-1',
    connection: null,
    connectionsState: 'loaded',
    selectConnection: noop,
    saveConnection: async () => {
      throw new Error('not in this test');
    },
    updateConnection: async () => {
      throw new Error('not in this test');
    },
    removeConnection: resolved,
    prepareHostingDevice: async () => {
      throw new Error('not in this test');
    },
    connect: resolved,
    disconnect: resolved,
    lastConnectFailure: null,
    reportConnectFailure: noop,

    snapshot: testSnapshot,
    lastVerified: null,
    observedPrivacy: {
      connectionId: 'connection-1',
      mode: 'private',
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
    runs: [],
    messages: [],
    messagesLoaded: true,
    historyBefore: null,
    labels: null,
    refreshError: null,
    refresh: resolved,
    loadOlder: noop,
    jumpToLatest: noop,

    teamId: 'team-1',
    channelId: testChannel.id,
    team: null,
    channel: testChannel,
    selectTeam: noop,
    selectChannel: noop,

    act: async (_source, _key, fn) => {
      try {
        return await fn();
      } catch {
        return undefined;
      }
    },
    error: null,
    errorSlotFor: (source: ErrorSource) => source === 'composer',
    registerErrorSlot: () => noop,
    reportError: noop,
    dismissError: noop,
    isPending: () => false,
    busy: false,
    request: async () => {
      throw new Error('not in this test');
    },
    mutate: async () => {
      throw new Error('not in this test');
    },
    markRead: resolved,

    draft: { body: '', attachments: [], references: [] },
    setBody: noop,
    addAttachment: noop,
    removeAttachment: noop,
    addReference: noop,
    removeReference: noop,
    contextChannels: [],
    setContextChannels: noop,
    send: resolved,
    clearBodyIfEquals: noop,

    startOwnedRun: async () => false,
    unknownRunDestination: null,
    inspectedPriorRun: false,
    setInspectedPriorRun: noop,
    cancelRun: resolved,

    grantSessionId: null,
    grantSession: resolved,

    signIn: { open: false, reason: null },
    openSignIn: noop,
    closeSignIn: noop,
    onSignedIn: noop,

    ui: { dialog: null, pane: null },
    openDialog: noop,
    closeDialog: noop,
    openPane: noop,
    closePane: noop,
    subscribeSurfaceReset: () => noop,

    joinStatus: null,
    setJoinStatus: noop,

    status: 'connected',
    screen: 'channel',
    effectivePrivacy: 'private',
    isHost: true,
  };
  return { ...base, ...overrides };
}

export function CrewTestProvider({
  controller,
  children,
}: {
  controller: CrewController;
  children: ReactNode;
}) {
  return <CrewControllerProvider controller={controller}>{children}</CrewControllerProvider>;
}
