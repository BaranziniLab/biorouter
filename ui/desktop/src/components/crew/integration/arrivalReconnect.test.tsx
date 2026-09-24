import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connectionBarCopy } from '../channel/copy';
import { CrewHttpError } from '../crewApi';
import { emptyCopy } from '../onboarding/copy';
import { crewObservationCopy, crewStatusCopy } from '../state/copy';
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
 * Live QA round 2, Q2-01 (five critics): coming back to Crew after about five minutes away. The
 * broker closes an SSH bridge after 300 s with no request; the daemon notices only when the next
 * request fails, marks the connection disconnected, and the observation ends with the generic
 * `observation_refused` while the renderer's list still says connected. That used to show "Live
 * updates for chen-lab stopped" and take Retry, then Connect. Now: "Reconnecting…", one automatic
 * connect — never as the person, so Sign in never opens by itself — and the channel back with the
 * draft in it, under the rules in `mayReconnectAutomatically`.
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
/** How the daemon answers `POST …/connect`; by default it connects. */
let connectAnswer: () => unknown;
let watcher: ReturnType<typeof watchForStopped> | null = null;

function connects(): number {
  return mocked.crewHttp.mock.calls.filter(
    ([path, method]) => path === CONNECT && method === 'POST'
  ).length;
}

/** The SSH bridge closed while Crew was away; the observation ends as the daemon ends it. */
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
  connectAnswer = () => {
    saved = 'connected';
    return {};
  };
  daemon = installDaemon({
    messages: richMessages(),
    http: (path, method) => {
      if (path === '/connections' && method === 'GET')
        return { connections: [{ ...connection, status: saved }] };
      if (path === CONNECT && method === 'POST') return connectAnswer();
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

describe('coming back after the SSH bridge closed while Crew was away (Q2-01)', () => {
  it('reconnects by itself, once, and hands the channel back with the draft', async () => {
    renderCrew();
    const composer = await channelReady();
    fireEvent.change(composer, { target: { value: 'half-written reply' } });
    watcher = watchForStopped();

    dropTheBridge();

    // Nothing verified stays on screen; the status says what is happening, not that it stopped.
    expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();
    await waitFor(() => expect(currentCrew().status).toBe('reconnecting'));
    expect(within(statusRow()).getByText(crewStatusCopy.reconnecting)).toBeInTheDocument();
    expect(currentCrew().screen).toBe('connecting');

    // One connect, then the next verified view, with the draft still in the composer.
    expect(await channelReady()).toHaveValue('half-written reply');
    expect(connects()).toBe(1);
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().refreshError).toBeNull();
    expect(currentCrew().reconnecting).toBe(false);
    // Not as the person: Sign in never opened, and no Retry or Connect was needed.
    expect(currentCrew().signIn.open).toBe(false);
    expect(watcher.seen.stopped).toBe(false);
    expect(
      screen.queryByRole('button', { name: connectionBarCopy.retryName })
    ).not.toBeInTheDocument();
  });

  it('shows the sign-in screen when the server wants a password, and never opens Sign in itself', async () => {
    connectAnswer = () => {
      throw new CrewHttpError('Crew SSH needs a password or code', 401, 'crew_ssh_auth_required');
    };
    renderCrew();
    await channelReady();
    watcher = watchForStopped();

    dropTheBridge();

    await waitFor(() => expect(currentCrew().screen).toBe('sign-in'));
    expect(connects()).toBe(1);
    expect(currentCrew().status).toBe('sign-in-needed');
    expect(currentCrew().signIn.open).toBe(false);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button', { name: emptyCopy.signInAction })).toBeInTheDocument();
    expect(watcher.seen.stopped).toBe(false);
  });

  it('never connects by itself after the person pressed Disconnect', async () => {
    renderCrew();
    await channelReady();
    await act(async () => {
      await currentCrew().disconnect();
    });
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));

    // Connected again from a terminal (`biorouter crew connect`), then refreshed here.
    saved = 'connected';
    await act(async () => {
      await currentCrew().refresh();
    });
    await channelReady();

    dropTheBridge();

    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    expect(connects()).toBe(0);
    expect(currentCrew().status).toBe('offline');
    // The one action that helps is the person's own.
    const connect = screen.getByRole('button', { name: emptyCopy.offlineAction('Fixture') });
    fireEvent.click(connect);
    expect(await channelReady()).toBeInTheDocument();
    expect(connects()).toBe(1);
  });

  it('leaves a second drop within a minute to the person: the offline screen', async () => {
    renderCrew();
    await channelReady();
    dropTheBridge();
    await channelReady();
    expect(connects()).toBe(1);
    await waitFor(() => expect(currentCrew().status).toBe('connected'));

    dropTheBridge();

    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    expect(connects()).toBe(1);
    expect(currentCrew().status).toBe('offline');
    expect(currentCrew().reconnecting).toBe(false);
    expect(
      screen.getByRole('button', { name: emptyCopy.offlineAction('Fixture') })
    ).toBeInTheDocument();
  });

  it('keeps the old path when the daemon still calls the connection connected', async () => {
    renderCrew();
    await channelReady();

    // The observation ended, but the saved record is still connected: not a dropped bridge.
    act(() =>
      daemon.emit({
        type: 'error',
        code: 'observation_refused',
        clear: true,
        error: DAEMON_SENTENCE,
      })
    );

    const bar = screen.getByTestId('crew-connection-bar');
    expect(await within(bar).findByRole('alert')).toHaveTextContent(
      crewObservationCopy.updatesStopped('lab')
    );
    expect(within(bar).getByRole('button', { name: connectionBarCopy.retryName })).toBeEnabled();
    expect(connects()).toBe(0);
    expect(currentCrew().status).toBe('updates-unavailable');
    expect(currentCrew().reconnecting).toBe(false);
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
