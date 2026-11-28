/**
 * The dictation keyboard rows of **Settings > Models > Speech models**.
 *
 * - macOS: **Dictate with the fn key** (on by default): hold fn to talk, double-tap fn for
 *   hands-free. Under it while it is on, **Dictation key** (Automatic, fn, or a modifier plus fn,
 *   DD-101) with a one-line status saying which key works now and why, and **Key held with fn**
 *   (Option by default, or Control or Shift; never Command). Below them, only what needs the
 *   person: Input Monitoring when macOS has not granted it, macOS's own use of fn when it may
 *   clash (only while fn alone dictates), and a listener that will not start.
 * - Windows and Linux: **Keyboard shortcut**, a read-only row naming Ctrl+Shift+D, because those
 *   systems do not let an app see fn.
 * - {@link FnOtherAppsRow}, under **Dictate into other apps** on macOS: the same gestures in
 *   other apps, off by default.
 * - {@link TypeIntoFrontAppRow}, under **Dictate into other apps** where main can type into
 *   another app (macOS, DD-97): fn in other apps and the global shortcut type the text into the
 *   app that was in front, on by default.
 *
 * The fn setting is main's (`<config>/dictation/fn-key.json`); this file reads and writes it only
 * through `fnKeyStore.ts`. The typing setting is main's too (`<config>/dictation/insert.json`),
 * read and written only through the preload's `dictationInsert*` members. Kept apart from
 * `SpeechModelSection.tsx`, which other work edits.
 */
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import {
  fnActive,
  fnKeyLabel,
  fnShortcutWorks,
  type FnActive,
  type FnKeySettings,
  type FnKeyView,
} from '../../../utils/dictationFnKey';
import type { KnownFnApp } from '../../../utils/fnCollision';
import {
  parseFnCombination,
  parseFnModifier,
  type FnCombination,
  type FnModifier,
} from '../../../utils/fnGesture';
import { getDictationShortcutText } from '../../../utils/keyboardShortcuts';
import { isBrowserSurface } from '../../../utils/surface';
import {
  DICTATION_ACTIONS,
  FN_KEY_LABELS,
  FN_MODIFIER_LABELS,
  FN_USAGE_ACTIONS,
  dictationMessage,
} from '../../dictation/copy';
import { getFnKeySnapshot, setFnKeySettings, subscribeFnKey } from '../../dictation/fnKeyStore';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { SegmentedControl } from '../../ui/segmented-control';
import { SettingRow } from '../../ui/setting-row';
import { Switch } from '../../ui/switch';

function useFnKeyView() {
  return useSyncExternalStore(subscribeFnKey, getFnKeySnapshot, () => null);
}

function openPane(pane: 'input-monitoring' | 'keyboard') {
  void window.electron?.dictationFnOpenSettings?.(pane);
}

/**
 * Which press dictates now: main's answer (`FnKeyView.active`), or, from a main that sent none,
 * the same rule applied here to the setting and the app it reported.
 */
function activeOf(view: FnKeyView): FnActive {
  return view.active ?? fnActive(view.settings, view.status.collidingApp ?? null);
}

/** The key that dictates now, as the help sentences name it: fn, or the modifier plus fn. */
function activeKeyLabel(view: FnKeyView): string {
  const active = activeOf(view);
  return fnKeyLabel(active.bare ? null : active.modifier);
}

/**
 * The setting with both DD-101 fields read as main reads them (a missing or unknown word is
 * Automatic and Option), so a write always sends the whole object.
 */
function fullSettings(settings: FnKeySettings): Required<FnKeySettings> {
  return {
    ...settings,
    combination: parseFnCombination(settings.combination),
    modifier: parseFnModifier(settings.modifier),
  };
}

/**
 * The status line under **Dictation key**: which key works now and why, in one line, or none
 * (design 5.5). `app` is a name from Biorouter's own list of apps known to use fn, never text
 * another app reports.
 */
function fnKeyStatusLine(
  combination: FnCombination,
  alt: string,
  app: KnownFnApp | null
): string | null {
  if (combination === 'automatic') {
    return app
      ? dictationMessage('MSG-FN-KEY-SWITCHED', { alt, app: app.name })
      : dictationMessage('MSG-FN-KEY-AUTO', { alt });
  }
  if (combination === 'fn' && app) return dictationMessage('MSG-FN-KEY-SHARED', { app: app.name });
  return null;
}

const MODIFIER_OPTIONS = (Object.keys(FN_MODIFIER_LABELS) as FnModifier[]).map((value) => ({
  value,
  label: FN_MODIFIER_LABELS[value],
  testId: `dictation-fn-modifier-${value}`,
}));

/**
 * **Dictation key** and **Key held with fn** (DD-101), while the fn shortcut is on and its
 * listener is in the build. Automatic uses fn and switches to the modifier plus fn while an app on
 * Biorouter's list also uses fn, and the modifier plus fn works at any time; fn and the modifier
 * choice keep one key. **Key held with fn** shows only while the modifier can apply.
 */
function FnKeyChoiceRows({ view }: { view: FnKeyView }) {
  const settings = fullSettings(view.settings);
  const { combination, modifier } = settings;
  const alt = fnKeyLabel(modifier);
  const app = activeOf(view).collidingApp ?? view.status.collidingApp ?? null;
  const line = fnKeyStatusLine(combination, alt, app);
  const write = (next: Partial<FnKeySettings>) => void setFnKeySettings({ ...settings, ...next });
  return (
    <>
      <SettingRow
        label={DICTATION_ACTIONS.dictationKey}
        help={dictationMessage('MSG-FN-KEY-HELP', { alt })}
        status={line ? <span data-testid="dictation-fn-key-status">{line}</span> : undefined}
        data-testid="dictation-fn-key-row"
      >
        <SegmentedControl<FnCombination>
          options={[
            {
              value: 'automatic',
              label: FN_KEY_LABELS.automatic,
              testId: 'dictation-fn-key-automatic',
            },
            { value: 'fn', label: FN_KEY_LABELS.fn, testId: 'dictation-fn-key-fn' },
            { value: 'modifier', label: alt, testId: 'dictation-fn-key-modifier' },
          ]}
          value={combination}
          onValueChange={(next) => write({ combination: next })}
        />
      </SettingRow>
      {combination !== 'fn' && (
        <SettingRow
          label={DICTATION_ACTIONS.keyHeldWithFn}
          help={dictationMessage('MSG-FN-MODIFIER-HELP')}
          data-testid="dictation-fn-modifier-row"
        >
          <SegmentedControl<FnModifier>
            options={MODIFIER_OPTIONS}
            value={modifier}
            onValueChange={(next) => write({ modifier: next })}
          />
        </SettingRow>
      )}
    </>
  );
}

export function DictationShortcutRows() {
  const view = useFnKeyView();
  if (isBrowserSurface()) return null;
  if (window.electron?.platform !== 'darwin') {
    const chord = getDictationShortcutText();
    return (
      <SettingRow
        label={DICTATION_ACTIONS.keyboardShortcut}
        help={dictationMessage('MSG-CHORD-HELP', { chord })}
        value={chord}
        data-testid="dictation-chord-row"
      />
    );
  }
  if (!view) return null;
  const { settings, status } = view;
  const usage = status.fnUsage;
  // macOS's own fn action is a bare fn's business, so its note shows only while fn alone dictates.
  const bare = activeOf(view).bare;
  return (
    <>
      <SettingRow
        label={DICTATION_ACTIONS.dictateWithFn}
        help={dictationMessage('MSG-FN-HELP', { key: activeKeyLabel(view) })}
        data-testid="dictation-fn-row"
      >
        <Switch
          checked={settings.enabled}
          onCheckedChange={(enabled) => void setFnKeySettings({ ...settings, enabled })}
        />
      </SettingRow>
      {settings.enabled && status.supported && <FnKeyChoiceRows view={view} />}
      {settings.enabled && !status.supported && (
        <Note tone="warning" role="status">
          {dictationMessage('MSG-FN-MISSING')}
        </Note>
      )}
      {settings.enabled && status.supported && status.failed && (
        <Note tone="warning" role="alert">
          {dictationMessage('MSG-FN-FAILED')}
        </Note>
      )}
      {settings.enabled && status.supported && !status.failed && status.permission === 'denied' && (
        <Note
          tone="warning"
          role="status"
          testId="dictation-fn-permission"
          action={
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => openPane('input-monitoring')}
            >
              {DICTATION_ACTIONS.openInputMonitoring}
            </Button>
          }
        >
          {dictationMessage('MSG-FN-PERMISSION')}
        </Note>
      )}
      {fnShortcutWorks(view) && usage !== 0 && bare && (
        <Note
          tone="neutral"
          testId="dictation-fn-conflict"
          action={
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => openPane('keyboard')}
            >
              {DICTATION_ACTIONS.openKeyboardSettings}
            </Button>
          }
        >
          {usage === null
            ? dictationMessage('MSG-FN-CONFLICT-UNKNOWN')
            : dictationMessage('MSG-FN-CONFLICT', { action: FN_USAGE_ACTIONS[usage] })}
        </Note>
      )}
    </>
  );
}

/** What main reports about typing into the app in front (`dictation:insert:get`). */
type InsertView = Awaited<ReturnType<NonNullable<Window['electron']['dictationInsertGet']>>>;

/** A view only when its three fields are the booleans main promises; anything else is none. */
function insertViewOf(value: unknown): InsertView | null {
  if (!value || typeof value !== 'object') return null;
  const { supported, enabled, allowed } = value as Record<string, unknown>;
  if (typeof supported !== 'boolean' || typeof enabled !== 'boolean') return null;
  if (typeof allowed !== 'boolean') return null;
  return { supported, enabled, allowed };
}

/**
 * **Type into the app you are using** (DD-97), under **Dictate into other apps**: fn in other
 * apps and the global shortcut paste the text into the app that was in front when the dictation
 * started, then put back what was on the clipboard. Absent where main cannot type into another
 * app (`supported` false: Windows, Linux, a browser surface, or no bridge).
 *
 * While it is on and macOS does not allow it, MSG-INSERT-PERMISSION and **Open Accessibility**
 * show. That button is the only thing here that may make macOS ask: reading the setting, turning
 * the switch and coming back to the window never do. The setting is read again whenever the
 * window takes focus, so a grant made in System Settings clears the request on return.
 */
export function TypeIntoFrontAppRow() {
  const [view, setView] = useState<InsertView | null>(null);
  const mounted = useRef(true);
  // Reads and writes answer out of order; only the newest one issued is shown.
  const issued = useRef(0);

  const adopt = useCallback((ticket: number, value: unknown) => {
    const next = insertViewOf(value);
    if (mounted.current && ticket === issued.current && next) setView(next);
  }, []);

  const read = useCallback(() => {
    const get = window.electron?.dictationInsertGet;
    if (typeof get !== 'function') return;
    const ticket = ++issued.current;
    void Promise.resolve()
      .then(() => get())
      .then((value) => adopt(ticket, value))
      .catch(() => undefined);
  }, [adopt]);

  useEffect(() => {
    mounted.current = true;
    if (isBrowserSurface()) return;
    read();
    window.addEventListener('focus', read);
    return () => {
      mounted.current = false;
      window.removeEventListener('focus', read);
    };
  }, [read]);

  if (isBrowserSurface() || !view || !view.supported) return null;

  const turn = (enabled: boolean) => {
    const set = window.electron?.dictationInsertSet;
    if (typeof set !== 'function') return;
    const ticket = ++issued.current;
    void Promise.resolve()
      .then(() => set(enabled))
      .then((value) => adopt(ticket, value))
      .catch(() => read());
  };

  const openAccessibility = () => {
    const open = window.electron?.dictationInsertOpenSettings;
    if (typeof open !== 'function') return;
    void Promise.resolve()
      .then(() => open())
      .catch(() => undefined)
      .then(() => read());
  };

  return (
    <>
      <SettingRow
        label={DICTATION_ACTIONS.typeIntoFrontApp}
        help={dictationMessage('MSG-INSERT-HELP')}
        data-testid="dictation-insert-row"
      >
        <Switch checked={view.enabled} onCheckedChange={turn} />
      </SettingRow>
      {view.enabled && !view.allowed && (
        <Note
          tone="warning"
          role="status"
          testId="dictation-insert-permission"
          action={
            <Button type="button" size="sm" variant="secondary" onClick={openAccessibility}>
              {DICTATION_ACTIONS.openAccessibility}
            </Button>
          }
        >
          {dictationMessage('MSG-INSERT-PERMISSION')}
        </Note>
      )}
    </>
  );
}

/** Under **Dictate into other apps** on macOS, while the fn shortcut is on. */
export function FnOtherAppsRow() {
  const view = useFnKeyView();
  if (window.electron?.platform !== 'darwin' || !view) return null;
  const { settings, status } = view;
  if (!settings.enabled || !status.supported) return null;
  return (
    <SettingRow
      label={DICTATION_ACTIONS.useFnInOtherApps}
      help={dictationMessage('MSG-FN-OTHER-APPS', { key: activeKeyLabel(view) })}
      data-testid="dictation-fn-other-apps-row"
    >
      <Switch
        checked={settings.otherApps}
        onCheckedChange={(otherApps) => void setFnKeySettings({ ...settings, otherApps })}
      />
    </SettingRow>
  );
}
