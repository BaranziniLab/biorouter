import { Switch } from '../../ui/switch';
import { useConfig } from '../../ConfigContext';

// The four backend config keys gating the BRSDK App-SDK safety frameworks.
// All default to false on the server, so these toggles only opt a user in.
interface BrsdkConfig {
  brsdk_pii_guardrail?: boolean;
  brsdk_llm_guardrails?: boolean;
  brsdk_encryption?: boolean;
  brsdk_tracing?: boolean;
}

type BrsdkKey = keyof BrsdkConfig;

interface BrsdkToggleMeta {
  key: BrsdkKey;
  label: string;
  description: string;
}

const BRSDK_TOGGLES: BrsdkToggleMeta[] = [
  {
    key: 'brsdk_pii_guardrail',
    label: 'PII / PHI guardrail',
    description:
      'Mask personal and health information before an app message reaches the model or chat.',
  },
  {
    key: 'brsdk_llm_guardrails',
    label: 'Goal stop-hook guardrail',
    description: 'Run the goal Stop-hook judge for Agent-Drafter apps that declare a goal.',
  },
  {
    key: 'brsdk_encryption',
    label: 'Encrypted vault',
    description: "Store each Agent-Drafter app's data in a per-app encrypted vault.",
  },
  {
    key: 'brsdk_tracing',
    label: 'Agent tracing',
    description: 'Permit trace timeline support for Agent-Drafter apps that declare tracing.',
  },
];

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
  // row rather than the last row of a nested box.
  return (
    <>
      {BRSDK_TOGGLES.map((toggle) => {
        const enabled = brsdkConfig[toggle.key] ?? false;
        return (
          <div
            key={toggle.key}
            className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5 text-text-default"
          >
            <div className="min-w-0 flex-1">
              <p className="text-label text-text-default">{toggle.label}</p>
              <p className="mt-0.5 max-w-md text-supporting text-text-muted">
                {toggle.description}
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center">
              <Switch
                checked={enabled}
                onCheckedChange={(checked) => handleToggle(toggle.key, checked)}
                variant="mono"
                aria-label={`Toggle ${toggle.label}`}
              />
            </div>
          </div>
        );
      })}
    </>
  );
};
