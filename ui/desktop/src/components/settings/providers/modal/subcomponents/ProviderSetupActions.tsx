import { SyntheticEvent } from 'react';
import { Button } from '../../../../ui/button';
import { Note } from '../../../../ui/note';
import { AlertTriangle } from '../../../../icons/app-icons';
import { ConfigKey } from '../../../../../api';
import { providerSetupCopy } from '../../copy';

interface ProviderSetupActionsProps {
  onCancel: () => void;
  onSubmit: (e: SyntheticEvent) => void;
  onDelete?: () => void;
  showDeleteConfirmation?: boolean;
  onConfirmDelete?: () => void;
  onCancelDelete?: () => void;
  canDelete?: boolean;
  providerName?: string;
  requiredParameters?: ConfigKey[];
  isActiveProvider?: boolean;
  isSubmitting?: boolean;
}

/**
 * The configure dialog's footer. Three states, one row each: Remove on the left with Cancel and
 * Save on the right; the inline "delete this configuration?" confirmation, which names its
 * consequence at the moment of decision; and the refusal to remove the provider in use.
 */
export default function ProviderSetupActions({
  onCancel,
  onSubmit,
  onDelete,
  showDeleteConfirmation,
  onConfirmDelete,
  onCancelDelete,
  canDelete,
  providerName,
  requiredParameters,
  isActiveProvider = false,
  isSubmitting = false,
}: ProviderSetupActionsProps) {
  if (showDeleteConfirmation) {
    if (isActiveProvider) {
      return (
        <div className="flex w-full items-start gap-3">
          <Note tone="warning" role="status" icon={AlertTriangle} className="flex-1">
            Switch to a different model before removing <strong>{providerName}</strong>.
          </Note>
          <Button variant="secondary" onClick={onCancelDelete}>
            {providerSetupCopy.ok}
          </Button>
        </div>
      );
    }

    return (
      <div className="flex w-full items-center justify-between gap-3">
        <p className="flex-1 text-secondary text-text-muted">
          Delete <strong className="text-text-default">{providerName}</strong> configuration? This
          cannot be undone.
        </p>
        <div className="flex flex-shrink-0 items-center gap-2">
          <Button variant="secondary" onClick={onCancelDelete}>
            {providerSetupCopy.cancel}
          </Button>
          <Button variant="destructive" onClick={onConfirmDelete}>
            {providerSetupCopy.delete}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex w-full items-center">
      {/* Left: destructive delete */}
      {canDelete && onDelete && (
        <Button
          type="button"
          variant="ghost"
          onClick={onDelete}
          className="mr-auto text-text-danger"
        >
          {providerSetupCopy.remove}
        </Button>
      )}

      {/* Right: cancel + primary action */}
      <div className={`flex items-center gap-2 ${!canDelete || !onDelete ? 'ml-auto' : ''}`}>
        <Button type="button" variant="secondary" onClick={onCancel}>
          {providerSetupCopy.cancel}
        </Button>
        <Button type="submit" onClick={onSubmit} disabled={isSubmitting}>
          {isSubmitting
            ? providerSetupCopy.checking
            : requiredParameters && requiredParameters.length > 0
              ? providerSetupCopy.save
              : providerSetupCopy.enable}
        </Button>
      </div>
    </div>
  );
}
