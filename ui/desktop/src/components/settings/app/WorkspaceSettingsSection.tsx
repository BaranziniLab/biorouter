/**
 * The two tab switches of Settings > App > General, both honoured by the DAEMON
 * and stored through the same `/config/upsert` route every other preference
 * uses, because the reader is the Rust side, not the renderer.
 *
 * - "Never open tabs automatically" (BR-71 §8.1, decision 7,
 *   `WORKSPACE_ANNOUNCE_ONLY`): when it is on, `workspace_open` and subagent
 *   spawns post a notification instead of opening a tab, and the tool result
 *   tells the model that no tab was opened.
 * - "Open subagent tabs automatically" (`BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS`,
 *   default off): subagents run in the background and the chat summary lists
 *   them; turned on, each new subagent opens its own tab as it used to. A spawn
 *   that asks for a tab (`visible: true`, or a placement) gets one either way.
 *   "Never open tabs automatically" wins over both: with it on, the daemon
 *   posts a notification instead of any subagent tab (`resolve_visibility` in
 *   `agents/subagent_tool.rs`), so this switch then changes nothing, and its
 *   help says so rather than the switch being disabled.
 *
 * It used to be a whole "Workspace" section for one row. They are rows of
 * Settings > App > General now (spec §3.13), each named by its visible label
 * and mounted one after the other by `AppSettingsSection`.
 */
import { useEffect } from 'react';
import { StoredSwitch } from '../StoredSwitch';
import { recall, useLastKnown } from '../lastKnown';
import { SettingRow } from '../../ui/setting-row';
import { useConfig } from '../../ConfigContext';
import { generalCopy } from './copy';

export const ANNOUNCE_ONLY_KEY = 'WORKSPACE_ANNOUNCE_ONLY';
export const OPEN_SUBAGENT_TABS_KEY = 'BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS';

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

export function OpenSubagentTabsRow() {
  const { upsert, read } = useConfig();
  // The same contract as the row above (V4-R7-2): `undefined` until the daemon has answered, and
  // only the first time this window asks, because each visit starts from the last answer and
  // `warmSettings.ts` asks while Settings opens. Never a guess: the switch holds its place
  // unpainted until it knows.
  const [openTabs, setOpenTabs] = useLastKnown<boolean>('openSubagentTabs');

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const value = await read(OPEN_SUBAGENT_TABS_KEY, false);
      if (!cancelled) setOpenTabs(value === true);
    })().catch(() => {
      /* unreadable config → the daemon's default (background, no tab), unless a read said otherwise */
      if (!cancelled && recall('openSubagentTabs') === undefined) setOpenTabs(false);
    });
    return () => {
      cancelled = true;
    };
  }, [read, setOpenTabs]);

  const onToggle = async (next: boolean) => {
    setOpenTabs(next);
    try {
      await upsert(OPEN_SUBAGENT_TABS_KEY, next, false);
    } catch {
      setOpenTabs(!next); // roll the switch back if the write failed
    }
  };

  return (
    <SettingRow label={generalCopy.openSubagentTabs} help={generalCopy.openSubagentTabsHelp}>
      <StoredSwitch checked={openTabs ?? null} onCheckedChange={(next) => void onToggle(next)} />
    </SettingRow>
  );
}
