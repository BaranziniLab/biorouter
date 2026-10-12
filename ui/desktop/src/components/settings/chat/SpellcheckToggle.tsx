import { useState, useEffect } from 'react';
import { StoredSwitch } from '../StoredSwitch';
import { recall, useLastKnown } from '../lastKnown';
import { SettingRow } from '../../ui/setting-row';
import { displayCopy } from './copy';

/**
 * Settings > Chat > Display > Spellcheck. Electron applies the setting when a window opens, so
 * the row says "Restart to apply" as a status line, but only while the switch differs from what
 * this window started with: a line that is always there is a paragraph, and nobody needs it
 * before they act. Turning it off and back on lands where the window already is, so the line
 * goes again (vision V4-21: it used to stay for good after the first click).
 */
export const SpellcheckRow = () => {
  // `undefined` until the main process has answered, and only the first time this window asks:
  // each visit starts from the last answer and replaces it when the new one lands (V4-R7-2). Until
  // one is known the switch holds its place unpainted (`StoredSwitch`): a placeholder `true` drew
  // the coral ON track for a person who had turned spellcheck off, on every visit.
  const [enabled, setEnabled] = useLastKnown<boolean>('spellcheck');
  // What the app was running with when this row first heard from it. `null` until it has, so
  // nothing is claimed about a setting nobody has read yet.
  const [startedWith, setStartedWith] = useState<boolean | null>(null);

  useEffect(() => {
    const loadState = async () => {
      try {
        const state = await window.electron.getSpellcheckState();
        setEnabled(state);
        setStartedWith((known) => known ?? state);
      } catch {
        // Unreadable: what the last read said, and with none the default, on.
        if (recall('spellcheck') === undefined) setEnabled(true);
      }
    };
    loadState();
  }, [setEnabled]);

  const handleToggle = async (checked: boolean) => {
    setEnabled(checked);
    await window.electron.setSpellcheck(checked);
  };

  const needsRestart = startedWith !== null && enabled !== undefined && enabled !== startedWith;

  return (
    <SettingRow
      label={displayCopy.spellcheck}
      help={displayCopy.spellcheckHelp}
      status={needsRestart ? displayCopy.restartToApply : undefined}
    >
      <StoredSwitch checked={enabled ?? null} onCheckedChange={handleToggle} />
    </SettingRow>
  );
};
