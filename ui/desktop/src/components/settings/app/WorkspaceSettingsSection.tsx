/**
 * BR-71 §8.1 (decision 7). One switch, honoured by the DAEMON: when it is on,
 * `workspace_open` and subagent spawns post a notification instead of opening a
 * tab, and the tool result tells the model that no tab was opened.
 *
 * Stored under the config key `WORKSPACE_ANNOUNCE_ONLY` through the same
 * `/config/upsert` route every other preference uses, because the reader is the
 * Rust side, not the renderer.
 */
import { useEffect, useState } from 'react';
import { Switch } from '../../ui/switch';
import { useConfig } from '../../ConfigContext';

const ANNOUNCE_ONLY_KEY = 'WORKSPACE_ANNOUNCE_ONLY';

export function WorkspaceSettingsSection() {
  const { upsert, read } = useConfig();
  const [announceOnly, setAnnounceOnly] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const value = await read(ANNOUNCE_ONLY_KEY, false);
      if (!cancelled) setAnnounceOnly(value === true);
    })().catch(() => {
      /* unreadable config → the default (tabs open), same as the daemon's */
    });
    return () => {
      cancelled = true;
    };
  }, [read]);

  const onToggle = async (next: boolean) => {
    setAnnounceOnly(next);
    try {
      await upsert(ANNOUNCE_ONLY_KEY, next, false);
    } catch {
      setAnnounceOnly(!next); // roll the switch back if the write failed
    }
  };

  // The App tab is a stack of titled `biorouter-settings-section` blocks
  // (`AppSettingsSection`: Appearance / Theme / Updates …), each a header over a
  // `biorouter-settings-list`. The task's snippet gave the ROW only; mounted
  // bare it renders — the row class carries its own border, hover and
  // min-height — but as an unlabelled orphan sitting under the previous
  // section's heading, which reads as part of Updates. The shell is what makes
  // it a Workspace setting.
  // ⚠ The `<div className="pb-8">` this used to sit in is gone, and the whole
  // element had to go rather than just its class: a bare `<div>` between two
  // sections breaks `.biorouter-settings-section + .biorouter-settings-section`
  // just as effectively, and the tail spacer now lives once on the App tab's own
  // wrapper in `SettingsView`.
  return (
    <div className="biorouter-settings-section">
      <div className="biorouter-settings-section-header">
        <h2 className="text-caps text-text-muted">Workspace</h2>
      </div>
      <div className="biorouter-settings-list">
        <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5">
          <div className="min-w-0 flex-1">
            <p className="text-label text-text-default">Never open tabs automatically</p>
            <p className="mt-0.5 max-w-md text-supporting text-text-muted">
              When an agent opens a chat or starts a subagent, notify me instead of opening a tab.
              Subagents still run; open them from History.
            </p>
          </div>
          <Switch
            checked={announceOnly}
            onCheckedChange={(next) => void onToggle(next)}
            variant="mono"
            aria-label="Never open tabs automatically"
          />
        </div>
      </div>
    </div>
  );
}
