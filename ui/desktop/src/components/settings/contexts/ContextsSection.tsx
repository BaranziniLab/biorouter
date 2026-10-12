import { Switch } from '../../ui/switch';
import { SettingRow } from '../../ui/setting-row';
import { useConfig } from '../../ConfigContext';
import { CONTEXTS, contextConfigKey } from './contexts';

/**
 * The shipped skills, as toggles.
 *
 * ⚠ **Default ON, and absence means on.** `?? true` rather than `?? false`:
 * these load today and always have, so a user who has never opened this screen
 * must see no change. Reading a missing key as "off" would silently strip four
 * contexts from every existing install the moment this shipped.
 *
 * ⚠ **Enablement is a plain config key, deliberately NOT
 * `skills-config.json`'s `disabled[]`.** That array is honoured by
 * `handle_load_skill`, which refuses a disabled skill outright, while
 * `prompts/system.md` unconditionally instructs the model to load
 * `about-biorouter`. Routing a Context through it would turn "I don't want this
 * in my sidebar" into "the agent reports a failed skill load on every turn".
 * Off here means not surfaced, not unloadable.
 */
export const ContextsSection = () => {
  const { config, upsert } = useConfig();
  const values = (config ?? {}) as Record<string, unknown>;

  const handleToggle = async (id: string, enabled: boolean) => {
    try {
      await upsert(contextConfigKey(id), enabled, false);
    } catch (error) {
      console.error(`Error updating context ${id}:`, error);
    }
  };

  // A fragment: the rows belong directly to the `.biorouter-settings-list` this
  // section mounts into, so they abut and `:last-child` selects the real last
  // row rather than the last row of a nested box. Each switch is named by its
  // row's label and described by the row's help.
  return (
    <>
      {CONTEXTS.map((context) => {
        const stored = values[contextConfigKey(context.id)];
        const enabled = typeof stored === 'boolean' ? stored : true;
        return (
          <SettingRow
            key={context.id}
            label={context.label}
            help={context.description}
            controlId={`context-${context.id}`}
          >
            <Switch
              checked={enabled}
              onCheckedChange={(checked) => handleToggle(context.id, checked)}
            />
          </SettingRow>
        );
      })}
    </>
  );
};
