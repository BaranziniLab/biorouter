import { useEffect, useId, useRef, useState } from 'react';
import { removeProviderSecret, SECRET_MASK } from '../../../../../../utils/providerSecret';
import { Pencil } from '../../../../../icons/app-icons';
import { Button } from '../../../../../ui/button';
import { Input } from '../../../../../ui/input';
import { IconAction } from '../../../../../ui/row-actions';
import { SecretInput } from '../../../../../ui/secret-input';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../../../ui/Tooltip';

const CHANGE_HINT = 'Edit to replace this saved key. The saved value is never shown.';

type SavedProviderSecretProps = {
  provider?: string;
  secretKey?: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  onCancelEdit?: () => void;
  onRemoved: () => void;
  id?: string;
  disabled?: boolean;
  'aria-describedby'?: string;
  'aria-invalid'?: boolean;
  'aria-required'?: boolean;
};

export function SavedProviderSecret({
  provider,
  secretKey,
  label,
  value,
  onChange,
  onCancelEdit,
  onRemoved,
  ...inputProps
}: SavedProviderSecretProps) {
  const hintId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const editRef = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState(() => value.length > 0);
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  // A different saved key starts from its own state: an open replacement, a pending removal
  // confirmation or an error belongs to the key it was made for.
  const identity = `${provider ?? ''}\u0000${secretKey ?? ''}`;
  const [shownIdentity, setShownIdentity] = useState(identity);
  if (shownIdentity !== identity) {
    setShownIdentity(identity);
    setEditing(value.length > 0);
    setConfirming(false);
    setError('');
  }

  const startEditing = () => {
    setConfirming(false);
    setError('');
    setEditing(true);
  };

  const cancelEditing = () => {
    onChange('');
    onCancelEdit?.();
    setEditing(false);
    requestAnimationFrame(() => editRef.current?.focus());
  };

  const remove = async () => {
    if (!secretKey || removing) return;
    setRemoving(true);
    setError('');
    try {
      await removeProviderSecret(secretKey);
      onRemoved();
    } catch (failure) {
      setError(
        failure instanceof Error && failure.message === 'Key is still present'
          ? 'This key is still present. If it comes from the environment, remove it there and restart the app.'
          : 'Could not remove the key. Please try again.'
      );
    } finally {
      setRemoving(false);
    }
  };

  return (
    <div className="flex min-w-0 flex-col gap-2 pb-3">
      <div className="flex min-w-0 items-center gap-2">
        {editing ? (
          <SecretInput
            ref={inputRef}
            {...inputProps}
            revealLabel={label}
            value={value}
            onChange={(event) => onChange(event.target.value)}
          />
        ) : (
          <>
            <Tooltip>
              <TooltipTrigger asChild>
                <Input
                  {...inputProps}
                  type="text"
                  readOnly
                  value={SECRET_MASK}
                  autoComplete="off"
                  spellCheck={false}
                  aria-describedby={[inputProps['aria-describedby'], hintId]
                    .filter(Boolean)
                    .join(' ')}
                  className="min-w-0 cursor-default"
                />
              </TooltipTrigger>
              <TooltipContent side="bottom" align="start">
                {CHANGE_HINT}
              </TooltipContent>
            </Tooltip>
            <IconAction
              ref={editRef}
              icon={Pencil}
              label={`Edit ${label}`}
              disabled={inputProps.disabled || removing}
              onSelect={startEditing}
            />
          </>
        )}
        {editing && (
          <Button
            type="button"
            variant="secondary"
            disabled={inputProps.disabled || removing}
            onClick={cancelEditing}
          >
            Cancel
          </Button>
        )}
        {secretKey && !confirming && (
          <Button
            type="button"
            variant="secondary"
            aria-label={`Remove ${label}`}
            disabled={inputProps.disabled || removing}
            onClick={() => setConfirming(true)}
          >
            Remove key
          </Button>
        )}
      </div>
      <span id={hintId} className="sr-only">
        {CHANGE_HINT}
      </span>
      {confirming && (
        <div className="flex flex-wrap items-center gap-2 text-supporting">
          <span>
            Remove the saved {label}? The provider may stop working until you add a new key.
          </span>
          <Button
            type="button"
            variant="destructive"
            aria-label={`Remove ${label}`}
            disabled={inputProps.disabled || removing}
            onClick={() => void remove()}
          >
            {removing ? 'Removing…' : 'Remove key'}
          </Button>
          <Button
            type="button"
            variant="secondary"
            disabled={inputProps.disabled || removing}
            onClick={() => setConfirming(false)}
          >
            Cancel
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="text-supporting text-text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
