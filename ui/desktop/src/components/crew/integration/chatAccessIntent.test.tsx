import { act, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  chatAccessRoute,
  chatAccessRouteState,
  forgetChatAccessIntents,
} from '../access/ChatConnectNote';
import { accessCopy } from '../access/copy';
import { CREW_APP_OPTIONS } from '../CrewApp';
import { CrewLayout } from '../layout/CrewLayout';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';
import { useCrewController } from '../state/useCrewController';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { rememberLastChannel } from '../state/draftStash';
import {
  connection,
  ids,
  installDaemon,
  mocked,
  richMessages,
  richSnapshot,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [{ name: 'fixture-provider', is_configured: true }],
  read: async () => '',
  getProviderModels: async () => ['fixture-model'],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * Q2-10 (live QA round 2): the ordinary chat's chip and its "Grant access again" promise the chat's
 * access pane in one hop, and three live arrivals showed only the note. The pieces an arrival waits
 * for — the saved connections, the first verified view, the channel settling, the chat's grants —
 * arrive in whatever order the daemon and the network give them, and each of the first three can
 * reset the surfaces and close a pane opened before it. Every order must end with the pane open, on
 * the grant's own channel.
 */

const CHAT = 'chat-plot-review';
const ARRIVAL = chatAccessRoute(CHAT);

function grantOnMethods(extra: Record<string, unknown> = {}) {
  return {
    session_id: CHAT,
    run_id: 'run-chat',
    connection_id: connection.id,
    channel_id: ids.methods,
    source_channels: [ids.methods],
    policy_epoch: 1,
    expired: false,
    kind: 'chat',
    session_name: 'Plot review',
    expires_at: Math.floor(Date.now() / 1000) + 3600,
    ...extra,
  };
}

/** A promise the test opens when the step it stands for should happen. */
function gate() {
  let open: () => void = () => {};
  const ready = new Promise<void>((resolve) => {
    open = resolve;
  });
  return { ready, open };
}

let controller: CrewController | null = null;

function CrewApp() {
  const crew = useCrewController(CREW_APP_OPTIONS);
  controller = crew;
  return (
    <CrewControllerProvider controller={crew}>
      <CrewLayout />
    </CrewControllerProvider>
  );
}

/** Arrive as the chat's chip or "Grant access again" sends the person: the route plus the intent. */
function arriveFromChat() {
  controller = null;
  return render(
    <MemoryRouter
      initialEntries={[
        {
          pathname: '/crew',
          search: ARRIVAL.slice('/crew'.length),
          state: chatAccessRouteState(),
        },
      ]}
    >
      <CrewApp />
    </MemoryRouter>
  );
}

interface Arrival {
  daemon: ScriptedDaemon;
  connections: ReturnType<typeof gate>;
  grants: ReturnType<typeof gate>;
}

/** A daemon whose connection list and grant list answer only when their gate opens. */
function scriptedArrival(grant: Record<string, unknown>): Arrival {
  const connections = gate();
  const grants = gate();
  const daemon = installDaemon({ hold: true });
  daemon.state.http = (path, method) => {
    if (path === '/connections' && method === 'GET')
      return connections.ready.then(() => ({ connections: daemon.state.connections }));
    if (path === `/connections/${connection.id}/grants` && method === 'GET')
      return grants.ready.then(() => ({ grants: [grant] }));
    return undefined;
  };
  return { daemon, connections, grants };
}

/** Let every answer already released settle into the view. */
async function settle() {
  for (let round = 0; round < 5; round += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

async function firstVerifiedFrame(daemon: ScriptedDaemon) {
  await waitFor(() => expect(mocked.observeCrew).toHaveBeenCalled());
  act(() => daemon.release());
}

function chatAccessPane(): HTMLElement {
  const aside = document.querySelector<HTMLElement>('aside.crew-pane');
  if (!aside) throw new Error('No details pane is mounted.');
  return aside;
}

async function expectPaneOpenOnMethods() {
  await waitFor(() =>
    expect(controller?.ui.pane).toEqual({ mode: 'chat-access', sessionId: CHAT })
  );
  expect(await screen.findByRole('textbox', { name: 'Message #methods' })).toBeInTheDocument();
  expect(chatAccessPane()).toHaveAttribute('data-state', 'open');
  // And it stays open once everything has arrived: no late reset closes it.
  await settle();
  expect(controller?.ui.pane).toEqual({ mode: 'chat-access', sessionId: CHAT });
  expect(controller?.channel?.id).toBe(ids.methods);
  return chatAccessPane();
}

describe('the one hop from a chat, whatever order Crew’s answers arrive in', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetChatAccessIntents();
  });

  it('connections, then the first verified view, then the channel settles, then the grants', async () => {
    const { daemon, connections, grants } = scriptedArrival(grantOnMethods());
    arriveFromChat();

    await act(async () => connections.open());
    await firstVerifiedFrame(daemon);
    // The channel settles on the team's first channel, with the chat's note still checking.
    expect(await screen.findByRole('textbox', { name: 'Message #general' })).toBeInTheDocument();
    expect(await screen.findByText(accessCopy.noteChecking)).toBeInTheDocument();
    expect(controller?.ui.pane).toBeNull();

    await act(async () => grants.open());
    const pane = await expectPaneOpenOnMethods();
    expect(await within(pane).findByText('“Plot review” can')).toBeInTheDocument();
  });

  it('the grants first, then the connections, then the first verified view', async () => {
    const { daemon, connections, grants } = scriptedArrival(grantOnMethods());
    arriveFromChat();

    await act(async () => grants.open());
    await act(async () => connections.open());
    await firstVerifiedFrame(daemon);

    const pane = await expectPaneOpenOnMethods();
    expect(await within(pane).findByText('“Plot review” can')).toBeInTheDocument();
  });

  it('connections, then the grants, then the first verified view', async () => {
    const { daemon, connections, grants } = scriptedArrival(grantOnMethods());
    arriveFromChat();

    await act(async () => connections.open());
    await act(async () => grants.open());
    await settle();
    await firstVerifiedFrame(daemon);

    await expectPaneOpenOnMethods();
  });

  it('reaches Allow for the grant’s channel from “Grant access again”, in one hop', async () => {
    const { daemon, connections, grants } = scriptedArrival(grantOnMethods({ expired: true }));
    arriveFromChat();

    await act(async () => connections.open());
    await firstVerifiedFrame(daemon);
    await act(async () => grants.open());

    const pane = await expectPaneOpenOnMethods();
    expect(
      await within(pane).findByRole('button', {
        name: accessCopy.allowChat('Plot review', '#methods'),
      })
    ).toBeInTheDocument();
    // Opening the consent granted nothing.
    expect(
      mocked.crewHttp.mock.calls.filter(([path]) => String(path).endsWith('/grant'))
    ).toHaveLength(0);
  });
});

const DAEMON_SENTENCE =
  'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.';

/** The observation a daemon gives a connection: refused while it is down, the view once it is up. */
function observeWhileConnected(isConnected: () => boolean, connectionIdSeen?: string[]) {
  mocked.observeCrew.mockImplementation(
    async (
      connectionId: string,
      channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      deliver: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      connectionIdSeen?.push(connectionId);
      if (!isConnected()) {
        deliver({ type: 'error', code: 'observation_refused', error: DAEMON_SENTENCE });
        return 'terminal';
      }
      deliver({
        type: 'state',
        connection_id: connectionId,
        connection_mode: 'private',
        connection_policy_epoch: 1,
        connection_institution_id: connection.institution_id,
        snapshot: richSnapshot(),
        runs: [],
        cursor: null,
      });
      if (channelId) {
        const messages = richMessages().filter((message) => message.channel_id === channelId);
        deliver({ type: 'messages', channel_id: channelId, messages, cursor: null, reset: true });
      }
      return 'terminal';
    }
  );
}

/**
 * Q3-08 (live QA round 3): "Connect in Crew" from a chat whose connection was down landed, once
 * connected, on #methods — the last channel visited — not on the chat's #general. An arrival from
 * an offline chat comes before any verified view exists; its intent waits for the first verified
 * view of the grant's connection and only then moves to the grant's channel. (Here the grant is on
 * #methods and the channel last visited is #general.)
 */
describe('an arrival from an offline chat lands on the chat’s channel once connected', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetChatAccessIntents();
    window.localStorage.clear();
  });

  it('waits through the offline screen, then selects the grant’s channel and opens its access', async () => {
    let saved: 'connected' | 'disconnected' = 'disconnected';
    const CONNECT = `/connections/${connection.id}/connect`;
    rememberLastChannel(connection.id, ids.general);
    const daemon = installDaemon();
    daemon.state.http = (path, method) => {
      if (path === '/connections' && method === 'GET')
        return { connections: [{ ...connection, status: saved }] };
      if (path === `/connections/${connection.id}/grants` && method === 'GET')
        return { grants: [grantOnMethods()] };
      if (path === CONNECT && method === 'POST') {
        saved = 'connected';
        return {};
      }
      return undefined;
    };
    observeWhileConnected(() => saved === 'connected');
    arriveFromChat();

    // The intent arrived before any verified view: nothing moves and nothing opens.
    await waitFor(() => expect(controller?.screen).toBe('offline'));
    await settle();
    expect(controller?.ui.pane).toBeNull();
    expect(controller?.channel).toBeNull();
    expect(
      mocked.crewHttp.mock.calls.filter(([path, method]) => path === CONNECT && method === 'POST')
    ).toHaveLength(0);

    // The person connects (the Connect button on the offline screen, or Crew acting on the chat's
    // "Connect in Crew"). The first verified view settles on #general, the channel last visited;
    // the intent then moves to the grant's #methods and opens the chat's access there.
    await act(async () => {
      await controller?.connect({ userInitiated: true });
    });
    const pane = await expectPaneOpenOnMethods();
    expect(await within(pane).findByText('“Plot review” can')).toBeInTheDocument();
    // Connecting granted nothing.
    expect(
      mocked.crewHttp.mock.calls.filter(([path]) => String(path).endsWith('/grant'))
    ).toHaveLength(0);
  });

  it('moves to the grant’s own connection first when Crew shows another one', async () => {
    const other = { ...connection, id: 'conn-0', name: 'Other lab' };
    const daemon = installDaemon({ connections: [other, connection] });
    daemon.state.http = (path, method) => {
      if (path === `/connections/${connection.id}/grants` && method === 'GET')
        return { grants: [grantOnMethods()] };
      if (path === `/connections/${other.id}/grants` && method === 'GET') return { grants: [] };
      return undefined;
    };
    const observed: string[] = [];
    observeWhileConnected(() => true, observed);
    arriveFromChat();

    await waitFor(() => expect(controller?.connectionId).toBe(connection.id));
    const pane = await expectPaneOpenOnMethods();
    expect(await within(pane).findByText('“Plot review” can')).toBeInTheDocument();
    expect(observed).toContain(connection.id);
  });
});

/** Arrive as `/crew` sends the person: the route alone, with no route state. */
function arriveByCommand() {
  controller = null;
  return render(
    <MemoryRouter initialEntries={[ARRIVAL]}>
      <CrewApp />
    </MemoryRouter>
  );
}

/**
 * Q3-28 (live QA round 3): `/crew` in a chat with no grant opens its consent at once, on the
 * channel Crew shows, rather than stopping at "Connect this chat to #general? [Review access]".
 * `/crew` carries no route state; its arrival stands in for the intent.
 */
describe('/crew from a chat with no grant', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetChatAccessIntents();
    window.localStorage.clear();
  });

  it('opens the consent once the view is verified, and grants nothing before Allow', async () => {
    const daemon = installDaemon({ hold: true });
    daemon.state.http = (path, method) => {
      if (path === `/connections/${connection.id}/grants` && method === 'GET')
        return { grants: [] };
      return undefined;
    };
    arriveByCommand();
    await settle();
    // No verified view yet: nothing opens.
    expect(controller?.ui.pane ?? null).toBeNull();

    await firstVerifiedFrame(daemon);
    await waitFor(() =>
      expect(controller?.ui.pane).toEqual({ mode: 'chat-access', sessionId: CHAT })
    );
    expect(await screen.findByRole('textbox', { name: 'Message #general' })).toBeInTheDocument();
    const pane = chatAccessPane();
    expect(pane).toHaveAttribute('data-state', 'open');
    const allow = await within(pane).findByRole('button', { name: accessCopy.allow });
    expect(allow).toBeInTheDocument();
    await settle();
    expect(controller?.ui.pane).toEqual({ mode: 'chat-access', sessionId: CHAT });
    // Q4-13: focus is on Allow, not on the pane's heading, where the browser drew a box that read
    // as a text field.
    expect(allow).toHaveFocus();
    expect(within(pane).getByRole('heading', { level: 2 })).not.toHaveFocus();
    // Q4-14: no "Connect this chat to #general? [Review access]" strip beside the consent.
    expect(screen.queryByTestId('crew-chat-connect-note')).toBeNull();
    expect(screen.queryByText(accessCopy.noteNone(null, '#general'))).toBeNull();
    expect(
      mocked.crewHttp.mock.calls.filter(([path]) => String(path).endsWith('/grant'))
    ).toHaveLength(0);
  });
});
