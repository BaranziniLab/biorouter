import { Switch } from '../../ui/switch';
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
  // row rather than the last row of a nested box.
  return (
    <>
      {CONTEXTS.map((context) => {
        const stored = values[contextConfigKey(context.id)];
        const enabled = typeof stored === 'boolean' ? stored : true;
        return (
          <div
            key={context.id}
            className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5 text-text-default"
          >
            <div className="min-w-0 flex-1">
              <p className="text-label text-text-default">{context.label}</p>
              <p className="mt-0.5 max-w-md text-supporting text-text-muted">
                {context.description}
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center">
              <Switch
                checked={enabled}
                onCheckedChange={(checked) => handleToggle(context.id, checked)}
                variant="mono"
                aria-label={`Toggle ${context.label} context`}
              />
            </div>
          </div>
        );
      })}
    </>
  );
};
