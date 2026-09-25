import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connectionBarCopy } from '../channel/copy';
import { CrewHttpError } from '../crewApi';
import { MEMBERSHIP_ENDED_CODE } from '../state/connectFailure';
import {
  OFFLINE_FOLLOW_INTERVAL_MS,
  OFFLINE_FOLLOW_WINDOW_MS,
  RECONNECTING_AFTER_MS,
} from '../state/useCrewConnections';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  installDaemon,
  mocked,
  renderCrew,
  renderedStatuses,
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
 * Live QA round 4, Q4-02 (P1) and Q4-07 (SECURITY-SENSITIVE). The daemon now re-dials a dropped
 * bridge, and a Connect that failed on the network, every few minutes for an hour (Q4-01,
 * D-KEEPALIVE). Crew used to stop reading the saved record 5 minutes after a loss, and never read
 * it after a failed Connect, so "Can't connect" stayed on screen long after the daemon had
 * reconnected while the chat already said Crew was live. Now, while Crew shows the connection
 * offline, it reads the record every 15 s (while the window is visible) for up to an hour, and
 * observes again the moment the daemon says connected. It only ever READS: nothing here connects
 * except the person's own click.
 */

const DAEMON_SENTENCE =
  'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.';
const CONNECT = `/connections/${connection.id}/connect`;
const DISCONNECT = `/connections/${connection.id}/disconnect`;

let daemon: ScriptedDaemon;
/** The saved record the daemon answers with, changed as the story goes. */
let saved: Record<string, unknown>;
/** Whether the daemon can reach the server when the person presses Connect. */
let networkUp: boolean;
let visibility: 'visible' | 'hidden';

function setVisibility(state: 'visible' | 'hidden') {
  visibility = state;
  act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
}

function reads(): number {
  return mocked.crewHttp.mock.calls.filter(
    ([path, method]) => path === '/connections' && (method ?? 'GET') === 'GET'
  ).length;
}
function connects(): number {
  return mocked.crewHttp.mock.calls.filter(
    ([path, method]) => path === CONNECT && method === 'POST'
  ).length;
}
function observations(): number {
  return mocked.observeCrew.mock.calls.length;
}

/** Advance the clock by `ms`, running every timer due meanwhile. */
async function wait(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** The daemon now calls the connection disconnected, and ends the observation as it does. */
function dropTheBridge(
  record: Record<string, unknown> = { ...connection, status: 'disconnected' }
) {
  saved = record;
  act(() =>
    daemon.emit({ type: 'error', code: 'observation_refused', clear: true, error: DAEMON_SENTENCE })
  );
}

/** The daemon's own re-dial got through: the record says connected again. */
function daemonRedialled() {
  saved = { ...connection, status: 'connected' };
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  visibility = 'visible';
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  });
  saved = { ...connection, status: 'connected' };
  networkUp = true;
  daemon = installDaemon({
    messages: richMessages(),
    http: (path, method) => {
      if (path === '/connections' && method === 'GET') return { connections: [saved] };
      if (path === CONNECT && method === 'POST') {
        if (!networkUp)
          throw new CrewHttpError(
            'Can’t reach the server.',
            502,
            'crew_ssh_unreachable',
            'ssh: connect to host 52.33.141.141 port 22: Connection refused'
          );
        saved = { ...connection, status: 'connected' };
        return {};
      }
      if (path === DISCONNECT && method === 'POST') {
        saved = { ...connection, status: 'disconnected' };
        return {};
      }
      return undefined;
    },
  });
  // A disconnected connection is refused, as the daemon refuses it; a connected one is answered.
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
      if (saved.status !== 'connected') {
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
  vi.useFakeTimers({ shouldAdvanceTime: true });
});
afterEach(() => {
  vi.useRealTimers();
  Reflect.deleteProperty(document, 'visibilityState');
});

describe('following the daemon’s re-dial while Crew shows a connection offline (Q4-02)', () => {
  it('after the person’s Connect failed: back by itself within 15 s of the re-dial, no click', async () => {
    saved = { ...connection, status: 'disconnected' };
    renderCrew();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    // The network is still down: the person's Connect fails, and "Can't connect" says so.
    networkUp = false;
    await act(async () => {
      await currentCrew().connect({ userInitiated: true });
    });
    expect(connects()).toBe(1);
    expect(currentCrew().status).toBe('cant-connect');
    const before = observations();

    // The network returns; the daemon's own retry gets through 40 s later.
    networkUp = true;
    await wait(40_000);
    expect(currentCrew().status).toBe('cant-connect');
    daemonRedialled();
    await wait(15_000);

    await channelReady();
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().lastConnectFailure).toBeNull();
    expect(observations()).toBeGreaterThan(before);
    // The one connect is still the person's own.
    expect(connects()).toBe(1);
    // And the reads stop once it is back.
    const after = reads();
    await wait(10 * 60_000);
    expect(reads()).toBe(after);
  });

  it('after a drop while watching: back by itself within 15 s of the re-dial, never a connect', async () => {
    renderCrew();
    await channelReady();

    dropTheBridge();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    await wait(40_000);
    expect(currentCrew().screen).toBe('offline');
    daemonRedialled();
    await wait(15_000);

    await channelReady();
    expect(currentCrew().status).toBe('connected');
    expect(connects()).toBe(0);
  });

  it('reads nothing while the window is hidden, and reads at once when it is shown again', async () => {
    renderCrew();
    await channelReady();
    dropTheBridge();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    setVisibility('hidden');
    const hidden = reads();
    await wait(10 * 60_000);
    expect(reads()).toBe(hidden);

    daemonRedialled();
    setVisibility('visible');
    await wait(0);
    expect(reads()).toBeGreaterThan(hidden);
    await channelReady();
    expect(connects()).toBe(0);
  });

  it('reads at once when the network comes back', async () => {
    renderCrew();
    await channelReady();
    dropTheBridge();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    const before = reads();

    daemonRedialled();
    act(() => {
      window.dispatchEvent(new Event('online'));
    });
    await wait(0);
    expect(reads()).toBe(before + 1);
    await channelReady();
    expect(connects()).toBe(0);
  });

  it('reads nothing after this window’s own Disconnect', async () => {
    renderCrew();
    await channelReady();
    await act(async () => {
      await currentCrew().disconnect();
    });
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    const before = reads();
    await wait(OFFLINE_FOLLOW_WINDOW_MS);
    expect(reads()).toBe(before);
    expect(connects()).toBe(0);
  });

  it('ends at a Disconnect pressed while it follows', async () => {
    renderCrew();
    await channelReady();
    dropTheBridge();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    await wait(OFFLINE_FOLLOW_INTERVAL_MS);

    await act(async () => {
      await currentCrew().disconnect();
    });
    const before = reads();
    await wait(OFFLINE_FOLLOW_WINDOW_MS);
    expect(reads()).toBe(before);
  });

  it('never follows a membership the workspace ended', async () => {
    renderCrew();
    await channelReady();
    dropTheBridge({
      ...connection,
      status: 'disconnected',
      last_error_code: MEMBERSHIP_ENDED_CODE,
    });
    await waitFor(() => expect(currentCrew().refreshError).not.toBeNull());

    const before = reads();
    await wait(OFFLINE_FOLLOW_WINDOW_MS);
    expect(reads()).toBe(before);
    expect(connects()).toBe(0);
  });

  it('stops after an hour, the daemon’s own retry window', async () => {
    renderCrew();
    await channelReady();
    dropTheBridge();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    const before = reads();
    await wait(OFFLINE_FOLLOW_WINDOW_MS);
    const followed = reads() - before;
    const expected = OFFLINE_FOLLOW_WINDOW_MS / OFFLINE_FOLLOW_INTERVAL_MS;
    expect(followed).toBeGreaterThanOrEqual(expected - 2);
    expect(followed).toBeLessThanOrEqual(expected);

    const ended = reads();
    await wait(OFFLINE_FOLLOW_WINDOW_MS);
    expect(reads()).toBe(ended);
    expect(connects()).toBe(0);
    expect(currentCrew().screen).toBe('offline');
  });
});

/**
 * Final acceptance NEW-1: after a person's failed Connect, the daemon re-dialled by itself and
 * Crew followed it to "Connected" — but the bar above the channel kept the failed Connect's note,
 * in the transport's own words, with Try again, for at least five minutes. The daemon's message
 * for every SSH failure is its transport record, so the bar never shows it, and the note goes as
 * soon as the connection verifies again, however it came back.
 */
describe('a failed Connect’s note after Crew reconnects by itself (NEW-1)', () => {
  const TRANSPORT_TEXT =
    'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: SSH connection closed; reconnect. Submitted operation outcome may be unknown; inspect history before retrying';
  const RAW = /Crew SSH failure|ssh_eof|child_before_cleanup|exit_255|Submitted operation/;
  const bar = () => screen.getAllByTestId('crew-connection-bar')[0];

  function failConnectsWith(code: string) {
    const answer = daemon.state.http;
    daemon.state.http = (path, method, body) => {
      if (path === CONNECT && method === 'POST' && !networkUp)
        throw new CrewHttpError(TRANSPORT_TEXT, 502, code);
      return answer?.(path, method, body);
    };
  }

  it('clears the note once the daemon’s own re-dial verifies, and never shows the transport’s words', async () => {
    saved = { ...connection, status: 'disconnected' };
    failConnectsWith('crew_ssh_unreachable');
    renderCrew();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    networkUp = false;
    await act(async () => {
      await currentCrew().connect({ userInitiated: true });
    });
    expect(currentCrew().error?.source).toBe('connect');
    await waitFor(() =>
      expect(bar()).toHaveTextContent(connectionBarCopy.unreachable('hpc.example.edu'))
    );
    expect(document.body.textContent).not.toMatch(RAW);

    // The network returns and the daemon's own retry gets through; nobody clicks.
    networkUp = true;
    daemonRedialled();
    await wait(15_000);
    await channelReady();
    expect(currentCrew().status).toBe('connected');

    // The failed Connect's note is gone with its classification, and stays gone.
    expect(currentCrew().error).toBeNull();
    expect(currentCrew().lastConnectFailure).toBeNull();
    expect(bar()).toBeEmptyDOMElement();
    await wait(5 * 60_000);
    expect(bar()).toBeEmptyDOMElement();
    expect(document.body.textContent).not.toMatch(RAW);
    expect(connects()).toBe(1);
  });

  it('says “Can’t connect to …” for any other SSH failure, never the transport record', async () => {
    saved = { ...connection, status: 'disconnected' };
    failConnectsWith('crew_ssh_failed');
    renderCrew();
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    networkUp = false;
    await act(async () => {
      await currentCrew().connect({ userInitiated: true });
    });
    await waitFor(() =>
      expect(bar()).toHaveTextContent(connectionBarCopy.cantConnect('hpc.example.edu'))
    );
    expect(document.body.textContent).not.toMatch(RAW);
  });
});

describe('“Reconnecting…” only for a loss that takes a while to decide (Q4-07)', () => {
  it('goes straight to Offline when the saved record answers at once', async () => {
    renderCrew();
    await channelReady();
    const from = renderedStatuses().length;

    dropTheBridge();
    await waitFor(() => expect(currentCrew().status).toBe('offline'));
    await wait(RECONNECTING_AFTER_MS * 2);

    const between = renderedStatuses().slice(from);
    expect(between).not.toContain('reconnecting');
    expect(between).not.toContain('updating');
    expect(currentCrew().reconnecting).toBe(false);
    expect(currentCrew().status).toBe('offline');
  });

  it('says “Reconnecting…” once the decision takes longer than that, and only then', async () => {
    renderCrew();
    await channelReady();
    let answer!: () => void;
    const held = new Promise<void>((resolve) => {
      answer = resolve;
    });
    const respond = daemon.state.http!;
    daemon.state.http = (path, method, body) =>
      path === '/connections' && method === 'GET'
        ? held.then(() => respond(path, method, body))
        : respond(path, method, body);

    dropTheBridge();
    await wait(RECONNECTING_AFTER_MS - 50);
    expect(currentCrew().reconnecting).toBe(false);
    expect(currentCrew().status).toBe('checking');
    await wait(100);
    expect(currentCrew().reconnecting).toBe(true);
    expect(currentCrew().status).toBe('reconnecting');

    await act(async () => {
      answer();
    });
    await waitFor(() => expect(currentCrew().status).toBe('offline'));
    expect(currentCrew().reconnecting).toBe(false);
    expect(connects()).toBe(0);
  });
});
