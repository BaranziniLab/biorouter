import { useEffect, useId, useState } from 'react';
import {
  loadProviderSecretPreview,
  removeProviderSecret,
  SECRET_MASK,
} from '../../../../../../utils/providerSecret';
import { Button } from '../../../../../ui/button';
import { Input } from '../../../../../ui/input';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../../../ui/Tooltip';

const CHANGE_HINT = 'To change this key, remove it first, then add a new one.';

type SavedProviderSecretProps = {
  provider?: string;
  secretKey?: string;
  label: string;
  onRemoved: () => void;
  id?: string;
  'aria-describedby'?: string;
  'aria-invalid'?: boolean;
  'aria-required'?: boolean;
};

export function SavedProviderSecret({
  provider,
  secretKey,
  label,
  onRemoved,
  ...inputProps
}: SavedProviderSecretProps) {
  const hintId = useId();
  const [loaded, setLoaded] = useState<{ provider: string; key: string; value: string } | null>(
    null
  );
  const preview =
    loaded && loaded.provider === provider && loaded.key === secretKey ? loaded.value : SECRET_MASK;
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!provider || !secretKey) return;
    let current = true;
    void loadProviderSecretPreview(provider, secretKey)
      .then((value) => {
        if (current) setLoaded({ provider, key: secretKey, value });
      })
      .catch(() => {
        if (current) setError('Could not load the key preview.');
      });
    return () => {
      current = false;
    };
  }, [provider, secretKey]);

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
        <Tooltip>
          <TooltipTrigger asChild>
            <Input
              {...inputProps}
              type="text"
              readOnly
              value={preview}
              autoComplete="off"
              spellCheck={false}
              aria-describedby={[inputProps['aria-describedby'], hintId].filter(Boolean).join(' ')}
              className="min-w-0 cursor-default"
            />
          </TooltipTrigger>
          <TooltipContent side="bottom" align="start">
            {CHANGE_HINT}
          </TooltipContent>
        </Tooltip>
        {secretKey && !confirming && (
          <Button
            type="button"
            variant="secondary"
            aria-label={`Remove ${label}`}
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
            disabled={removing}
            onClick={() => void remove()}
          >
            {removing ? 'Removing…' : 'Remove key'}
          </Button>
          <Button
            type="button"
            variant="secondary"
            disabled={removing}
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
