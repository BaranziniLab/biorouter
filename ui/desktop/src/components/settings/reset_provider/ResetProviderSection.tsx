import { useState } from 'react';
import { Button } from '../../ui/button';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { SettingRow } from '../../ui/setting-row';
import { useConfig } from '../../ConfigContext';
import { View, ViewOptions } from '../../../utils/navigationUtils';
import { HostManagedModelNote } from '../../privacy/HostManagedModelNote';
import { HOST_MANAGED_MODEL_REASON } from '../../privacy/hostManagedModelCopy';
import { isBrowserSurface } from '../../../utils/surface';
import { modelsDangerCopy } from '../models/copy';

interface ResetProviderSectionProps {
  setView: (view: View, viewOptions?: ViewOptions) => void;
}

/**
 * The Models tab's one Danger zone row (spec §3.13): the label, an InfoTip, and a `Reset…`
 * button (`destructive sm`). The consequence is stated at the moment of decision, in the
 * confirmation, so it is not repeated up front.
 */
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
    <>
      <SettingRow label={modelsDangerCopy.row} help={modelsDangerCopy.rowHelp}>
        <Button
          type="button"
          variant="destructive"
          size="sm"
          onClick={() => setConfirming(true)}
          disabled={hostManaged}
          title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
        >
          {modelsDangerCopy.open}
        </Button>
      </SettingRow>
      {/*
        The SHORT form. `ModelSettingsButtons` sits on this same page and
        already carries the full three-sentence reason, so repeating it here
        would say the same thing twice on one screen; the button's `title`
        holds the whole of it for anyone who reaches for the control.
      */}
      <HostManagedModelNote short className="mt-3" />
      <ConfirmationModal
        isOpen={confirming}
        title={modelsDangerCopy.confirmTitle}
        message={modelsDangerCopy.confirmMessage}
        confirmLabel={modelsDangerCopy.confirm}
        cancelLabel={modelsDangerCopy.cancel}
        confirmVariant="destructive"
        onConfirm={handleResetProvider}
        onCancel={() => setConfirming(false)}
      />
    </>
  );
}
