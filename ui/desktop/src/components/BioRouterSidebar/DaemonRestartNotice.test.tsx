import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import DaemonRestartNotice, { daemonNoticeCopy } from './DaemonRestartNotice';

type State = 'attached' | 'lost' | 'reconnecting';

function installBridge(initial: State) {
  let emit: (state: State) => void = () => undefined;
  const bridge = {
    getDaemonConnection: vi.fn(async () => initial),
    onDaemonConnection: vi.fn((callback: (state: State) => void) => {
      emit = callback;
      return vi.fn();
    }),
    reconnectDaemon: vi.fn(async () => true),
    restartApp: vi.fn(),
  };
  Object.defineProperty(window, 'electron', { configurable: true, writable: true, value: bridge });
  return { bridge, emit: (state: State) => act(() => emit(state)) };
}

describe('DaemonRestartNotice (R-1)', () => {
  const original = window.electron;
  afterEach(() => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: original,
    });
  });

  it('says nothing while the app is attached', async () => {
    installBridge('attached');
    render(<DaemonRestartNotice />);
    await act(async () => undefined);
    expect(screen.queryByTestId('daemon-restart-notice')).toBeNull();
  });

  it('says reconnecting failed, with a working Try again and Quit and reopen', async () => {
    const { bridge } = installBridge('lost');
    render(<DaemonRestartNotice />);
    expect(await screen.findByText(daemonNoticeCopy.failed)).toBeInTheDocument();
    // The failure stays visible; its consequence is the InfoTip's description
    // (F-15), so it is still read out without a hover.
    expect(screen.getByTestId('daemon-restart-notice-help')).toHaveAccessibleDescription(
      daemonNoticeCopy.consequence
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Try again' }));
    expect(bridge.reconnectDaemon).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Quit and reopen' }));
    expect(bridge.restartApp).toHaveBeenCalledTimes(1);
  });

  it('shows nothing while the app reconnects on its own', async () => {
    const { emit } = installBridge('attached');
    render(<DaemonRestartNotice />);
    await act(async () => undefined);
    emit('reconnecting');
    expect(screen.queryByTestId('daemon-restart-notice')).toBeNull();
    emit('attached');
    expect(screen.queryByTestId('daemon-restart-notice')).toBeNull();
  });

  it('follows the main process after a failure: Try again runs, then gone once attached', async () => {
    const { emit } = installBridge('attached');
    render(<DaemonRestartNotice />);
    await act(async () => undefined);
    emit('reconnecting');
    emit('lost');
    expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled();
    emit('reconnecting');
    expect(screen.getByRole('button', { name: 'Reconnecting…' })).toBeDisabled();
    emit('attached');
    expect(screen.queryByTestId('daemon-restart-notice')).toBeNull();
    // A later automatic reconnect is silent again.
    emit('reconnecting');
    expect(screen.queryByTestId('daemon-restart-notice')).toBeNull();
  });

  it('renders nothing on a surface with no shared daemon', async () => {
    Object.defineProperty(window, 'electron', { configurable: true, writable: true, value: {} });
    render(<DaemonRestartNotice />);
    await act(async () => undefined);
    expect(screen.queryByTestId('daemon-restart-notice')).toBeNull();
  });
});
