import { useState, useEffect } from 'react';
import { Switch } from '../../ui/switch';
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
  const [enabled, setEnabled] = useState(true);
  // What the app was running with when this row first heard from it. `null` until it has, so
  // nothing is claimed about a setting nobody has read yet.
  const [startedWith, setStartedWith] = useState<boolean | null>(null);

  useEffect(() => {
    const loadState = async () => {
      const state = await window.electron.getSpellcheckState();
      setEnabled(state);
      setStartedWith((known) => known ?? state);
    };
    loadState();
  }, []);

  const handleToggle = async (checked: boolean) => {
    setEnabled(checked);
    await window.electron.setSpellcheck(checked);
  };

  const needsRestart = startedWith !== null && enabled !== startedWith;

  return (
    <SettingRow
      label={displayCopy.spellcheck}
      help={displayCopy.spellcheckHelp}
      status={needsRestart ? displayCopy.restartToApply : undefined}
    >
      <Switch checked={enabled} onCheckedChange={handleToggle} />
    </SettingRow>
  );
};
