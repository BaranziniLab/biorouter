/**
 * The dictation keyboard rows of **Settings > Models > Speech models**.
 *
 * - macOS: **Dictate with the fn key** (on by default): hold fn to talk, double-tap fn for
 *   hands-free. Below it, only what needs the person: Input Monitoring when macOS has not granted
 *   it, macOS's own use of fn when it may clash, and a listener that will not start.
 * - Windows and Linux: **Keyboard shortcut**, a read-only row naming Ctrl+Shift+D, because those
 *   systems do not let an app see fn.
 * - {@link FnOtherAppsRow}, under **Dictate into other apps** on macOS: the same gestures in
 *   other apps, off by default.
 *
 * The setting is main's (`<config>/dictation/fn-key.json`); this file reads and writes it only
 * through `fnKeyStore.ts`. Kept apart from `SpeechModelSection.tsx`, which other work edits.
 */
import { useSyncExternalStore } from 'react';
import { fnShortcutWorks } from '../../../utils/dictationFnKey';
import { getDictationShortcutText } from '../../../utils/keyboardShortcuts';
import { isBrowserSurface } from '../../../utils/surface';
import { DICTATION_ACTIONS, FN_USAGE_ACTIONS, dictationMessage } from '../../dictation/copy';
import { getFnKeySnapshot, setFnKeySettings, subscribeFnKey } from '../../dictation/fnKeyStore';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { SettingRow } from '../../ui/setting-row';
import { Switch } from '../../ui/switch';

function useFnKeyView() {
  return useSyncExternalStore(subscribeFnKey, getFnKeySnapshot, () => null);
}

function openPane(pane: 'input-monitoring' | 'keyboard') {
  void window.electron?.dictationFnOpenSettings?.(pane);
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
  return (
    <>
      <SettingRow
        label={DICTATION_ACTIONS.dictateWithFn}
        help={dictationMessage('MSG-FN-HELP')}
        data-testid="dictation-fn-row"
      >
        <Switch
          checked={settings.enabled}
          onCheckedChange={(enabled) => void setFnKeySettings({ ...settings, enabled })}
        />
      </SettingRow>
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
      {fnShortcutWorks(view) && usage !== 0 && (
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

/** Under **Dictate into other apps** on macOS, while the fn shortcut is on. */
export function FnOtherAppsRow() {
  const view = useFnKeyView();
  if (window.electron?.platform !== 'darwin' || !view) return null;
  const { settings, status } = view;
  if (!settings.enabled || !status.supported) return null;
  return (
    <SettingRow
      label={DICTATION_ACTIONS.useFnInOtherApps}
      help={dictationMessage('MSG-FN-OTHER-APPS')}
      data-testid="dictation-fn-other-apps-row"
    >
      <Switch
        checked={settings.otherApps}
        onCheckedChange={(otherApps) => void setFnKeySettings({ ...settings, otherApps })}
      />
    </SettingRow>
  );
}
