import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import CrewAuthentication from './CrewAuthentication';

const mocks = vi.hoisted(() => {
  const terminals: FakeTerminal[] = [];
  const dataListeners: ((event: { sessionId: string; data: string }) => void)[] = [];
  const exitListeners: ((event: { sessionId: string; exitCode: number | null }) => void)[] = [];

  class FakeTerminal {
    cols = 80;
    rows = 12;
    writes: string[] = [];
    open = vi.fn();
    loadAddon = vi.fn();
    focus = vi.fn();
    dispose = vi.fn();
    onData = vi.fn(() => ({ dispose: vi.fn() }));
    write = vi.fn((data: string) => this.writes.push(data));

    constructor() {
      terminals.push(this);
    }
  }

  return {
    terminals,
    dataListeners,
    exitListeners,
    Terminal: FakeTerminal,
    createCrewAuthentication: vi.fn(),
    disposeTerminalSession: vi.fn().mockResolvedValue(undefined),
    resizeTerminalSession: vi.fn().mockResolvedValue(undefined),
    writeTerminalSession: vi.fn().mockResolvedValue(undefined),
    onTerminalData: vi.fn((listener: (event: { sessionId: string; data: string }) => void) => {
      dataListeners.push(listener);
      return vi.fn();
    }),
    onTerminalExit: vi.fn(
      (listener: (event: { sessionId: string; exitCode: number | null }) => void) => {
        exitListeners.push(listener);
        return vi.fn();
      }
    ),
  };
});

vi.mock('@xterm/xterm', () => ({ Terminal: mocks.Terminal }));
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = vi.fn();
  },
}));
vi.mock('./CrewHostTrust', () => ({ default: () => <div /> }));

class ResizeObserverStub {
  observe = vi.fn();
  disconnect = vi.fn();
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function installElectron() {
  window.electron = {
    createCrewAuthentication: mocks.createCrewAuthentication,
    disposeTerminalSession: mocks.disposeTerminalSession,
    resizeTerminalSession: mocks.resizeTerminalSession,
    writeTerminalSession: mocks.writeTerminalSession,
    onTerminalData: mocks.onTerminalData,
    onTerminalExit: mocks.onTerminalExit,
  } as unknown as typeof window.electron;
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  mocks.terminals.length = 0;
  mocks.dataListeners.length = 0;
  mocks.exitListeners.length = 0;
  vi.clearAllMocks();
  installElectron();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('CrewAuthentication', () => {
  it('retains terminal data and an early exit diagnostic that arrive before IPC resolves', async () => {
    const creation = deferred<{ success: true; sessionId: string }>();
    mocks.createCrewAuthentication.mockReturnValue(creation.promise);

    render(
      <CrewAuthentication connectionId="connection-1" onConnected={vi.fn()} onClose={vi.fn()} />
    );
    await waitFor(() =>
      expect(mocks.createCrewAuthentication).toHaveBeenCalledWith('connection-1')
    );

    mocks.dataListeners[0]?.({ sessionId: 'session-1', data: 'Password: ' });
    mocks.exitListeners[0]?.({ sessionId: 'session-1', exitCode: 255 });
    creation.resolve({ success: true, sessionId: 'session-1' });

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'SSH authentication ended (exit 255)'
    );
    expect(mocks.terminals[0]?.writes).toEqual(['Password: ']);
    expect(mocks.resizeTerminalSession).not.toHaveBeenCalled();
  });

  it('ignores early events for an unrelated session and disposes a session closed while creation is pending', async () => {
    const creation = deferred<{ success: true; sessionId: string }>();
    let view: ReturnType<typeof render> | undefined;
    const onClose = vi.fn(() => view?.unmount());
    mocks.createCrewAuthentication.mockReturnValue(creation.promise);

    view = render(
      <CrewAuthentication connectionId="connection-2" onConnected={vi.fn()} onClose={onClose} />
    );
    await waitFor(() => expect(mocks.createCrewAuthentication).toHaveBeenCalled());
    mocks.dataListeners[0]?.({ sessionId: 'other-session', data: 'must not appear' });
    fireEvent.click(screen.getByRole('button', { name: 'Close authentication connection' }));
    creation.resolve({ success: true, sessionId: 'session-2' });

    await waitFor(() => expect(mocks.disposeTerminalSession).toHaveBeenCalledWith('session-2'));
    expect(mocks.terminals[0]?.writes).toEqual([]);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('keeps the owned session alive when the authentication route unmounts', async () => {
    mocks.createCrewAuthentication.mockResolvedValue({ success: true, sessionId: 'session-3' });
    const view = render(
      <CrewAuthentication connectionId="connection-3" onConnected={vi.fn()} onClose={vi.fn()} />
    );

    await waitFor(() =>
      expect(mocks.resizeTerminalSession).toHaveBeenCalledWith('session-3', 80, 12)
    );
    view.unmount();

    expect(mocks.disposeTerminalSession).not.toHaveBeenCalled();
  });

  it('filters unrelated live events and unsubscribes listeners on unmount', async () => {
    mocks.createCrewAuthentication.mockResolvedValue({ success: true, sessionId: 'session-4' });
    const view = render(
      <CrewAuthentication connectionId="connection-4" onConnected={vi.fn()} onClose={vi.fn()} />
    );

    await waitFor(() =>
      expect(mocks.resizeTerminalSession).toHaveBeenCalledWith('session-4', 80, 12)
    );
    mocks.dataListeners[0]?.({ sessionId: 'other-session', data: 'wrong data' });
    mocks.exitListeners[0]?.({ sessionId: 'other-session', exitCode: 17 });
    mocks.dataListeners[0]?.({ sessionId: 'session-4', data: 'matching data' });

    expect(mocks.terminals[0]?.writes).toEqual(['matching data']);
    expect(screen.queryByRole('alert')).toBeNull();

    const removeData = mocks.onTerminalData.mock.results[0]?.value as ReturnType<typeof vi.fn>;
    const removeExit = mocks.onTerminalExit.mock.results[0]?.value as ReturnType<typeof vi.fn>;
    view.unmount();
    mocks.dataListeners[0]?.({ sessionId: 'session-4', data: 'late data' });
    mocks.exitListeners[0]?.({ sessionId: 'session-4', exitCode: 99 });

    expect(mocks.terminals[0]?.writes).toEqual(['matching data']);
    expect(removeData).toHaveBeenCalledOnce();
    expect(removeExit).toHaveBeenCalledOnce();
  });
});
