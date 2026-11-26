import { useCallback, useEffect, useRef, useState } from 'react';
import {
  changeDictationHistorySettings,
  getDictationHistorySettings,
  type Allowance,
  type DictationHistorySettings,
  type DictationHistorySettingsRequest,
} from '../../../api';
import { toastError } from '../../../toasts';
import { userActionHeaders } from '../../../utils/userAction';
import { DICTATION_ACTIONS, fillMessage } from '../../dictation/copy';
import type { DictationStatus } from '../../dictation/dictationApi';
import { DICTIONARY_LABELS, dictionaryMessage } from '../../dictionary/copy';
import { failureOf } from '../../dictionary/useDictionary';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { SettingRow } from '../../ui/setting-row';
import { Switch } from '../../ui/switch';
import { SETTINGS_GROUP_IDS } from '../settingsSections';

/**
 * The dictation history's rows in **Settings > Models > Speech models** (DD-96, "Settings" in
 * the history design). One group at the end of the section, after the dictionary's rows, which
 * the deep link `dictation-history` scrolls to:
 *
 * - **Dictation history** (`keep_history`, on by default): whether new dictations are kept.
 *   Turning it off always works and deletes nothing; turning it on is a raise.
 * - **Save new dictations as public** (`public_by_default`, off by default), only while the
 *   history is on. Turning it on lowers privacy for every new entry, so it asks
 *   MSG-HISTORY-PUBLIC-CONFIRM first.
 *
 * Every value is the daemon's (`GET` and `POST /dictation/history/settings`), read when the group
 * mounts and again whenever the window takes focus, and every request carries
 * `userActionHeaders()`. A raise the caller has no standing to make (`authority.raise_settings`)
 * is refused before the click, with the daemon's reason under the label; lowering always works.
 * Absent, and nothing is read, where managed policy turns the history off (`policy.history`), and
 * absent against a daemon that has no history routes.
 */
export default function DictationHistorySettingsRows({ status }: { status: DictationStatus }) {
  const allowed = historyPolicy(status)?.allowed !== false;
  const { settings, change } = useHistorySettings(allowed);
  const [confirmPublic, setConfirmPublic] = useState(false);
  if (!allowed || !settings) return null;

  const labels = DICTIONARY_LABELS.historySettings;
  const keep = switchGate(settings, settings.keep_history, '--keep on');
  const publicByDefault = switchGate(
    settings,
    settings.public_by_default,
    '--public-by-default on'
  );
  return (
    <div
      id={SETTINGS_GROUP_IDS.dictationHistory}
      className="flex flex-col"
      data-testid="settings-dictation-history"
    >
      <SettingRow
        label={labels.keep}
        help={dictionaryMessage('MSG-HISTORY-HELP')}
        status={keep.reason ?? undefined}
        data-testid="dictation-history-keep"
      >
        <Switch
          checked={settings.keep_history}
          disabled={keep.disabled}
          onCheckedChange={(checked) => {
            if (checked && keep.disabled) return;
            void change({ keep_history: checked }, labels.keep);
          }}
        />
      </SettingRow>
      {settings.keep_history ? (
        <SettingRow
          label={labels.publicByDefault}
          help={dictionaryMessage('MSG-HISTORY-PUBLIC-HELP')}
          status={publicByDefault.reason ?? undefined}
          data-testid="dictation-history-public"
        >
          <Switch
            checked={settings.public_by_default}
            disabled={publicByDefault.disabled}
            onCheckedChange={(checked) => {
              if (!checked) {
                void change({ public_by_default: false }, labels.publicByDefault);
                return;
              }
              if (!publicByDefault.disabled) setConfirmPublic(true);
            }}
          />
        </SettingRow>
      ) : null}
      <ConfirmationModal
        isOpen={confirmPublic}
        title={labels.publicConfirmTitle}
        message={dictionaryMessage('MSG-HISTORY-PUBLIC-CONFIRM')}
        confirmLabel={labels.publicConfirm}
        cancelLabel={DICTATION_ACTIONS.cancel}
        confirmVariant="default"
        onConfirm={() => {
          setConfirmPublic(false);
          void change({ public_by_default: true }, labels.publicByDefault);
        }}
        onCancel={() => setConfirmPublic(false)}
      />
    </div>
  );
}

/**
 * `policy.history` (DD-96), the managed `dictation.history` key as the status reports it. A
 * daemon older than the history sends none, and its absence is not a refusal.
 */
function historyPolicy(status: DictationStatus): Allowance | null {
  return status.policy.history ?? null;
}

/**
 * Whether a switch that is `on` now may be switched, and why not. Turning one on is a raise
 * (`authority.raise_settings`); turning one off always works. The reason is the daemon's
 * (MSG-HISTORY-HOST-SETTINGS on a keyless `serve` daemon), its `<option>` filled for this switch,
 * and shows only while the switch is off and cannot be turned on.
 */
function switchGate(
  settings: DictationHistorySettings,
  on: boolean,
  option: string
): { disabled: boolean; reason: string | null } {
  if (on || settings.authority.raise_settings.allowed) return { disabled: false, reason: null };
  const reason = settings.authority.raise_settings.reason?.trim();
  return { disabled: true, reason: reason ? fillMessage(reason, { option }) : null };
}

/** The settings answer, checked, since a body can be missing or from another daemon. */
function settingsOf(value: unknown): DictationHistorySettings | null {
  if (!value || typeof value !== 'object') return null;
  const body = value as Partial<DictationHistorySettings>;
  if (typeof body.keep_history !== 'boolean' || typeof body.public_by_default !== 'boolean') {
    return null;
  }
  if (!body.authority || typeof body.authority.raise_settings?.allowed !== 'boolean') return null;
  return body as DictationHistorySettings;
}

/**
 * The daemon's history settings: read on mount and on window focus while `enabled`, and changed
 * with `change`. A read or a write that answers after a newer one was issued is dropped, so a
 * slow read never puts back a value the person just changed. A refused write says why under the
 * setting's own name, and the settings are read again.
 */
function useHistorySettings(enabled: boolean) {
  const [settings, setSettings] = useState<DictationHistorySettings | null>(null);
  const issued = useRef(0);
  const mounted = useRef(true);

  const read = useCallback(async () => {
    const ticket = ++issued.current;
    let result: { data?: unknown; error?: unknown; response?: Response };
    try {
      result = await getDictationHistorySettings({ headers: await userActionHeaders() });
    } catch {
      return;
    }
    if (!mounted.current || ticket !== issued.current || !result.response?.ok) return;
    const next = settingsOf(result.data);
    if (next) setSettings(next);
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!enabled) return;
    void read();
    const onFocus = () => void read();
    window.addEventListener('focus', onFocus);
    return () => {
      mounted.current = false;
      window.removeEventListener('focus', onFocus);
    };
  }, [enabled, read]);

  const change = useCallback(
    async (body: DictationHistorySettingsRequest, title: string) => {
      const ticket = ++issued.current;
      let result: { data?: unknown; error?: unknown; response?: Response };
      try {
        result = await changeDictationHistorySettings({
          body,
          headers: await userActionHeaders(),
        });
      } catch {
        result = {};
      }
      if (!mounted.current) return;
      const next = result.response?.ok ? settingsOf(result.data) : null;
      if (next) {
        if (ticket === issued.current) setSettings(next);
        return;
      }
      toastError({ title, msg: failureOf(result).message });
      await read();
    },
    [read]
  );

  return { settings, change };
}
