import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeConnection, makeCrew, renderWithCrew } from '../onboarding/testCrew';
import { signInCopy } from './copy';
import { SignInDialog } from './SignInDialog';

// The real terminal body, with xterm and the desktop bridge stood in: the dialog holds the
// terminal, its one alert, "Trouble signing in?" and the body's own Close.
const mocks = vi.hoisted(() => {
  const exitListeners: ((event: { sessionId: string; exitCode: number | null }) => void)[] = [];
  return {
    exitListeners,
    config: {} as Record<string, unknown>,
    electron: {
      createCrewAuthentication: vi.fn(),
      disposeTerminalSession: vi.fn().mockResolvedValue(undefined),
      resizeTerminalSession: vi.fn().mockResolvedValue(undefined),
      writeTerminalSession: vi.fn().mockResolvedValue(undefined),
      onTerminalData: vi.fn(() => vi.fn()),
      onTerminalExit: vi.fn(
        (listener: (event: { sessionId: string; exitCode: number | null }) => void) => {
          exitListeners.push(listener);
          return vi.fn();
        }
      ),
    },
  };
});

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 12;
    options: Record<string, unknown> = {};
    open = vi.fn();
    loadAddon = vi.fn();
    focus = vi.fn();
    dispose = vi.fn();
    write = vi.fn();
    onData = vi.fn(() => ({ dispose: vi.fn() }));
  },
}));
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = vi.fn();
  },
}));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  mocks.exitListeners.length = 0;
  vi.clearAllMocks();
  mocks.electron.createCrewAuthentication.mockResolvedValue({ success: true, sessionId: 's-1' });
  window.electron = mocks.electron as unknown as typeof window.electron;
  for (const key of Object.keys(mocks.config)) delete mocks.config[key];
  window.appConfig = {
    get: (key: string) => mocks.config[key],
  } as unknown as typeof window.appConfig;
});
afterEach(() => {
  vi.unstubAllGlobals();
});

function renderSignIn() {
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection: fakeConnection(),
    signIn: { open: true, reason: 'user' },
  });
  renderWithCrew(<SignInDialog />, crew);
  return crew;
}

describe('SignInDialog with its terminal', () => {
  it('holds the help under the terminal: same credentials, jump host and the known-hosts file', async () => {
    renderSignIn();
    const dialog = screen.getByRole('dialog', { name: signInCopy.title('hpc.ucsf.edu') });
    await waitFor(() =>
      expect(mocks.electron.resizeTerminalSession).toHaveBeenCalledWith('s-1', 80, 12)
    );

    fireEvent.click(within(dialog).getByRole('button', { name: signInCopy.help }));
    expect(within(dialog).getByText(signInCopy.sameCredentials)).toBeInTheDocument();
    expect(within(dialog).getByText(signInCopy.jumpHost)).toBeInTheDocument();
    expect(within(dialog).getByText('~/.ssh/known_hosts')).toBeInTheDocument();
    expect(
      within(dialog).getByRole('button', { name: `Copy ${signInCopy.knownHostsLabel}` })
    ).toBeInTheDocument();
  });

  it('names the isolated profile’s own known-hosts file in a development profile', async () => {
    mocks.config.BIOROUTER_DEV_PROFILE_ROOT = '/tmp/profiles/bob/';
    renderSignIn();
    fireEvent.click(screen.getByRole('button', { name: signInCopy.help }));
    expect(screen.getByText('/tmp/profiles/bob/home/.ssh/known_hosts')).toBeInTheDocument();
    await waitFor(() => expect(mocks.electron.resizeTerminalSession).toHaveBeenCalled());
  });

  it('says once, in its one alert, that sign-in ended, and closes only through Close', async () => {
    const crew = renderSignIn();
    await waitFor(() => expect(mocks.exitListeners.length).toBeGreaterThan(0));
    await waitFor(() => expect(mocks.electron.resizeTerminalSession).toHaveBeenCalled());
    await act(async () => mocks.exitListeners[0]({ sessionId: 's-1', exitCode: 255 }));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('SSH authentication ended (exit 255)');
    expect(alert).toHaveTextContent('Choose Reconnect to check the connection.');
    expect(screen.getAllByRole('alert')).toHaveLength(1);

    const close = screen.getByRole('button', { name: signInCopy.closeName });
    expect(close).toHaveTextContent(signInCopy.close);
    fireEvent.click(close);
    expect(crew.closeSignIn).toHaveBeenCalledOnce();
    expect(mocks.electron.disposeTerminalSession).toHaveBeenCalledWith('s-1');
  });
});
