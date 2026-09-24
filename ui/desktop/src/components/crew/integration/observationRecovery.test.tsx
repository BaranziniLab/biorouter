import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connectionBarCopy } from '../channel/copy';
import { crewObservationCopy, crewStatusCopy } from '../state/copy';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  ids,
  installDaemon,
  mocked,
  renderCrew,
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
 * Live QA round 1, P0-1 and T-06/T-07/T-08/T-09/T-14/T-51: what a member sees when the daemon
 * ends their observation — which it did for every member each time anyone accepted an invitation,
 * because the broker moves the workspace policy epoch on every accept.
 */

/** The daemon's one sentence for every observer error. It must never reach the page. */
const DAEMON_SENTENCE =
  'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.';

function ended(code: string) {
  return { type: 'error', code, clear: true, error: DAEMON_SENTENCE };
}

function bar(): HTMLElement {
  return screen.getByTestId('crew-connection-bar');
}

/** Remembers whether an alert ever appeared in the connection bar, however briefly. */
function watchTheBar() {
  const seen = { alert: false, daemonWords: false };
  const check = () => {
    const current = document.querySelector('[data-testid="crew-connection-bar"]');
    if (current?.querySelector('[role="alert"]')) seen.alert = true;
    if (document.body.textContent?.includes('Room observation')) seen.daemonWords = true;
  };
  const observer = new MutationObserver(check);
  observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  check();
  return { seen, stop: () => observer.disconnect() };
}

/** Every later observation ends at once with `code`, as a daemon that keeps refusing does. */
function keepEnding(code: string) {
  mocked.observeCrew.mockImplementation(
    async (
      _connectionId: string,
      _channelId: string | undefined,
      _after: string | null,
      signal: AbortSignal,
      deliver: (frame: unknown) => void
    ) => {
      if (signal.aborted) return 'terminal';
      deliver(ended(code));
      return 'terminal';
    }
  );
}

let daemon: ScriptedDaemon;
let watcher: ReturnType<typeof watchTheBar> | null = null;

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  daemon = installDaemon({ messages: richMessages() });
});
afterEach(() => {
  watcher?.stop();
  watcher = null;
  vi.useRealTimers();
});

describe('a member’s view when someone else’s invitation is accepted (P0-1)', () => {
  it('comes back by itself with the draft kept, and no banner', async () => {
    renderCrew();
    const composer = await channelReady();
    await screen.findByText('Counts are in.');
    fireEvent.change(composer, { target: { value: 'half-written reply' } });
    watcher = watchTheBar();

    // Someone accepted an invitation: the workspace policy epoch moved, and a member joined.
    daemon.state.snapshot = richSnapshot({
      workspace: { ...richSnapshot().workspace, policy_epoch: 2 },
    });
    const before = mocked.observeCrew.mock.calls.length;
    act(() => daemon.emit(ended('policy_changed')));

    // `clear: true`: nothing verified stays on screen, and the status is a neutral "Updating…".
    expect(screen.queryByText('Counts are in.')).toBeNull();
    expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();
    expect(
      within(screen.getByRole('navigation', { name: 'Crew' })).getByText(crewStatusCopy.updating)
    ).toBeInTheDocument();
    expect(currentCrew().draft.body).toBe('half-written reply');

    // It observes again by itself, and the same channel opens with the draft in it.
    expect(await channelReady()).toHaveValue('half-written reply');
    expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before);
    expect(await screen.findByText('Counts are in.')).toBeInTheDocument();
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().error).toBeNull();
    expect(currentCrew().refreshError).toBeNull();
    expect(watcher.seen).toEqual({ alert: false, daemonWords: false });
  });

  it('clears a written draft when the workspace became public meanwhile, and says why', async () => {
    renderCrew();
    const composer = await channelReady();
    fireEvent.change(composer, { target: { value: 'sensitive words' } });

    daemon.state.snapshot = richSnapshot({
      workspace: { ...richSnapshot().workspace, mode: 'public', policy_epoch: 2 },
    });
    act(() => daemon.emit(ended('policy_changed')));

    expect(await channelReady()).toHaveValue('');
    expect(within(bar()).getByText(crewObservationCopy.scopeChanged)).toBeInTheDocument();
    expect(currentCrew().draft).toEqual({ body: '', attachments: [], references: [] });
  });

  it('says nothing about a draft when there was none to clear', async () => {
    renderCrew();
    await channelReady();
    watcher = watchTheBar();

    daemon.state.snapshot = richSnapshot({
      workspace: { ...richSnapshot().workspace, mode: 'public', policy_epoch: 2 },
    });
    act(() => daemon.emit(ended('policy_changed')));

    expect(await channelReady()).toHaveValue('');
    expect(screen.queryByText(crewObservationCopy.scopeChanged)).toBeNull();
    expect(watcher.seen).toEqual({ alert: false, daemonWords: false });
  });

  it('says plainly that updates stopped once observing again keeps failing, and Retry recovers', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderCrew();
    const composer = await channelReady();
    fireEvent.change(composer, { target: { value: 'keep me' } });

    keepEnding('policy_changed');
    act(() => daemon.emit(ended('policy_changed')));
    for (const wait of [300, 1000, 3000]) {
      expect(within(bar()).queryByRole('alert')).toBeNull();
      const before = mocked.observeCrew.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(wait);
      });
      await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before));
    }

    const note = await within(bar()).findByRole('alert');
    expect(note).toHaveTextContent(
      `${crewObservationCopy.updatesStopped('lab')} ${crewObservationCopy.draftRetained}`
    );
    expect(document.body).not.toHaveTextContent(/Room observation|cursor/);
    expect(currentCrew().status).toBe('updates-unavailable');
    expect(currentCrew().draft.body).toBe('keep me');

    // The daemon answers again; Retry brings the channel back, draft and all.
    daemon = installDaemon({ messages: richMessages() });
    fireEvent.click(within(bar()).getByRole('button', { name: connectionBarCopy.retryName }));
    expect(await channelReady()).toHaveValue('keep me');
    expect(within(bar()).queryByRole('alert')).toBeNull();
  });
});

describe('before a person is let in (T-06, T-14)', () => {
  it.each(['observation_refused', 'unauthorized'])(
    'finds a join started from the terminal by the refusal’s code (%s), and shows no observation note',
    async (code) => {
      daemon = installDaemon({
        http: (path) =>
          path === `/connections/${connection.id}/join`
            ? { status: 'invited', code: '7QK2M9XA3JTPWZ4D' }
            : undefined,
      });
      keepEnding(code);
      renderCrew();

      await waitFor(() => expect(currentCrew().screen).toBe('join'));
      expect(
        mocked.crewHttp.mock.calls.some(([path]) => path === `/connections/${connection.id}/join`)
      ).toBe(true);
      expect(currentCrew().status).toBe('not-joined');
      expect(within(bar()).queryByRole('alert')).toBeNull();
      expect(within(bar()).queryByRole('button', { name: connectionBarCopy.retryName })).toBeNull();
      expect(document.body).not.toHaveTextContent(/Room observation/);
      // The error stays in the controller, where the probe read its code.
      expect(currentCrew().refreshErrorCode).toBe(code);
    }
  );
});

describe('a team the verified view does not have yet (T-08)', () => {
  it('appears on the next state frame after it is selected, without leaving Crew', async () => {
    daemon = installDaemon({
      snapshot: richSnapshot({ teams: [], channels: [], read_positions: {}, unread: {} }),
    });
    renderCrew();
    await waitFor(() => expect(currentCrew().screen).toBe('no-team'));
    expect(currentCrew().channelId).toBe('');

    // Create team answered; the dialog selects the new team before any refresh.
    daemon.state.snapshot = richSnapshot();
    const before = mocked.observeCrew.mock.calls.length;
    act(() => currentCrew().selectTeam(ids.team));

    await channelReady();
    expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before);
    expect(currentCrew().team?.name).toBe('Analysis Lab');
  });
});

describe('a connection connected from somewhere else (T-09, T-51)', () => {
  it('is picked up when the window comes back, and observed without Retry', async () => {
    daemon = installDaemon({ connections: [{ ...connection, status: 'disconnected' }] });
    keepEnding('observation_refused');
    renderCrew();
    await waitFor(() => expect(currentCrew().refreshError).not.toBeNull());
    // The offline screen offers Connect; the bar does not repeat it with a Retry.
    expect(currentCrew().screen).toBe('offline');
    expect(within(bar()).queryByRole('alert')).toBeNull();

    // `biorouter crew connect` in a terminal, then back to the app.
    daemon = installDaemon({ messages: richMessages() });
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });

    await channelReady();
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().refreshError).toBeNull();
  });
});
