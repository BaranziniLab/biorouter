import { useState, useEffect } from 'react';
import { Switch } from '../../ui/switch';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { AlertCircle } from '../../icons/app-icons';

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
        setUrlError('URL must use http or https protocol');
        return false;
      }
      setUrlError(null);
      return true;
    } catch {
      setUrlError('Invalid URL format');
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

  // ⚠ **Nothing mounts this today**, and it is rebuilt rather than deleted
  // because it is the only UI that can set `userActionKey` (see the field's own
  // comment above: without it an external backend cannot reach its own private
  // chats at all). It was the last `Card`-based settings block; on the shell it
  // now lands right the day it is wired in, rather than arriving as the one
  // section on the tab with a card, a title style and a row spec of its own.
  return (
    <section id="external-backend" className="biorouter-settings-section">
      <div className="biorouter-settings-section-header">
        <h2 className="text-caps text-text-muted mb-1">Biorouter Server</h2>
        <p className="text-supporting text-text-muted">
          By default Biorouter launches a server for you, use this to connect to an external
          Biorouter server
        </p>
      </div>

      <div className="biorouter-settings-list">
        <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5">
          <div className="min-w-0 flex-1">
            <h3 className="text-label text-text-default">Use external server</h3>
            <p className="mt-0.5 max-w-md text-supporting text-text-muted">
              Connect to a Biorouter server running elsewhere (requires app restart)
            </p>
          </div>
          <Switch
            checked={config.enabled}
            onCheckedChange={(checked) => saveConfig(updateField('enabled', checked))}
            disabled={isSaving}
            variant="mono"
            aria-label="Use external server"
          />
        </div>
      </div>

      {config.enabled && (
        <div className="mt-3 space-y-4 px-3">
          <div className="space-y-2">
            <label htmlFor="external-url" className="text-label text-text-default">
              Server URL
            </label>
            <Input
              id="external-url"
              type="url"
              placeholder="http://127.0.0.1:3000"
              value={config.url}
              onChange={(e) => handleUrlChange(e.target.value)}
              onBlur={handleUrlBlur}
              disabled={isSaving}
              className={urlError ? 'border-border-danger' : ''}
            />
            {urlError && (
              <p className="flex items-center gap-1 text-supporting text-text-danger">
                <AlertCircle size={16} />
                {urlError}
              </p>
            )}
          </div>

          <div className="space-y-2">
            <label htmlFor="external-secret" className="text-label text-text-default">
              Secret Key
            </label>
            <Input
              id="external-secret"
              type="password"
              placeholder="Enter the server's secret key"
              value={config.secret}
              onChange={(e) => updateField('secret', e.target.value)}
              onBlur={() => saveConfig(config)}
              disabled={isSaving}
            />
            <p className="text-supporting text-text-muted">
              The secret key configured on the biorouterd server (BIOROUTER_SERVER__SECRET_KEY)
            </p>
          </div>

          <div className="space-y-2">
            <label htmlFor="external-user-action-key" className="text-label text-text-default">
              User Action Key
            </label>
            <Input
              id="external-user-action-key"
              type="password"
              placeholder="Enter the key the server was started with"
              value={config.userActionKey}
              onChange={(e) => updateField('userActionKey', e.target.value)}
              onBlur={() => saveConfig(config)}
              disabled={isSaving}
            />
            <p className="text-supporting text-text-muted">
              Proves a request came from you rather than from the model. The server is given the
              SHA-256 of this key on stdin when it starts. Without it, private chats cannot be
              opened, branched, or reported through this backend.
            </p>
          </div>

          <Note tone="warning">
            <strong>Note:</strong> Changes require restarting Biorouter to take effect. New chat
            windows will connect to the external server.
          </Note>
        </div>
      )}
    </section>
  );
}
