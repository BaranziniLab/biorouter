import { useState } from 'react';
import { Button } from '../../ui/button';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { RotateCcw } from '../../icons/app-icons';
import { useConfig } from '../../ConfigContext';
import { View, ViewOptions } from '../../../utils/navigationUtils';
import { HostManagedModelNote } from '../../privacy/HostManagedModelNote';
import { HOST_MANAGED_MODEL_REASON } from '../../privacy/hostManagedModelCopy';
import { isBrowserSurface } from '../../../utils/surface';

interface ResetProviderSectionProps {
  setView: (view: View, viewOptions?: ViewOptions) => void;
}

export default function ResetProviderSection(_props: ResetProviderSectionProps) {
  const { remove } = useConfig();
  /**
   * SD-1, and the half of it that is easy to miss: `/config/remove` is guarded
   * by the same predicate as `/config/upsert` (`is_capability_key`), because a
   * delete is not the absence of a write — it is a write of the key's default.
   * So this button 409s in a browser exactly as the model picker does, and it
   * would leave the session with no provider at all if it ever succeeded there.
   */
  const hostManaged = isBrowserSurface();
  // The reset clears both keys and reloads the window, so it asks first: a
  // destructive-styled button that acted on its first click was one stray
  // click away from sending the person back through onboarding.
  const [confirming, setConfirming] = useState(false);

  const handleResetProvider = async () => {
    if (hostManaged) return;
    setConfirming(false);
    try {
      await remove('BIOROUTER_PROVIDER', false);
      await remove('BIOROUTER_MODEL', false);

      // Refresh the page to trigger the ProviderGuard check
      window.location.reload();
    } catch (error) {
      console.error('Failed to reset provider and model:', error);
    }
  };

  return (
    <div className="biorouter-settings-row px-3 py-2.5">
      <p className="text-label text-text-default">Reset provider and model</p>
      <p className="mt-0.5 max-w-md text-supporting text-text-muted">
        This will clear your selected model and provider settings. If no defaults are available,
        you'll be taken to the welcome screen to set them up again.
      </p>
      {/* ⚠ The button carried `className="flex items-center justify-center gap-2"`
          and the `flex` was flipping the cva base's `inline-flex` through
          tailwind-merge — which is what rendered a destructive action as a
          full-width red bar across the whole reading column. The other three
          classes were exact duplicates of that base. The strip is `padding: 0`,
          so the `pt-4` is what keeps the 16px separation the `mb-4` used to
          give. */}
      <div className="biorouter-settings-control-strip pt-4">
        <Button
          onClick={() => setConfirming(true)}
          disabled={hostManaged}
          title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
          variant="destructive"
        >
          <RotateCcw className="h-4 w-4" />
          Reset provider and model
        </Button>
      </div>
      {/*
        The SHORT form. `ModelSettingsButtons` sits on this same page and
        already carries the full three-sentence reason, so repeating it here
        would say the same thing twice on one screen; the button's `title`
        holds the whole of it for anyone who reaches for the control.
      */}
      <HostManagedModelNote short className="mt-3" />
      <ConfirmationModal
        isOpen={confirming}
        title="Reset provider and model?"
        message="New chats will have no provider or model until you choose them again. If no defaults are available, Biorouter opens the welcome screen."
        confirmLabel="Reset"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        onConfirm={handleResetProvider}
        onCancel={() => setConfirming(false)}
      />
    </div>
  );
}
