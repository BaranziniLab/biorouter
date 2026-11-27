import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  fnActive,
  fnKeyLabel,
  type FnActive,
  type FnKeySettings,
  type FnKeyView,
} from '../../../utils/dictationFnKey';
import type { KnownFnApp } from '../../../utils/fnCollision';
import type { FnKeyStatus } from '../../../utils/fnKeyHelper';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';
import {
  DICTATION_ACTIONS,
  DICTATION_MESSAGES,
  FN_KEY_LABELS,
  FN_MODIFIER_LABELS,
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

// Main always fills `combination` and `modifier` (`parseFnKeySettings`), and computes `active`
// with `fnActive` from the setting and the app it found (DD-101).
const view = (
  over: {
    settings?: Partial<FnKeySettings>;
    status?: Partial<FnKeyStatus>;
    active?: FnActive;
  } = {}
): FnKeyView => {
  const settings: FnKeySettings = {
    enabled: true,
    otherApps: false,
    combination: 'automatic',
    modifier: 'control',
    ...over.settings,
  };
  const fnStatus = status(over.status);
  return {
    settings,
    status: fnStatus,
    anywhereOn: true,
    active: over.active ?? fnActive(settings, fnStatus.collidingApp ?? null),
  };
};

const WISPR: KnownFnApp = { bundle: 'com.electron.wispr-flow', name: 'Wispr Flow' };

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
    // The whole setting goes to main, the two DD-101 fields included.
    expect(set).toHaveBeenCalledWith({
      enabled: false,
      otherApps: false,
      combination: 'automatic',
      modifier: 'control',
    });
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

/**
 * **Dictation key** and **Key held with fn** (DD-101, owner ruling O-46): fn stays the key for most
 * people, and a modifier plus fn does the same job when another app also uses fn. The status line
 * says which key works now and why; the conflict note about macOS's own fn action shows only while
 * fn alone dictates.
 */
describe('Dictation key and Key held with fn (macOS)', () => {
  const keyGroup = () => screen.findByRole('radiogroup', { name: DICTATION_ACTIONS.dictationKey });
  const modifierGroup = () =>
    screen.queryByRole('radiogroup', { name: DICTATION_ACTIONS.keyHeldWithFn });
  const statusLine = () => screen.queryByTestId('dictation-fn-key-status');

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  it('offers Automatic, fn and the chosen modifier plus fn, and says fn works with no clash', async () => {
    bridge(view());
    render(<DictationShortcutRows />);
    const group = await keyGroup();
    const radios = within(group).getAllByRole('radio');
    expect(radios.map((radio) => radio.textContent)).toEqual([
      FN_KEY_LABELS.automatic,
      FN_KEY_LABELS.fn,
      FN_KEY_LABELS.control,
    ]);
    expect(within(group).getByRole('radio', { name: FN_KEY_LABELS.automatic })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(statusLine()).toHaveTextContent(
      dictationMessage('MSG-FN-KEY-AUTO', { alt: FN_KEY_LABELS.control })
    );
    expect(statusLine()).toHaveTextContent('Using fn. Control+fn works too.');
    expect(
      screen.getByRole('button', { name: `About ${DICTATION_ACTIONS.dictationKey}` })
    ).toHaveAccessibleDescription(
      dictationMessage('MSG-FN-KEY-HELP', { alt: FN_KEY_LABELS.control })
    );
  });

  it('says it switched, and names the app, while a listed app also uses fn in Automatic', async () => {
    bridge(view({ status: { collidingApp: WISPR } }));
    render(<DictationShortcutRows />);
    await keyGroup();
    expect(statusLine()).toHaveTextContent('Using Control+fn, because Wispr Flow also uses fn.');
    expect(statusLine()).toHaveTextContent(
      dictationMessage('MSG-FN-KEY-SWITCHED', { alt: FN_KEY_LABELS.control, app: WISPR.name })
    );
  });

  it('warns that one press may start both under the fn choice while a listed app runs', async () => {
    bridge(view({ settings: { combination: 'fn' }, status: { collidingApp: WISPR } }));
    const first = render(<DictationShortcutRows />);
    const group = await keyGroup();
    expect(within(group).getByRole('radio', { name: FN_KEY_LABELS.fn })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(statusLine()).toHaveTextContent('Wispr Flow also uses fn, so one press may start both.');
    first.unmount();

    // No listed app: the fn choice says nothing.
    resetFnKeyStoreForTests();
    bridge(view({ settings: { combination: 'fn' } }));
    const second = render(<DictationShortcutRows />);
    await keyGroup();
    expect(statusLine()).toBeNull();
    second.unmount();

    // The modifier choice says nothing either, with or without a listed app.
    for (const collidingApp of [null, WISPR]) {
      resetFnKeyStoreForTests();
      bridge(view({ settings: { combination: 'modifier' }, status: { collidingApp } }));
      const shown = render(<DictationShortcutRows />);
      await keyGroup();
      expect(statusLine()).toBeNull();
      shown.unmount();
    }
  });

  it('hides Key held with fn for the fn choice and shows it for the other two', async () => {
    bridge(view({ settings: { combination: 'fn' } }));
    const first = render(<DictationShortcutRows />);
    await keyGroup();
    expect(modifierGroup()).toBeNull();
    first.unmount();

    for (const combination of ['automatic', 'modifier'] as const) {
      resetFnKeyStoreForTests();
      bridge(view({ settings: { combination } }));
      const shown = render(<DictationShortcutRows />);
      await keyGroup();
      expect(modifierGroup()).not.toBeNull();
      shown.unmount();
    }
  });

  it('offers Control, Option and Shift, never Command, and explains why in its InfoTip', async () => {
    bridge(view({ settings: { modifier: 'option' } }));
    render(<DictationShortcutRows />);
    await keyGroup();
    const group = modifierGroup()!;
    const radios = within(group).getAllByRole('radio');
    expect(radios.map((radio) => radio.textContent)).toEqual([
      FN_MODIFIER_LABELS.control,
      FN_MODIFIER_LABELS.option,
      FN_MODIFIER_LABELS.shift,
    ]);
    expect(within(group).getByRole('radio', { name: FN_MODIFIER_LABELS.option })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(screen.queryByRole('radio', { name: /command|⌘/i })).toBeNull();
    expect(
      screen.getByRole('button', { name: `About ${DICTATION_ACTIONS.keyHeldWithFn}` })
    ).toHaveAccessibleDescription(DICTATION_MESSAGES['MSG-FN-MODIFIER-HELP']);
    // The Dictation key option names the chosen modifier.
    expect(screen.getByRole('radio', { name: FN_KEY_LABELS.option })).toBeInTheDocument();
    expect(screen.queryByRole('radio', { name: FN_KEY_LABELS.control })).toBeNull();
  });

  it('writes the whole setting through main for each choice', async () => {
    const { set } = bridge(view());
    const user = userEvent.setup();
    render(<DictationShortcutRows />);
    await keyGroup();
    await user.click(screen.getByRole('radio', { name: FN_KEY_LABELS.control }));
    expect(set).toHaveBeenLastCalledWith({
      enabled: true,
      otherApps: false,
      combination: 'modifier',
      modifier: 'control',
    });
    await waitFor(() =>
      expect(screen.getByRole('radio', { name: FN_KEY_LABELS.control })).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );
    await user.click(screen.getByRole('radio', { name: FN_MODIFIER_LABELS.shift }));
    expect(set).toHaveBeenLastCalledWith({
      enabled: true,
      otherApps: false,
      combination: 'modifier',
      modifier: 'shift',
    });
    await waitFor(() =>
      expect(screen.getByRole('radio', { name: FN_KEY_LABELS.shift })).toBeInTheDocument()
    );
    await user.click(screen.getByRole('radio', { name: FN_KEY_LABELS.fn }));
    expect(set).toHaveBeenLastCalledWith({
      enabled: true,
      otherApps: false,
      combination: 'fn',
      modifier: 'shift',
    });
  });

  it('shows macOS’s own fn note only while fn alone dictates', async () => {
    // The modifier choice: fn alone is left to other apps, so macOS's fn action is not Biorouter's.
    bridge(view({ settings: { combination: 'modifier' }, status: { fnUsage: 2 } }));
    const first = render(<DictationShortcutRows />);
    await keyGroup();
    expect(screen.queryByTestId('dictation-fn-conflict')).toBeNull();
    first.unmount();

    // Automatic while a listed app runs: the same.
    resetFnKeyStoreForTests();
    bridge(view({ status: { fnUsage: 2, collidingApp: WISPR } }));
    const second = render(<DictationShortcutRows />);
    await keyGroup();
    expect(screen.queryByTestId('dictation-fn-conflict')).toBeNull();
    second.unmount();

    // Automatic with no clash: fn alone dictates, so the note shows.
    resetFnKeyStoreForTests();
    bridge(view({ status: { fnUsage: 2 } }));
    render(<DictationShortcutRows />);
    await keyGroup();
    expect(screen.getByTestId('dictation-fn-conflict')).toHaveTextContent(
      dictationMessage('MSG-FN-CONFLICT', { action: FN_USAGE_ACTIONS[2] })
    );
  });

  it('names the key that works now in the fn help and the other apps help', async () => {
    const cases: Array<[Partial<FnKeySettings>, KnownFnApp | null, string]> = [
      [{}, null, 'fn'],
      [{}, WISPR, 'Control+fn'],
      [{ combination: 'fn' }, WISPR, 'fn'],
      [{ combination: 'modifier', modifier: 'option' }, null, 'Option+fn'],
    ];
    for (const [settings, collidingApp, key] of cases) {
      resetFnKeyStoreForTests();
      bridge(view({ settings, status: { collidingApp } }));
      const shown = render(
        <>
          <DictationShortcutRows />
          <FnOtherAppsRow />
        </>
      );
      await keyGroup();
      expect(
        screen.getByRole('button', { name: `About ${DICTATION_ACTIONS.dictateWithFn}` })
      ).toHaveAccessibleDescription(dictationMessage('MSG-FN-HELP', { key }));
      expect(
        screen.getByRole('button', { name: `About ${DICTATION_ACTIONS.useFnInOtherApps}` })
      ).toHaveAccessibleDescription(dictationMessage('MSG-FN-OTHER-APPS', { key }));
      shown.unmount();
    }
  });

  it('names each key with the copy module’s words', () => {
    expect(fnKeyLabel(null)).toBe(FN_KEY_LABELS.fn);
    expect(fnKeyLabel('control')).toBe(FN_KEY_LABELS.control);
    expect(fnKeyLabel('option')).toBe(FN_KEY_LABELS.option);
    expect(fnKeyLabel('shift')).toBe(FN_KEY_LABELS.shift);
  });

  it('shows no key rows while the shortcut is off or the listener is missing', async () => {
    bridge(view({ settings: { enabled: false } }));
    const first = render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.queryByRole('radiogroup')).toBeNull();
    first.unmount();

    resetFnKeyStoreForTests();
    bridge(view({ status: { supported: false, listening: false } }));
    render(<DictationShortcutRows />);
    await fnSwitch();
    expect(screen.queryByRole('radiogroup')).toBeNull();
  });

  it('renders nothing of it off macOS or on a browser surface', async () => {
    bridge(view(), 'win32');
    const first = render(<DictationShortcutRows />);
    await screen.findByTestId('dictation-chord-row');
    expect(screen.queryByRole('radiogroup')).toBeNull();
    first.unmount();

    resetFnKeyStoreForTests();
    bridge(view());
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const second = render(<DictationShortcutRows />);
    await act(async () => undefined);
    expect(second.container).toBeEmptyDOMElement();
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
    expect(set).toHaveBeenCalledWith({
      enabled: true,
      otherApps: true,
      combination: 'automatic',
      modifier: 'control',
    });
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
