import { Switch } from '../../ui/switch';
import { SettingRow } from '../../ui/setting-row';
import { useConfig } from '../../ConfigContext';
import { BRSDK_TOGGLES, type BrsdkKey } from './copy';

// The four backend config keys gating the BRSDK App-SDK safety frameworks.
// All default to false on the server, so these toggles only opt a user in.
type BrsdkConfig = Partial<Record<BrsdkKey, boolean>>;

export const BrsdkSection = () => {
  const { config, upsert } = useConfig();
  const brsdkConfig = (config as BrsdkConfig) ?? {};

  const handleToggle = async (key: BrsdkKey, enabled: boolean) => {
    try {
      await upsert(key, enabled, false);
    } catch (error) {
      console.error(`Error updating ${key}:`, error);
    }
  };

  // A fragment: the rows belong directly to the `.biorouter-settings-list` this
  // section mounts into, so they abut and `:last-child` selects the real last
  // row rather than the last row of a nested box. Each switch is named by its
  // row's label and described by the row's help.
  //
  // ⚠ `px-3 py-2.5` stays on the row: `BrsdkSection.test.tsx` pins the one settings-row
  // padding, and the row recipe (`.br-setting-row`) draws the same 12px / 10px.
  return (
    <>
      {BRSDK_TOGGLES.map((toggle) => {
        const enabled = brsdkConfig[toggle.key] ?? false;
        return (
          <SettingRow
            key={toggle.key}
            label={toggle.label}
            help={toggle.help}
            controlId={`brsdk-${toggle.key}`}
            className="px-3 py-2.5"
          >
            <Switch
              checked={enabled}
              onCheckedChange={(checked) => handleToggle(toggle.key, checked)}
            />
          </SettingRow>
        );
      })}
    </>
  );
};
