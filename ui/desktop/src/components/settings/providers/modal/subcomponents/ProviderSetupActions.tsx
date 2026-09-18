import { SyntheticEvent } from 'react';
import { Button } from '../../../../ui/button';
import { Trash2, AlertTriangle } from '../../../../icons/app-icons';
import { ConfigKey } from '../../../../../api';

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
        <div className="flex items-start gap-3 w-full">
          <div className="flex-1 flex items-start gap-2 text-sm text-text-warning bg-background-warning/10 border border-border-warning/40 rounded-element px-3 py-2.5">
            <AlertTriangle className="w-4 h-4 mt-0.5 flex-shrink-0" />
            <span>
              Switch to a different model before removing <strong>{providerName}</strong>.
            </span>
          </div>
          <Button variant="ghost" size="sm" onClick={onCancelDelete}>
            OK
          </Button>
        </div>
      );
    }

    return (
      <div className="flex items-center justify-between w-full gap-3">
        <p className="flex-1 text-sm text-text-muted">
          Delete <strong className="text-text-default">{providerName}</strong> configuration? This
          cannot be undone.
        </p>
        <div className="flex items-center gap-2 flex-shrink-0">
          <Button variant="ghost" size="sm" onClick={onCancelDelete}>
            Cancel
          </Button>
          <Button size="sm" variant="destructive" onClick={onConfirmDelete}>
            <Trash2 className="w-3.5 h-3.5" />
            Delete
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex items-center w-full">
      {/* Left: destructive delete */}
      {canDelete && onDelete && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={onDelete}
          className="text-text-danger hover:bg-background-danger/10 mr-auto"
        >
          <Trash2 className="w-3.5 h-3.5" />
          Remove
        </Button>
      )}

      {/* Right: cancel + primary action */}
      <div className={`flex items-center gap-2 ${!canDelete || !onDelete ? 'ml-auto' : ''}`}>
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" size="sm" onClick={onSubmit} disabled={isSubmitting}>
          {isSubmitting
            ? 'Checking…'
            : requiredParameters && requiredParameters.length > 0
              ? 'Save'
              : 'Enable'}
        </Button>
      </div>
    </div>
  );
}
