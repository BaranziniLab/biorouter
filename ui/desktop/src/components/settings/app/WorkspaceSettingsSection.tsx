/**
 * BR-71 §8.1 (decision 7). One switch, honoured by the DAEMON: when it is on,
 * `workspace_open` and subagent spawns post a notification instead of opening a
 * tab, and the tool result tells the model that no tab was opened.
 *
 * Stored under the config key `WORKSPACE_ANNOUNCE_ONLY` through the same
 * `/config/upsert` route every other preference uses, because the reader is the
 * Rust side, not the renderer.
 *
 * It used to be a whole "Workspace" section for one row. It is a row of
 * Settings > App > General now (spec §3.13), named by its visible label.
 */
import { useEffect } from 'react';
import { StoredSwitch } from '../StoredSwitch';
import { recall, useLastKnown } from '../lastKnown';
import { SettingRow } from '../../ui/setting-row';
import { useConfig } from '../../ConfigContext';
import { generalCopy } from './copy';

export const ANNOUNCE_ONLY_KEY = 'WORKSPACE_ANNOUNCE_ONLY';

export function NeverOpenTabsRow() {
  const { upsert, read } = useConfig();
  // `undefined` until the daemon has answered, and only the first time this window asks: each
  // visit starts from the last answer and replaces it when the new one lands (V4-R7-2). Until one
  // is known the switch holds its place unpainted (`StoredSwitch`): a placeholder `false` drew
  // the OFF knob for a person who had turned this on, and slid it across on every visit.
  const [announceOnly, setAnnounceOnly] = useLastKnown<boolean>('announceOnly');

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const value = await read(ANNOUNCE_ONLY_KEY, false);
      if (!cancelled) setAnnounceOnly(value === true);
    })().catch(() => {
      /* unreadable config → the default (tabs open), same as the daemon's, unless a read said otherwise */
      if (!cancelled && recall('announceOnly') === undefined) setAnnounceOnly(false);
    });
    return () => {
      cancelled = true;
    };
  }, [read, setAnnounceOnly]);

  const onToggle = async (next: boolean) => {
    setAnnounceOnly(next);
    try {
      await upsert(ANNOUNCE_ONLY_KEY, next, false);
    } catch {
      setAnnounceOnly(!next); // roll the switch back if the write failed
    }
  };

  return (
    <SettingRow label={generalCopy.announceOnly} help={generalCopy.announceOnlyHelp}>
      <StoredSwitch
        checked={announceOnly ?? null}
        onCheckedChange={(next) => void onToggle(next)}
      />
    </SettingRow>
  );
}
