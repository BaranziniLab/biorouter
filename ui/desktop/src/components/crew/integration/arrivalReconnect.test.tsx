import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connectionBarCopy } from '../channel/copy';
import { emptyCopy } from '../onboarding/copy';
import { crewObservationCopy, crewStatusCopy } from '../state/copy';
import { chatAccessRouteState } from '../access/ChatConnectNote';
import {
  CREW_CONNECT_ROUTE_KEY,
  OFFLINE_FOLLOW_INTERVAL_MS,
  QUIET_REOBSERVE_GAPS_MS,
} from '../state/useCrewConnections';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  installDaemon,
  mocked,
  renderCrew,
  richMessages,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * Live QA round 2, Q2-01 (five critics), and its round-2 review (SECURITY-SENSITIVE). Coming back
 * to Crew after a while away used to show "Live updates for chen-lab stopped" and take Retry, then
 * Connect. The daemon now keeps an idle bridge alive and dials a dropped one again by itself
 * (D-KEEPALIVE), and never after a Disconnect. The renderer never connects by itself: the daemon
 * ends observation with the same `observation_refused` for a dropped bridge and for a Disconnect
 * made in a terminal or another window, which this window cannot tell apart. So it reads the saved
 * record again: still (or again) connected, it observes again quietly, a few times with growing
 * gaps; disconnected, it shows the offline screen, whose Connect is the person's, and follows the
 * record every 15 s for up to an hour (Q4-02) to pick up the daemon's own re-dial.
 */

const DAEMON_SENTENCE =
  'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.';
const CONNECT = `/connections/${connection.id}/connect`;
const DISCONNECT = `/connections/${connection.id}/disconnect`;

/** Remembers whether anything on the page ever said updates stopped, however briefly. */
function watchForStopped() {
  const seen = { stopped: false };
  const check = () => {
    if (document.body.textContent?.includes(crewObservationCopy.updatesStopped('lab')))
      seen.stopped = true;
  };
  const observer = new MutationObserver(check);
  observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  check();
  return { seen, stop: () => observer.disconnect() };
}

function statusRow(): HTMLElement {
  return within(screen.getByRole('navigation', { name: 'Crew' })).getByRole('status', {
    name: /connection status/i,
  });
}

let daemon: ScriptedDaemon;
/** The saved record's status the daemon answers with, changed as the story goes. */
let saved: 'connected' | 'disconnected';
let watcher: ReturnType<typeof watchForStopped> | null = null;

function connects(): number {
  return mocked.crewHttp.mock.calls.filter(
    ([path, method]) => path === CONNECT && method === 'POST'
  ).length;
}

/**
 * The daemon now calls the connection disconnected — a bridge it could not dial again, or a
 * Disconnect made anywhere — and the observation ends as the daemon ends it.
 */
function dropTheBridge() {
  saved = 'disconnected';
  act(() =>
    daemon.emit({ type: 'error', code: 'observation_refused', clear: true, error: DAEMON_SENTENCE })
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  saved = 'connected';
  daemon = installDaemon({
    messages: richMessages(),
    http: (path, method) => {
      if (path === '/connections' && method === 'GET')
        return { connections: [{ ...connection, status: saved }] };
      if (path === CONNECT && method === 'POST') {
        saved = 'connected';
        return {};
      }
      if (path === DISCONNECT && method === 'POST') {
        saved = 'disconnected';
        return {};
      }
      return undefined;
    },
  });
});
afterEach(() => {
  watcher?.stop();
  watcher = null;
});

/** Advance the clock by `ms`, running every timer due meanwhile. */
async function wait(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** A drop the daemon repaired before this window looked: the record still says connected. */
function dropRepairedBridge() {
  act(() =>
    daemon.emit({ type: 'error', code: 'observation_refused', clear: true, error: DAEMON_SENTENCE })
  );
}

function reads(): number {
  return mocked.crewHttp.mock.calls.filter(
    ([path, method]) => path === '/connections' && (method ?? 'GET') === 'GET'
  ).length;
}

describe('coming back after the SSH bridge dropped (Q2-01): Crew never connects by itself', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('observes again quietly when the daemon already dialled the bridge again: no Retry, no connect', async () => {
    renderCrew();
    const composer = await channelReady();
    fireEvent.change(composer, { target: { value: 'half-written reply' } });
    watcher = watchForStopped();
    const observations = mocked.observeCrew.mock.calls.length;
    // The saved record is read again before anything else; hold that read to look at the page.
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const answer = daemon.state.http!;
    daemon.state.http = (path, method, body) =>
      path === '/connections' && method === 'GET'
        ? held.then(() => answer(path, method, body))
        : answer(path, method, body);

    dropRepairedBridge();

    // Nothing verified stays on screen; the status says what is happening, not that it stopped.
    expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();
    await waitFor(() => expect(currentCrew().status).toBe('reconnecting'));
    expect(within(statusRow()).getByText(crewStatusCopy.reconnecting)).toBeInTheDocument();
    expect(currentCrew().screen).toBe('connecting');
    await act(async () => {
      release();
    });

    // Observed again once, and the next verified view hands the draft back.
    expect(await channelReady()).toHaveValue('half-written reply');
    expect(mocked.observeCrew.mock.calls.length).toBe(observations + 1);
    expect(connects()).toBe(0);
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().refreshError).toBeNull();
    expect(currentCrew().reconnecting).toBe(false);
    expect(currentCrew().signIn.open).toBe(false);
    expect(watcher.seen.stopped).toBe(false);
    expect(
      screen.queryByRole('button', { name: connectionBarCopy.retryName })
    ).not.toBeInTheDocument();
  });

  it.each([
    ['`biorouter crew disconnect` in a terminal'],
    ['Disconnect in another window'],
    ['an edit of the connection, which disconnects it'],
  ])('never undoes a Disconnect made elsewhere: %s', async () => {
    renderCrew();
    const composer = await channelReady();
    fireEvent.change(composer, { target: { value: 'half-written reply' } });

    // From this window, every one of them is exactly this: the record says disconnected, and the
    // observation ends with the daemon's generic code.
    dropTheBridge();

    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    expect(currentCrew().status).toBe('offline');
    expect(currentCrew().reconnecting).toBe(false);
    expect(connects()).toBe(0);

    // A quarter of an hour of following the saved record (Q4-02): it is only ever read, every
    // 15 s, and still nothing connects.
    const before = reads();
    await wait(15 * 60_000);
    expect(reads()).toBe(before + (15 * 60_000) / OFFLINE_FOLLOW_INTERVAL_MS);
    expect(connects()).toBe(0);
    expect(currentCrew().screen).toBe('offline');

    // The one action that connects is the person's own, and the draft comes back with it.
    fireEvent.click(screen.getByRole('button', { name: emptyCopy.offlineAction('Fixture') }));
    expect(await channelReady()).toHaveValue('half-written reply');
    expect(connects()).toBe(1);
  });

  it('never connects after this window’s own Disconnect either', async () => {
    renderCrew();
    await channelReady();
    await act(async () => {
      await currentCrew().disconnect();
    });
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    await wait(15 * 60_000);
    expect(connects()).toBe(0);
    expect(currentCrew().screen).toBe('offline');
  });

  it('picks up the daemon’s own re-dial without a click, and still never connects', async () => {
    renderCrew();
    const composer = await channelReady();
    fireEvent.change(composer, { target: { value: 'half-written reply' } });

    dropTheBridge();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    // The daemon's first retry after a network failure (20 s later) got through.
    saved = 'connected';
    await wait(OFFLINE_FOLLOW_INTERVAL_MS);

    expect(await channelReady()).toHaveValue('half-written reply');
    expect(connects()).toBe(0);
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().refreshError).toBeNull();
    // Nothing more is read once it is back.
    const after = reads();
    await wait(15 * 60_000);
    expect(reads()).toBe(after);
  });

  it('says so, with Retry, when the same end comes again sooner than the growing gap', async () => {
    renderCrew();
    await channelReady();
    dropRepairedBridge();
    await channelReady();

    await wait(QUIET_REOBSERVE_GAPS_MS[1]! - 5_000);
    dropRepairedBridge();

    const bar = screen.getByTestId('crew-connection-bar');
    expect(await within(bar).findByRole('alert')).toHaveTextContent(
      crewObservationCopy.updatesStopped('lab')
    );
    expect(within(bar).getByRole('button', { name: connectionBarCopy.retryName })).toBeEnabled();
    expect(connects()).toBe(0);
    expect(currentCrew().status).toBe('updates-unavailable');
    expect(currentCrew().reconnecting).toBe(false);
  });

  it('shows a bridge that keeps dropping after three quiet re-observations, however spaced', async () => {
    renderCrew();
    await channelReady();
    const bar = () => screen.getByTestId('crew-connection-bar');

    // Three drops, each after a gap the back-off allows: each is picked up quietly.
    for (const gap of [0, 20_000, 60_000]) {
      await wait(gap);
      dropRepairedBridge();
      await channelReady();
      expect(within(bar()).queryByRole('alert')).toBeNull();
    }

    // The fourth, minutes later but inside the window, is a failure worth seeing.
    await wait(5 * 60_000);
    dropRepairedBridge();
    expect(await within(bar()).findByRole('alert')).toHaveTextContent(
      crewObservationCopy.updatesStopped('lab')
    );
    expect(within(bar()).getByRole('button', { name: connectionBarCopy.retryName })).toBeEnabled();
    expect(connects()).toBe(0);
    expect(currentCrew().status).toBe('updates-unavailable');
  });

  it('does not connect by itself after an app restart, whose first list says disconnected', async () => {
    saved = 'disconnected';
    mocked.observeCrew.mockImplementation(
      async (
        _connectionId: string,
        _channelId: string | undefined,
        _after: string | null,
        signal: AbortSignal,
        deliver: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        deliver({ type: 'error', code: 'observation_refused', error: DAEMON_SENTENCE });
        return 'terminal';
      }
    );
    renderCrew();

    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    expect(currentCrew().status).toBe('offline');
    expect(connects()).toBe(0);
    expect(currentCrew().reconnecting).toBe(false);
  });
});

/**
 * Live QA round 3, Q3-08 (SECURITY-SENSITIVE). A chat's "Connect in Crew" is the person's click, so
 * Crew connects the connection it names on arrival — once per intent, as the Connect button would —
 * instead of opening the offline screen for a second click. Nothing else here connects by itself.
 */
describe('arriving from a chat’s “Connect in Crew” (Q3-08)', () => {
  function refuseWhileDisconnected() {
    const answer = mocked.observeCrew.getMockImplementation()!;
    mocked.observeCrew.mockImplementation(
      async (
        connectionId: string,
        channelId: string | undefined,
        after: string | null,
        signal: AbortSignal,
        deliver: (frame: unknown) => void
      ) => {
        if (signal.aborted) return 'terminal';
        if (saved === 'disconnected') {
          deliver({
            type: 'error',
            code: 'observation_refused',
            clear: true,
            error: DAEMON_SENTENCE,
          });
          return 'terminal';
        }
        return answer(connectionId, channelId, after, signal, deliver);
      }
    );
  }
  const fromChat = (state = chatAccessRouteState()) => ({
    pathname: '/crew',
    search: '?sessionId=chat-1',
    state: { ...state, [CREW_CONNECT_ROUTE_KEY]: connection.id },
  });

  it('connects once and opens the channel, with no offline screen to click through', async () => {
    saved = 'disconnected';
    refuseWhileDisconnected();
    const entry = fromChat();
    const first = renderCrew(entry);

    await channelReady();
    expect(connects()).toBe(1);
    expect(currentCrew().status).toBe('connected');
    expect(
      screen.queryByRole('button', { name: emptyCopy.offlineAction('Fixture') })
    ).not.toBeInTheDocument();

    // Disconnected again later, the same history entry never connects it a second time.
    first.unmount();
    saved = 'disconnected';
    renderCrew(entry);
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    expect(
      await screen.findByRole('button', { name: emptyCopy.offlineAction('Fixture') })
    ).toBeInTheDocument();
    expect(connects()).toBe(1);
  });

  it('connects nothing when the connection is already connected', async () => {
    renderCrew(fromChat());
    await channelReady();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(0);
  });

  it('connects nothing for an arrival that names no connection', async () => {
    saved = 'disconnected';
    refuseWhileDisconnected();
    renderCrew({ pathname: '/crew', search: '?sessionId=chat-1', state: chatAccessRouteState() });
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(connects()).toBe(0);
  });
});
