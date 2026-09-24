import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { useCrewErrorSlot } from '../state/CrewControllerContext';
import { ConnectionBar } from './ConnectionBar';
import { connectionBarCopy } from './copy';
import {
  alice,
  connection,
  currentCrew,
  installDaemon,
  installObserver,
  makeSnapshot,
  renderCrew,
} from './crewTestHarness';
import { seenDevicesKey } from './useNewDeviceNotice';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
}));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return {
    ...actual,
    crewHttp: mocks.crewHttp,
    crewRequest: mocks.crewRequest,
    observeCrew: mocks.observeCrew,
  };
});

function AgentSlot() {
  const mine = useCrewErrorSlot('pane:agent');
  const crew = currentCrew();
  return <div data-testid="agent-slot">{mine ? crew.error?.message : ''}</div>;
}

function Layout() {
  const [paneMounted, setPaneMounted] = useState(true);
  return (
    <>
      <ConnectionBar />
      <button onClick={() => setPaneMounted(false)}>Close agent pane</button>
      {paneMounted && <AgentSlot />}
    </>
  );
}

function bar() {
  return screen.getByTestId('crew-connection-bar');
}

async function verified() {
  await waitFor(() => expect(currentCrew().status).toBe('connected'));
}

function observationFailure(message: string, code = 'temporary_observer_error') {
  mocks.observeCrew.mockImplementation(
    async (
      _connection: string,
      _channel: string | undefined,
      _after: string | null,
      _signal: AbortSignal,
      receive: (frame: unknown) => void
    ) => {
      receive({ type: 'error', code, error: message });
      return 'terminal';
    }
  );
}

let electron: unknown;

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  electron = (window as { electron?: unknown }).electron;
  installDaemon();
  installObserver();
});

afterEach(() => {
  (window as { electron?: unknown }).electron = electron;
  vi.useRealTimers();
});

describe('ConnectionBar', () => {
  it('is empty while everything is healthy', async () => {
    renderCrew(Layout);
    await verified();
    expect(bar()).toBeEmptyDOMElement();
  });

  it('renders an observation error once, with Retry named "Retry Crew updates"', async () => {
    renderCrew(Layout);
    await verified();
    observationFailure('observer temporarily unavailable');
    await act(async () => {
      await currentCrew().refresh();
    });
    await waitFor(() =>
      expect(screen.getAllByText(/observer temporarily unavailable/)).toHaveLength(1)
    );
    expect(screen.getByRole('alert')).toHaveTextContent('observer temporarily unavailable');

    installObserver();
    const before = mocks.observeCrew.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: connectionBarCopy.retryName }));
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(before));
    await waitFor(() => expect(bar()).toBeEmptyDOMElement());
  });

  it('offers no Retry on a connection the daemon calls disconnected', async () => {
    installDaemon([{ ...connection, status: 'disconnected' }]);
    observationFailure('Crew connection is not connected');
    renderCrew(Layout);
    await screen.findByText(/Crew connection is not connected/);
    expect(screen.queryByRole('button', { name: connectionBarCopy.retryName })).toBeNull();
  });

  it('shows a global action error once, with Dismiss', async () => {
    renderCrew(Layout);
    await verified();
    act(() => currentCrew().reportError('mark read failed', 'global'));
    expect(screen.getAllByText('mark read failed')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: connectionBarCopy.dismiss }));
    expect(screen.queryByText('mark read failed')).toBeNull();
  });

  it('leaves a surface’s own error to that surface while it is mounted, then takes it', async () => {
    renderCrew(Layout);
    await verified();
    await act(async () => {
      await currentCrew().act('pane:agent', 'run.start', async () => {
        throw new Error('start failed');
      });
    });
    expect(screen.getByTestId('agent-slot')).toHaveTextContent('start failed');
    expect(bar()).not.toHaveTextContent('start failed');

    fireEvent.click(screen.getByRole('button', { name: 'Close agent pane' }));
    await waitFor(() => expect(bar()).toHaveTextContent('start failed'));
    expect(screen.getAllByText('start failed')).toHaveLength(1);
  });

  it('names an unreachable server once, in its own words, with Try again', async () => {
    renderCrew(Layout);
    await verified();
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection] };
      if (path === '/connections/conn-1/connect')
        throw new CrewHttpError('ssh: connect to host timed out', 502, 'crew_ssh_unreachable');
      return {};
    });
    await act(async () => {
      await currentCrew().connect({ userInitiated: true });
    });
    const note = await screen.findByText(connectionBarCopy.unreachable('hpc.example.edu'));
    expect(note.closest('[role="alert"]')).not.toBeNull();
    expect(screen.queryByText(/timed out/)).toBeNull();
    expect(screen.getAllByText(connectionBarCopy.unreachable('hpc.example.edu'))).toHaveLength(1);

    // Connection settings… lives in the note.
    fireEvent.click(screen.getByRole('button', { name: connectionBarCopy.connectionSettings }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'connection-settings',
      connectionId: 'conn-1',
    });

    const connects = () =>
      mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections/conn-1/connect').length;
    const before = connects();
    fireEvent.click(screen.getByRole('button', { name: connectionBarCopy.tryAgain }));
    await waitFor(() => expect(connects()).toBe(before + 1));
  });

  it('keeps naming the unreachable server as the one need after the error is dismissed elsewhere', async () => {
    renderCrew(Layout);
    await verified();
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection] };
      if (path === '/connections/conn-1/connect')
        throw new CrewHttpError('no route', 502, 'crew_ssh_unreachable');
      return {};
    });
    await act(async () => {
      await currentCrew().connect({ userInitiated: true });
    });
    act(() => currentCrew().dismissError());
    const note = await screen.findByText(connectionBarCopy.unreachable('hpc.example.edu'));
    expect(note.closest('[role="status"]')).not.toBeNull();
    expect(screen.getAllByText(connectionBarCopy.unreachable('hpc.example.edu'))).toHaveLength(1);
  });

  it('shows any other connect failure in the daemon’s words, with Try again', async () => {
    renderCrew(Layout);
    await verified();
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection] };
      if (path === '/connections/conn-1/connect')
        throw new CrewHttpError('ssh failed for a reason', 502, 'crew_ssh_failed');
      return {};
    });
    await act(async () => {
      await currentCrew().connect({ userInitiated: true });
    });
    expect(await screen.findByText('ssh failed for a reason')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: connectionBarCopy.tryAgain })).toBeEnabled();
    expect(screen.queryByRole('button', { name: connectionBarCopy.dismiss })).toBeNull();
  });

  it('asks to unlock a locked vault and refreshes once it is unlocked', async () => {
    const crewCredentials = vi.fn(async (action: string) =>
      action === 'unlock'
        ? { backend: 'encrypted_vault', initialized: true, locked: false }
        : { backend: 'encrypted_vault', initialized: true, locked: true }
    );
    (window as { electron?: unknown }).electron = { crewCredentials };
    renderCrew(Layout);
    await verified();
    const unlock = await screen.findByRole('button', { name: connectionBarCopy.unlock });
    expect(screen.getByText(connectionBarCopy.vaultLocked)).toBeInTheDocument();
    const before = mocks.observeCrew.mock.calls.length;
    fireEvent.click(unlock);
    await waitFor(() => expect(crewCredentials).toHaveBeenCalledWith('unlock'));
    await waitFor(() => expect(screen.queryByText(connectionBarCopy.vaultLocked)).toBeNull());
    await waitFor(() => expect(mocks.observeCrew.mock.calls.length).toBeGreaterThan(before));
  });

  it('never reports the keychain as locked', async () => {
    const crewCredentials = vi.fn(async () => ({
      backend: 'keyring',
      initialized: true,
      locked: true,
    }));
    (window as { electron?: unknown }).electron = { crewCredentials };
    renderCrew(Layout);
    await verified();
    await waitFor(() => expect(crewCredentials).toHaveBeenCalledWith('status'));
    expect(screen.queryByText(connectionBarCopy.vaultLocked)).toBeNull();
  });

  it('says it is reconnecting only after a connect has taken over a second', async () => {
    renderCrew(Layout);
    await verified();
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections') return { connections: [connection] };
      if (path === '/connections/conn-1/connect') return new Promise(() => undefined);
      return {};
    });
    vi.useFakeTimers();
    act(() => {
      void currentCrew().connect({ userInitiated: true });
    });
    act(() => vi.advanceTimersByTime(900));
    expect(screen.queryByText(connectionBarCopy.reconnecting('lab'))).toBeNull();
    act(() => vi.advanceTimersByTime(200));
    expect(
      screen.getByText(connectionBarCopy.reconnecting('lab')).closest('[role="status"]')
    ).not.toBeNull();
  });

  it('renders its notes in the specified order', async () => {
    const crewCredentials = vi.fn(async () => ({
      backend: 'encrypted_vault',
      initialized: true,
      locked: true,
    }));
    (window as { electron?: unknown }).electron = { crewCredentials };
    window.localStorage.setItem(seenDevicesKey('conn-1', alice.id), JSON.stringify(['AAAA']));
    installObserver({
      snapshot: makeSnapshot({
        actor: {
          ...alice,
          devices: [
            { fingerprint: 'AAAA', added_at: 1_700_000_000 },
            { fingerprint: 'BBBB', added_at: 1_700_000_500 },
          ],
        },
      }),
    });
    renderCrew(Layout);
    await verified();
    await screen.findByText(connectionBarCopy.vaultLocked);
    await screen.findByRole('button', { name: connectionBarCopy.reviewName });
    act(() => currentCrew().reportError('global failure', 'global'));
    observationFailure('observation broke');
    await act(async () => {
      await currentCrew().refresh();
    });
    act(() => currentCrew().reportError('global failure', 'global'));

    await screen.findByText(/observation broke/);
    const text = bar().textContent ?? '';
    const order = ['observation broke', 'global failure', connectionBarCopy.vaultLocked].map(
      (part) => text.indexOf(part)
    );
    expect(order.every((index) => index >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });
});

describe('the new-device notice', () => {
  const devices = (fingerprints: [string, number | undefined][]) =>
    fingerprints.map(([fingerprint, added_at]) => ({
      fingerprint,
      ...(added_at === undefined ? {} : { added_at }),
    }));
  let current = devices([['AAAA', 1_700_000_000]]);

  beforeEach(() => {
    current = devices([['AAAA', 1_700_000_000]]);
    installObserver(() => ({
      snapshot: makeSnapshot({ actor: { ...alice, devices: current } }),
    }));
  });

  it('records the first list silently, then announces a device this computer has not seen', async () => {
    renderCrew(Layout);
    await verified();
    expect(bar()).toBeEmptyDOMElement();
    expect(
      JSON.parse(window.localStorage.getItem(seenDevicesKey('conn-1', alice.id)) ?? '[]')
    ).toEqual(['AAAA']);

    // 2023-11-14T22:13:20Z, a Tuesday in another year.
    current = devices([
      ['AAAA', 1_700_000_000],
      ['BBBB', 1_700_000_000],
    ]);
    await act(async () => {
      await currentCrew().refresh();
    });
    const notice = await screen.findByText(/A new device was added to your account on/);
    expect(notice).toHaveTextContent(/November 1[45], 2023\.$/);
    expect(notice.closest('[role="status"]')).not.toBeNull();

    fireEvent.click(screen.getByRole('button', { name: connectionBarCopy.reviewName }));
    expect(currentCrew().ui.dialog).toEqual({ kind: 'keys' });
    expect(screen.queryByText(/A new device was added/)).toBeNull();
    expect(
      JSON.parse(window.localStorage.getItem(seenDevicesKey('conn-1', alice.id)) ?? '[]')
    ).toEqual(['AAAA', 'BBBB']);
  });

  it('says so without a date when the broker gave none', async () => {
    window.localStorage.setItem(seenDevicesKey('conn-1', alice.id), JSON.stringify(['AAAA']));
    current = devices([
      ['AAAA', 1_700_000_000],
      ['BBBB', undefined],
    ]);
    renderCrew(Layout);
    expect(await screen.findByText(connectionBarCopy.newDeviceUndated)).toBeInTheDocument();
  });

  it('still renders when storage refuses every access', async () => {
    const local = window.localStorage;
    const spies = [
      vi.spyOn(local, 'getItem').mockImplementation(() => {
        throw new Error('blocked');
      }),
      vi.spyOn(local, 'setItem').mockImplementation(() => {
        throw new Error('blocked');
      }),
    ];
    try {
      renderCrew(Layout);
      await verified();
      // Nothing recorded means nothing to compare against: no notice, and no crash.
      expect(bar()).toBeEmptyDOMElement();
      expect(spies[0]).toHaveBeenCalled();
    } finally {
      spies.forEach((spy) => spy.mockRestore());
    }
  });
});
