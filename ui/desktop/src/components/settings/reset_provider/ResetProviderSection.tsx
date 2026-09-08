import { Button } from '../../ui/button';
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

  const handleResetProvider = async () => {
    if (hostManaged) return;
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
    <div className="biorouter-settings-row px-3 py-3">
      <p className="text-sm text-text-default mb-1">Reset Provider and Model</p>
      <p className="text-xs text-text-muted mb-4">
        This will clear your selected model and provider settings. If no defaults are available,
        you'll be taken to the welcome screen to set them up again.
      </p>
      <Button
        onClick={handleResetProvider}
        disabled={hostManaged}
        title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
        variant="destructive"
        className="flex items-center justify-center gap-2"
      >
        <RotateCcw className="h-4 w-4" />
        Reset Provider and Model
      </Button>
      {/*
        The SHORT form. `ModelSettingsButtons` sits on this same page and
        already carries the full three-sentence reason, so repeating it here
        would say the same thing twice on one screen; the button's `title`
        holds the whole of it for anyone who reaches for the control.
      */}
      <HostManagedModelNote short className="mt-3" />
    </div>
  );
}
