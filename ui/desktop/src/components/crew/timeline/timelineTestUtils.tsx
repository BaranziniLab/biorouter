import { render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterAll, beforeAll, vi } from 'vitest';
import type { Channel, CrewMessage, ObservedRun, Snapshot } from '../crewApi';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';

/**
 * Fixtures and a stand-in controller for the timeline's tests. The IDs are
 * UUID-shaped on purpose: every test can then assert that none of them reaches
 * the rendered page.
 */

export const ID = {
  alice: '11111111-1111-4111-8111-111111111111',
  bob: '22222222-2222-4222-8222-222222222222',
  carol: '33333333-3333-4333-8333-333333333333',
  gone: '99999999-9999-4999-8999-999999999999',
  team: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  general: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
  methods: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
  workspace: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
  run: 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee',
  runB: 'ffffffff-ffff-4fff-8fff-ffffffffffff',
  session: '12121212-1212-4212-8212-121212121212',
} as const;

/** A UUID, a 64-hex digest, or a fixture UID — none may appear in the default DOM. */
export const MACHINE_STRING =
  /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[0-9a-f]{64}|\b100[0-9]\b/i;

export const channel: Channel = {
  id: ID.general,
  team_id: ID.team,
  name: 'general',
  created_by: ID.alice,
  owner_id: ID.alice,
  members: [ID.alice, ID.bob],
  archived: false,
  classification: 'public_safe',
};

export function snapshotFor(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    workspace: { id: ID.workspace, host_uid: 1000, mode: 'private', policy_epoch: 1 },
    actor: { id: ID.alice, uid: 1000, username: 'alice', nickname: 'Alice Chen' },
    principals: [
      { id: ID.alice, uid: 1000, username: 'alice', nickname: 'Alice Chen' },
      { id: ID.bob, uid: 1001, username: 'bob', nickname: 'Bob Lee' },
      { id: ID.carol, uid: 1002, username: 'carol', nickname: 'carol' },
    ],
    teams: [
      {
        id: ID.team,
        name: 'Lab',
        created_by: ID.alice,
        members: [ID.alice, ID.bob, ID.carol],
        general_channel_id: ID.general,
      },
    ],
    channels: [channel],
    invitations: [],
    runs: [],
    ...overrides,
  };
}

let nextSequence = 1;

/** A message, by default from Bob in #general at 10:02 on 2026-09-22 (local time). */
export function message(overrides: Partial<CrewMessage> & { at?: Date } = {}): CrewMessage {
  const { at = new Date(2026, 8, 22, 10, 2), ...rest } = overrides;
  const sequence = String(nextSequence++).padStart(8, '0');
  return {
    id: `m-${sequence}`,
    sequence,
    channel_id: ID.general,
    actor_id: ID.bob,
    body: 'Counts are in.',
    created_at: Math.floor(at.getTime() / 1000),
    restricted: false,
    source_channels: [ID.general],
    attachments: [],
    ...rest,
  };
}

export function run(overrides: Partial<ObservedRun> = {}): ObservedRun {
  return {
    run_id: ID.run,
    channel_id: ID.general,
    session_id: ID.session,
    status: 'running',
    ...overrides,
  };
}

/** Every controller member, stubbed; tests override what they read or assert on. */
export function makeController(overrides: Partial<CrewController> = {}): CrewController {
  const snapshot = overrides.snapshot === undefined ? snapshotFor() : overrides.snapshot;
  return {
    connections: [],
    connectionId: 'conn-1',
    connection: null,
    connectionsState: 'loaded',
    selectConnection: vi.fn(),
    saveConnection: vi.fn(),
    updateConnection: vi.fn(),
    removeConnection: vi.fn(),
    prepareHostingDevice: vi.fn(),
    connect: vi.fn(),
    disconnect: vi.fn(),
    lastConnectFailure: null,
    reportConnectFailure: vi.fn(),
    snapshot,
    lastVerified: null,
    observedPrivacy: null,
    runs: [],
    messages: [],
    messagesLoaded: true,
    historyBefore: null,
    labels: null,
    refreshError: null,
    refresh: vi.fn(async () => {}),
    loadOlder: vi.fn(),
    jumpToLatest: vi.fn(),
    teamId: ID.team,
    channelId: ID.general,
    team: snapshot?.teams[0] ?? null,
    channel,
    selectTeam: vi.fn(),
    selectChannel: vi.fn(),
    act: vi.fn(async (_source, _key, fn) => fn()) as CrewController['act'],
    error: null,
    errorSlotFor: vi.fn(() => false),
    registerErrorSlot: vi.fn(() => () => {}),
    reportError: vi.fn(),
    dismissError: vi.fn(),
    isPending: vi.fn(() => false),
    busy: false,
    request: vi.fn(async () => ({})) as CrewController['request'],
    mutate: vi.fn(async () => ({})) as CrewController['mutate'],
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
    status: 'connected',
    screen: 'channel',
    effectivePrivacy: 'private',
    isHost: true,
    ...overrides,
  };
}

/** Where the router is now, so a test can see a navigation. */
function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location" hidden>{`${location.pathname}${location.search}`}</div>;
}

export function renderWithController(ui: ReactElement, controller: CrewController) {
  const result = render(
    <MemoryRouter initialEntries={['/crew']}>
      <CrewControllerProvider controller={controller}>
        <Routes>
          <Route path="*" element={ui} />
        </Routes>
        <LocationProbe />
      </CrewControllerProvider>
    </MemoryRouter>
  );
  return {
    ...result,
    rerenderWith(next: CrewController) {
      result.rerender(
        <MemoryRouter initialEntries={['/crew']}>
          <CrewControllerProvider controller={next}>
            <Routes>
              <Route path="*" element={ui} />
            </Routes>
            <LocationProbe />
          </CrewControllerProvider>
        </MemoryRouter>
      );
    },
  };
}

/**
 * jsdom has no `ResizeObserver`, and Radix mounts one the moment a pointer
 * enters the scroll area (its hover scrollbar) or a tooltip or menu opens. Call
 * at the top of a spec file that clicks inside the timeline.
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

/**
 * Row actions are revealed by `:hover` and `:focus-within` (`timeline.css`,
 * applied here because vitest loads CSS), and jsdom evaluates neither, so a
 * pointer click on one needs the pointer-events check off.
 */
export const pointerAnywhere = { pointerEventsCheck: 0 } as const;
