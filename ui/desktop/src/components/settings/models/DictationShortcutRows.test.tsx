import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FnKeySettings, FnKeyView } from '../../../utils/dictationFnKey';
import type { FnKeyStatus } from '../../../utils/fnKeyHelper';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';
import {
  DICTATION_ACTIONS,
  DICTATION_MESSAGES,
  FN_USAGE_ACTIONS,
  dictationMessage,
} from '../../dictation/copy';
import { resetFnKeyStoreForTests } from '../../dictation/fnKeyStore';
import {
  DictationShortcutRows,
  FnOtherAppsRow,
  TypeIntoFrontAppRow,
} from './DictationShortcutRows';

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

  // DD-98: macOS runs its own fn action on every press, so the note says a hold or a double-tap
  // can run it too, and how to keep fn for dictation.
  it('names macOS’s own use of fn when it may clash, and says nothing when it is Do Nothing', async () => {
    const { openSettings } = bridge(view({ status: { fnUsage: 2 } }));
    const user = userEvent.setup();
    const { unmount } = render(<DictationShortcutRows />);
    await fnSwitch();
    const conflict = screen.getByTestId('dictation-fn-conflict');
    expect(conflict).toHaveTextContent(
      dictationMessage('MSG-FN-CONFLICT', { action: FN_USAGE_ACTIONS[2] })
    );
    expect(conflict).toHaveTextContent(
      'macOS also uses fn to show emoji and symbols, so a hold or a double-tap can do that too. To keep fn for dictation, set Press 🌐 key to Do Nothing in Keyboard settings.'
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
    expect(screen.getByTestId('dictation-fn-conflict')).toHaveTextContent(
      'a double-tap can do it twice'
    );
    expect(
      screen.getByRole('button', { name: DICTATION_ACTIONS.openKeyboardSettings })
    ).toBeInTheDocument();
    second.unmount();

    // Each value macOS reports names its own action in the same sentence.
    for (const usage of [1, 3] as const) {
      resetFnKeyStoreForTests();
      bridge(view({ status: { fnUsage: usage } }));
      const shown = render(<DictationShortcutRows />);
      await fnSwitch();
      expect(screen.getByTestId('dictation-fn-conflict')).toHaveTextContent(
        dictationMessage('MSG-FN-CONFLICT', { action: FN_USAGE_ACTIONS[usage] })
      );
      shown.unmount();
    }

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

/**
 * **Type into the app you are using** (DD-97): main's setting (`dictation:insert:*`), with
 * whether this system can type into another app and whether macOS allows it. Only **Open
 * Accessibility** may make macOS ask.
 */
describe('Type into the app you are using', () => {
  type Insert = { supported: boolean; enabled: boolean; allowed: boolean };
  const insert = (over: Partial<Insert> = {}): Insert => ({
    supported: true,
    enabled: true,
    allowed: true,
    ...over,
  });

  function insertBridge(initial: Insert) {
    let current = initial;
    const get = vi.fn(async () => current);
    const set = vi.fn(async (enabled: boolean) => {
      current = { ...current, enabled };
      return current;
    });
    const openSettings = vi.fn(async () => undefined);
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        dictationInsertGet: get,
        dictationInsertSet: set,
        dictationInsertOpenSettings: openSettings,
      },
    });
    return {
      get,
      set,
      openSettings,
      answer: (next: Partial<Insert>) => {
        current = { ...current, ...next };
      },
    };
  }

  const insertSwitch = () =>
    screen.findByRole('switch', { name: DICTATION_ACTIONS.typeIntoFrontApp });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  it('is a switch with the help in its InfoTip, and no request while macOS allows it', async () => {
    const { set, openSettings } = insertBridge(insert());
    const user = userEvent.setup();
    render(<TypeIntoFrontAppRow />);
    const control = await insertSwitch();
    expect(control).toHaveAttribute('aria-checked', 'true');
    expect(
      screen.getByRole('button', { name: `About ${DICTATION_ACTIONS.typeIntoFrontApp}` })
    ).toHaveAccessibleDescription(DICTATION_MESSAGES['MSG-INSERT-HELP']);
    expect(screen.queryByTestId('dictation-insert-permission')).toBeNull();
    await user.click(control);
    expect(set).toHaveBeenCalledWith(false);
    await waitFor(() => expect(control).toHaveAttribute('aria-checked', 'false'));
    expect(openSettings).not.toHaveBeenCalled();
  });

  it('is absent where main cannot type into another app, with no bridge, and in a browser', async () => {
    const { get } = insertBridge(insert({ supported: false, enabled: false, allowed: false }));
    const first = render(<TypeIntoFrontAppRow />);
    await waitFor(() => expect(get).toHaveBeenCalled());
    await act(async () => undefined);
    expect(screen.queryByTestId('dictation-insert-row')).toBeNull();
    expect(screen.queryByRole('switch')).toBeNull();
    first.unmount();

    Object.assign(window, { electron: { platform: 'darwin' } });
    const second = render(<TypeIntoFrontAppRow />);
    await act(async () => undefined);
    expect(screen.queryByTestId('dictation-insert-row')).toBeNull();
    second.unmount();

    const browser = insertBridge(insert());
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(<TypeIntoFrontAppRow />);
    await act(async () => undefined);
    expect(screen.queryByTestId('dictation-insert-row')).toBeNull();
    expect(browser.get).not.toHaveBeenCalled();
  });

  it('asks for Accessibility with one button while it is on and macOS does not allow it', async () => {
    const { openSettings, get } = insertBridge(insert({ allowed: false }));
    const user = userEvent.setup();
    render(<TypeIntoFrontAppRow />);
    await insertSwitch();
    const note = screen.getByTestId('dictation-insert-permission');
    expect(note).toHaveTextContent(DICTATION_MESSAGES['MSG-INSERT-PERMISSION']);
    expect(note).toHaveAttribute('role', 'status');
    const reads = get.mock.calls.length;
    await user.click(
      within(note).getByRole('button', { name: DICTATION_ACTIONS.openAccessibility })
    );
    expect(openSettings).toHaveBeenCalledTimes(1);
    expect(openSettings).toHaveBeenCalledWith();
    // The setting is read again once main has opened the pane.
    await waitFor(() => expect(get.mock.calls.length).toBeGreaterThan(reads));
  });

  it('shows no request while it is off, even where macOS does not allow it', async () => {
    insertBridge(insert({ enabled: false, allowed: false }));
    render(<TypeIntoFrontAppRow />);
    expect(await insertSwitch()).toHaveAttribute('aria-checked', 'false');
    expect(screen.queryByTestId('dictation-insert-permission')).toBeNull();
  });

  it('turning it on where macOS does not allow it shows the request and never asks macOS', async () => {
    const { set, openSettings } = insertBridge(insert({ enabled: false, allowed: false }));
    const user = userEvent.setup();
    render(<TypeIntoFrontAppRow />);
    await user.click(await insertSwitch());
    expect(set).toHaveBeenCalledWith(true);
    expect(await screen.findByTestId('dictation-insert-permission')).toBeInTheDocument();
    expect(openSettings).not.toHaveBeenCalled();
  });

  it('reads the setting again when the window takes focus, so a grant clears the request', async () => {
    const { get, answer, openSettings } = insertBridge(insert({ allowed: false }));
    render(<TypeIntoFrontAppRow />);
    await insertSwitch();
    expect(screen.getByTestId('dictation-insert-permission')).toBeInTheDocument();
    expect(get).toHaveBeenCalledTimes(1);
    answer({ allowed: true });
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByTestId('dictation-insert-permission')).toBeNull());
    // Coming back to the window reads; it never asks.
    expect(openSettings).not.toHaveBeenCalled();
  });

  it('keeps the last good answer when main answers with something else', async () => {
    const { get } = insertBridge(insert());
    render(<TypeIntoFrontAppRow />);
    await insertSwitch();
    get.mockResolvedValueOnce({ supported: 'yes' } as unknown as Insert);
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
    expect(await insertSwitch()).toHaveAttribute('aria-checked', 'true');
  });
});
