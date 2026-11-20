import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FnKeySettings, FnKeyView } from '../../../utils/dictationFnKey';
import type { FnKeyStatus } from '../../../utils/fnKeyHelper';
import { DICTATION_ACTIONS, DICTATION_MESSAGES } from '../../dictation/copy';
import { resetFnKeyStoreForTests } from '../../dictation/fnKeyStore';
import { DictationShortcutRows, FnOtherAppsRow } from './DictationShortcutRows';

const status = (over: Partial<FnKeyStatus> = {}): FnKeyStatus => ({
  supported: true,
  active: true,
  permission: 'granted',
  listening: true,
  fnUsage: 0,
  failed: false,
  ...over,
});

const view = (
  over: { settings?: Partial<FnKeySettings>; status?: Partial<FnKeyStatus> } = {}
): FnKeyView => ({
  settings: { enabled: true, otherApps: false, ...over.settings },
  status: status(over.status),
  anywhereOn: true,
});

let broadcast: ((next: FnKeyView) => void) | null = null;

function bridge(initial: FnKeyView, platform = 'darwin') {
  const set = vi.fn(async (settings: FnKeySettings) => ({ ...initial, settings }));
  const openSettings = vi.fn(async () => true);
  Object.assign(window, {
    electron: {
      platform,
      dictationFnGet: vi.fn(async () => initial),
      dictationFnSet: set,
      dictationFnOpenSettings: openSettings,
      onDictationFnState: (callback: (next: FnKeyView) => void) => {
        broadcast = callback;
        return () => undefined;
      },
    },
  });
  return { set, openSettings };
}

const fnSwitch = () => screen.findByRole('switch', { name: DICTATION_ACTIONS.dictateWithFn });

beforeEach(() => {
  resetFnKeyStoreForTests();
  broadcast = null;
});
afterEach(() => resetFnKeyStoreForTests());

describe('Dictate with the fn key (macOS)', () => {
  it('is a switch, on by default, and turning it off writes the setting through main', async () => {
    const { set } = bridge(view());
    const user = userEvent.setup();
    render(<DictationShortcutRows />);
    const control = await fnSwitch();
    expect(control).toHaveAttribute('aria-checked', 'true');
    await user.click(control);
    expect(set).toHaveBeenCalledWith({ enabled: false, otherApps: false });
    await waitFor(() => expect(control).toHaveAttribute('aria-checked', 'false'));
  });

  it('asks for Input Monitoring with one button, and nothing else, while macOS has not granted it', async () => {
    const { openSettings } = bridge(view({ status: { permission: 'denied', listening: false } }));
    const user = userEvent.setup();
    render(<DictationShortcutRows />);
    await fnSwitch();
    const note = screen.getByTestId('dictation-fn-permission');
    expect(note).toHaveTextContent(DICTATION_MESSAGES['MSG-FN-PERMISSION']);
    await user.click(screen.getByRole('button', { name: DICTATION_ACTIONS.openInputMonitoring }));
    expect(openSettings).toHaveBeenCalledWith('input-monitoring');
    expect(screen.queryByTestId('dictation-fn-conflict')).toBeNull();
  });

  it('names macOS’s own use of fn when it may clash, and says nothing when it is Do Nothing', async () => {
    const { openSettings } = bridge(view({ status: { fnUsage: 2 } }));
    const user = userEvent.setup();
    const { unmount } = render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.getByTestId('dictation-fn-conflict')).toHaveTextContent(
      'macOS also uses fn to show emoji and symbols.'
    );
    await user.click(screen.getByRole('button', { name: DICTATION_ACTIONS.openKeyboardSettings }));
    expect(openSettings).toHaveBeenCalledWith('keyboard');
    unmount();

    resetFnKeyStoreForTests();
    bridge(view({ status: { fnUsage: null } }));
    const second = render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.getByTestId('dictation-fn-conflict')).toHaveTextContent(
      DICTATION_MESSAGES['MSG-FN-CONFLICT-UNKNOWN']
    );
    second.unmount();

    resetFnKeyStoreForTests();
    bridge(view({ status: { fnUsage: 0 } }));
    render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.queryByTestId('dictation-fn-conflict')).toBeNull();
    expect(screen.queryByTestId('dictation-fn-permission')).toBeNull();
  });

  it('says so when the listener is missing from the build or keeps failing', async () => {
    bridge(view({ status: { supported: false, listening: false } }));
    const { unmount } = render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.getByRole('status')).toHaveTextContent(DICTATION_MESSAGES['MSG-FN-MISSING']);
    unmount();
    resetFnKeyStoreForTests();
    bridge(view({ status: { failed: true, listening: false } }));
    render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.getByRole('alert')).toHaveTextContent(DICTATION_MESSAGES['MSG-FN-FAILED']);
  });

  it('shows no guidance while the shortcut is off', async () => {
    bridge(
      view({
        settings: { enabled: false },
        status: { active: false, permission: 'denied', listening: false, fnUsage: 2 },
      })
    );
    render(<DictationShortcutRows />);
    expect(await fnSwitch()).toHaveAttribute('aria-checked', 'false');
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByTestId('dictation-fn-conflict')).toBeNull();
  });

  it('follows main’s broadcasts: a grant made in System Settings clears the request', async () => {
    bridge(view({ status: { permission: 'denied', listening: false } }));
    render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.getByTestId('dictation-fn-permission')).toBeInTheDocument();
    act(() => broadcast!(view()));
    await waitFor(() => expect(screen.queryByTestId('dictation-fn-permission')).toBeNull());
  });
});

describe('the keyboard shortcut off macOS', () => {
  it('names Ctrl+Shift+D and why there is no fn, with no switch', async () => {
    bridge(view({ settings: { enabled: false }, status: { supported: false } }), 'win32');
    render(<DictationShortcutRows />);
    const row = await screen.findByTestId('dictation-chord-row');
    expect(row).toHaveTextContent(DICTATION_ACTIONS.keyboardShortcut);
    expect(row).toHaveTextContent('Ctrl+Shift+D');
    expect(screen.queryByRole('switch')).toBeNull();
  });
});

describe('Use fn in other apps', () => {
  it('is off by default, shows only with the fn shortcut on, and writes its own field', async () => {
    const { set } = bridge(view());
    const user = userEvent.setup();
    render(<FnOtherAppsRow />);
    const control = await screen.findByRole('switch', {
      name: DICTATION_ACTIONS.useFnInOtherApps,
    });
    expect(control).toHaveAttribute('aria-checked', 'false');
    await user.click(control);
    expect(set).toHaveBeenCalledWith({ enabled: true, otherApps: true });
    act(() => broadcast!(view({ settings: { enabled: false } })));
    await waitFor(() => expect(screen.queryByRole('switch')).toBeNull());
  });

  it('is absent off macOS', async () => {
    bridge(view(), 'linux');
    render(<FnOtherAppsRow />);
    await act(async () => undefined);
    expect(screen.queryByRole('switch')).toBeNull();
  });
});
