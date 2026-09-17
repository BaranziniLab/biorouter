import { useEffect, useState } from 'react';
import {
  getExtensionCredentials,
  purgeExtensionCredentials,
  type ExtensionCredential,
} from '../../../../api';
import { userActionHeaders } from '../../../../utils/userAction';
import { Button } from '../../../ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../../ui/dialog';

interface Props {
  name: string;
  onClose: () => void;
  onDeleted: (keys: string[]) => void;
}

export default function ExtensionCredentialsDialog({ name, onClose, onDeleted }: Props) {
  const [credentials, setCredentials] = useState<ExtensionCredential[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [deleted, setDeleted] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data } = await getExtensionCredentials({
          path: { name },
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        if (!cancelled) setCredentials(data);
      } catch {
        if (!cancelled)
          setError('Could not verify saved credentials. Close this dialog and try again.');
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [name]);

  const removable = credentials.filter(
    (credential) => credential.stored && credential.used_by.length === 0
  );
  const purge = async () => {
    setBusy(true);
    setError('');
    const keys = removable.map((credential) => credential.key);
    try {
      const { data } = await purgeExtensionCredentials({
        path: { name },
        body: { keys },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      setCredentials(data);
      setDeleted(true);
      onDeleted(keys);
    } catch {
      setError(
        'Credentials could not be deleted. References may have changed or storage is unavailable. Close this dialog and review again.'
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={() => {
        if (!busy) onClose();
      }}
    >
      <DialogContent className="sm:max-w-[600px] max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Saved credentials for {name}</DialogTitle>
          <DialogDescription>
            Delete saved credentials separately from removing the extension. Credentials referenced
            by other installed extensions or providers are protected.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 py-4 text-sm">
          <p>
            Only saved values listed below can be deleted. Environment variables, values in
            extension settings, and credentials already loaded in running chats stay unchanged.
            Restart Biorouter after deleting credentials.
          </p>
          {loading ? (
            <p role="status">Checking saved credentials…</p>
          ) : credentials.length === 0 && !error ? (
            <p>No saved credential references for this extension.</p>
          ) : (
            <ul className="space-y-3">
              {credentials.map((credential) => (
                <li key={credential.key}>
                  <span className="font-medium break-all">{credential.key}</span>
                  <p className="text-text-muted">
                    {!credential.stored
                      ? 'No saved value'
                      : credential.used_by.length > 0
                        ? `Retained — ${credential.used_by.join('; ')}`
                        : 'Will be deleted from saved credentials'}
                  </p>
                </li>
              ))}
            </ul>
          )}
          {deleted && (
            <p role="status">
              Saved credentials deleted. You will need to enter them again when required unless they
              are supplied by your environment or extension settings.
            </p>
          )}
          {error && (
            <p role="alert" className="text-text-danger">
              {error}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>
            {deleted ? 'Done' : 'Cancel'}
          </Button>
          <Button
            variant="destructive"
            onClick={purge}
            disabled={loading || busy || !!error || removable.length === 0}
          >
            {busy
              ? 'Deleting…'
              : `Delete ${removable.length} saved credential${removable.length === 1 ? '' : 's'}`}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
