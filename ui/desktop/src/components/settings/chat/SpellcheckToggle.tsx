import { useState, useEffect } from 'react';
import { Switch } from '../../ui/switch';

export const SpellcheckToggle = () => {
  const [enabled, setEnabled] = useState(true);

  useEffect(() => {
    const loadState = async () => {
      const state = await window.electron.getSpellcheckState();
      setEnabled(state);
    };
    loadState();
  }, []);

  const handleToggle = async (checked: boolean) => {
    setEnabled(checked);
    await window.electron.setSpellcheck(checked);
  };

  return (
    <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5 text-text-default">
      <div className="min-w-0 flex-1">
        <p className="text-label text-text-default">Enable spellcheck</p>
        <p className="mt-0.5 max-w-md text-supporting text-text-muted">
          Check spelling in the chat input. Requires restart to take effect.
        </p>
      </div>
      <Switch
        checked={enabled}
        onCheckedChange={handleToggle}
        variant="mono"
        aria-label="Enable spellcheck"
      />
    </div>
  );
};
