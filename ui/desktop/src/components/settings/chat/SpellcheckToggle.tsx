import { useState, useEffect } from 'react';
import { Switch } from '../../ui/switch';
import { SettingRow } from '../../ui/setting-row';
import { displayCopy } from './copy';

/**
 * Settings > Chat > Display > Spellcheck. Electron applies the setting at the next launch, so
 * the row says "Restart to apply" as a status line, but only once the person has changed it:
 * a line that is always there is a paragraph, and nobody needs it before they act.
 */
export const SpellcheckRow = () => {
  const [enabled, setEnabled] = useState(true);
  const [changed, setChanged] = useState(false);

  useEffect(() => {
    const loadState = async () => {
      const state = await window.electron.getSpellcheckState();
      setEnabled(state);
    };
    loadState();
  }, []);

  const handleToggle = async (checked: boolean) => {
    setEnabled(checked);
    setChanged(true);
    await window.electron.setSpellcheck(checked);
  };

  return (
    <SettingRow
      label={displayCopy.spellcheck}
      help={displayCopy.spellcheckHelp}
      status={changed ? displayCopy.restartToApply : undefined}
    >
      <Switch checked={enabled} onCheckedChange={handleToggle} />
    </SettingRow>
  );
};
