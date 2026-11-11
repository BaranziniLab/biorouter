import { useState, useEffect } from 'react';
import { Switch } from '../../ui/switch';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import { dictationMessage } from '../../dictation/copy';
import { serverCopy } from './copy';

interface ExternalBiorouterdConfig {
  enabled: boolean;
  url: string;
  secret: string;
  /**
   * The proof-of-user key (issue #56, DR-16), matching what the external daemon
   * was launched with.
   *
   * ⚠ Without it, an external backend cannot reach its own private chats AT
   * ALL. `main.ts` reads exactly this field for `getUserActionKey`, and the
   * daemon compares `sha256` of what arrives against the digest handed to it on
   * stdin at launch. With no key here the renderer sends nothing, every private
   * chat is refused, and the refusal says "use the desktop app" to somebody who
   * IS using the desktop app.
   *
   * It was read by `main.ts` and settable nowhere. Worse, this component's own
   * shape did not carry it, and `saveConfig` writes the whole object, so a
   * hand-edited `settings.json` lost the key the next time anyone touched the
   * URL or the switch.
   */
  userActionKey: string;
}

interface Settings {
  externalBiorouterd?: Partial<ExternalBiorouterdConfig>;
}

const DEFAULT_CONFIG: ExternalBiorouterdConfig = {
  enabled: false,
  url: '',
  secret: '',
  userActionKey: '',
};

function parseConfig(
  partial: Partial<ExternalBiorouterdConfig> | undefined
): ExternalBiorouterdConfig {
  return {
    enabled: partial?.enabled ?? DEFAULT_CONFIG.enabled,
    url: partial?.url ?? DEFAULT_CONFIG.url,
    secret: partial?.secret ?? DEFAULT_CONFIG.secret,
    userActionKey: partial?.userActionKey ?? DEFAULT_CONFIG.userActionKey,
  };
}

/** The host a person typed, for the remote disclosure; the text as typed when it is no URL. */
function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/**
 * Settings > App > Server (PR-29, S17): whether this app starts its own `biorouterd` or
 * connects to one running elsewhere. It sits after About and before the Danger zone, so the
 * App tab's order up to About stays the one spec §3.13 records. One `SettingRow` per field,
 * with each explanation behind its row's InfoTip; the sentences live in `serverCopy`.
 *
 * The two keys stay plain password fields rather than `SecretInput`: these fields load the
 * stored value back, and `SecretInput`'s reveal toggle is only for a value typed in this session.
 */
export default function ExternalBackendSection() {
  const [config, setConfig] = useState<ExternalBiorouterdConfig>(DEFAULT_CONFIG);
  const [isSaving, setIsSaving] = useState(false);
  const [urlError, setUrlError] = useState<string | null>(null);

  useEffect(() => {
    const loadSettings = async () => {
      const settings = (await window.electron.getSettings()) as Settings | null;
      setConfig(parseConfig(settings?.externalBiorouterd));
    };
    loadSettings();
  }, []);

  const validateUrl = (value: string): boolean => {
    if (!value) {
      setUrlError(null);
      return true;
    }
    try {
      const parsed = new URL(value);
      if (!['http:', 'https:'].includes(parsed.protocol)) {
        setUrlError(serverCopy.urlWrongScheme);
        return false;
      }
      setUrlError(null);
      return true;
    } catch {
      setUrlError(serverCopy.urlInvalid);
      return false;
    }
  };

  const saveConfig = async (newConfig: ExternalBiorouterdConfig): Promise<void> => {
    setIsSaving(true);
    try {
      const currentSettings = ((await window.electron.getSettings()) as Settings) || {};
      await window.electron.saveSettings({
        ...currentSettings,
        externalBiorouterd: newConfig,
      });
    } catch (error) {
      console.error('Failed to save external backend settings:', error);
    } finally {
      setIsSaving(false);
    }
  };

  const updateField = <K extends keyof ExternalBiorouterdConfig>(
    field: K,
    value: ExternalBiorouterdConfig[K]
  ) => {
    const newConfig = { ...config, [field]: value };
    setConfig(newConfig);
    return newConfig;
  };

  const handleUrlChange = (value: string) => {
    updateField('url', value);
    validateUrl(value);
  };

  const handleUrlBlur = async () => {
    if (validateUrl(config.url)) {
      await saveConfig(config);
    }
  };

  return (
    <SettingSection id="external-backend" title={serverCopy.section} help={serverCopy.sectionHelp}>
      <SettingRow label={serverCopy.useExternal} help={serverCopy.useExternalHelp}>
        <Switch
          checked={config.enabled}
          onCheckedChange={(checked) => saveConfig(updateField('enabled', checked))}
          disabled={isSaving}
        />
      </SettingRow>

      {config.enabled && (
        <>
          <SettingRow
            label={serverCopy.url}
            controlId="external-url"
            status={
              urlError ? (
                <span role="alert" className="text-text-danger">
                  {urlError}
                </span>
              ) : undefined
            }
          >
            <Input
              machine
              type="url"
              placeholder={serverCopy.urlPlaceholder}
              value={config.url}
              onChange={(e) => handleUrlChange(e.target.value)}
              onBlur={handleUrlBlur}
              disabled={isSaving}
              aria-invalid={urlError ? true : undefined}
            />
          </SettingRow>

          <SettingRow
            label={serverCopy.secret}
            help={serverCopy.secretHelp}
            controlId="external-secret"
          >
            <Input
              machine
              type="password"
              autoComplete="off"
              placeholder={serverCopy.secretPlaceholder}
              value={config.secret}
              onChange={(e) => updateField('secret', e.target.value)}
              onBlur={() => saveConfig(config)}
              disabled={isSaving}
            />
          </SettingRow>

          <SettingRow
            label={serverCopy.userActionKey}
            help={serverCopy.userActionKeyHelp}
            controlId="external-user-action-key"
          >
            <Input
              machine
              type="password"
              autoComplete="off"
              placeholder={serverCopy.userActionKeyPlaceholder}
              value={config.userActionKey}
              onChange={(e) => updateField('userActionKey', e.target.value)}
              onBlur={() => saveConfig(config)}
              disabled={isSaving}
            />
          </SettingRow>

          {config.url && (
            <Note tone="neutral" testId="external-dictation-disclosure">
              {dictationMessage('MSG-DISCLOSE-REMOTE', { host: hostOf(config.url) })}
            </Note>
          )}
          <Note tone="warning">
            <strong className="font-semibold">{serverCopy.restartLead}</strong> {serverCopy.restart}
          </Note>
        </>
      )}
    </SettingSection>
  );
}
