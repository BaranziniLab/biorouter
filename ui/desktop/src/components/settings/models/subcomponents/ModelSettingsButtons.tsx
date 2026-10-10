import { useState, type ReactNode } from 'react';
import { Button } from '../../../ui/button';
import { SettingRow } from '../../../ui/setting-row';
import { SwitchModelModal } from './SwitchModelModal';
import type { View } from '../../../../utils/navigationUtils';
import { shouldShowPredefinedModels } from '../predefinedModelsUtils';
import { HostManagedModelNote } from '../../../privacy/HostManagedModelNote';
import { HOST_MANAGED_MODEL_REASON } from '../../../privacy/hostManagedModelCopy';
import { isBrowserSurface } from '../../../../utils/surface';
import { defaultModelCopy } from '../copy';

interface ConfigureModelButtonsProps {
  setView: (view: View) => void;
  /** The model and provider in force, shown as the Model row's value. */
  modelValue?: ReactNode;
}

/**
 * The "Default model" section's rows: the model in force with a `Change…` action, and, when
 * the deployment does not pin its models, a `Providers` row with `Manage…` (spec §3.13).
 *
 * The visible words are short ("Change…", "Manage…"); the accessible names stay the longer
 * ones the daemon's docs and the tests use ("Switch models", "Configure providers"). They are
 * set as `aria-label`, which `SettingRow` leaves alone, so the row label never replaces them.
 */
export default function ModelSettingsButtons({ setView, modelValue }: ConfigureModelButtonsProps) {
  const [isAddModelModalOpen, setIsAddModelModalOpen] = useState(false);
  const hasPredefinedModels = shouldShowPredefinedModels();
  // SD-1: the dialog this opens ends in `/config/set_provider`, which a
  // browser-served daemon refuses. "Manage…" beside it stays live — storing an
  // API key is not a capability write, and the picker it eventually reaches
  // carries the same explanation.
  const hostManaged = isBrowserSurface();

  return (
    <>
      <SettingRow label={defaultModelCopy.model} value={modelValue}>
        <Button
          variant="secondary"
          size="sm"
          aria-label={defaultModelCopy.changeName}
          disabled={hostManaged}
          title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
          onClick={hostManaged ? undefined : () => setIsAddModelModalOpen(true)}
        >
          {defaultModelCopy.change}
        </Button>
      </SettingRow>
      {isAddModelModalOpen ? (
        <SwitchModelModal
          sessionId={null}
          setView={setView}
          onClose={() => setIsAddModelModalOpen(false)}
        />
      ) : null}
      {!hasPredefinedModels && (
        <SettingRow label={defaultModelCopy.providers}>
          <Button
            variant="secondary"
            size="sm"
            aria-label={defaultModelCopy.manageName}
            onClick={() => {
              setView('ConfigureProviders');
            }}
          >
            {defaultModelCopy.manage}
          </Button>
        </SettingRow>
      )}
      <HostManagedModelNote className="mt-3" />
    </>
  );
}
