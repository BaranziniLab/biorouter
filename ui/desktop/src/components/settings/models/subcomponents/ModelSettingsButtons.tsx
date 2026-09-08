import { useState } from 'react';
import { Button } from '../../../ui/button';
import { SwitchModelModal } from './SwitchModelModal';
import type { View } from '../../../../utils/navigationUtils';
import { shouldShowPredefinedModels } from '../predefinedModelsUtils';
import { HostManagedModelNote } from '../../../privacy/HostManagedModelNote';
import { HOST_MANAGED_MODEL_REASON } from '../../../privacy/hostManagedModelCopy';
import { isBrowserSurface } from '../../../../utils/surface';

interface ConfigureModelButtonsProps {
  setView: (view: View) => void;
}

export default function ModelSettingsButtons({ setView }: ConfigureModelButtonsProps) {
  const [isAddModelModalOpen, setIsAddModelModalOpen] = useState(false);
  const hasPredefinedModels = shouldShowPredefinedModels();
  // SD-1: the dialog this opens ends in `/config/set_provider`, which a
  // browser-served daemon refuses. "Configure providers" beside it stays live —
  // storing an API key is not a capability write, and the picker it eventually
  // reaches carries the same explanation.
  const hostManaged = isBrowserSurface();

  return (
    <div className="pt-4">
      <div className="flex gap-2">
        <Button
          variant="default"
          disabled={hostManaged}
          title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
          onClick={hostManaged ? undefined : () => setIsAddModelModalOpen(true)}
        >
          Switch models
        </Button>
        {isAddModelModalOpen ? (
          <SwitchModelModal
            sessionId={null}
            setView={setView}
            onClose={() => setIsAddModelModalOpen(false)}
          />
        ) : null}
        {!hasPredefinedModels && (
          <Button
            variant="secondary"
            onClick={() => {
              setView('ConfigureProviders');
            }}
          >
            Configure providers
          </Button>
        )}
      </div>
      <HostManagedModelNote className="mt-3" />
    </div>
  );
}
