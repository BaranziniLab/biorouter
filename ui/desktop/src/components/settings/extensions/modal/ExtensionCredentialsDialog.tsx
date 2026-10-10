import { useEffect, useState } from 'react';
import {
  getExtensionCredentials,
  purgeExtensionCredentials,
  type ExtensionCredential,
} from '../../../../api';
import { userActionHeaders } from '../../../../utils/userAction';
import { Button } from '../../../ui/button';
import { InfoTip } from '../../../ui/info-tip';
import { ModalShell } from '../../../ModalShell';
import { EXTENSION_CREDENTIALS_COPY as COPY } from '../copy';
import './extension-modal.css';

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
        if (!cancelled) setError(COPY.loadFailed);
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
      setError(COPY.deleteFailed);
    } finally {
      setBusy(false);
    }
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      size="md"
      // Nothing dismisses a deletion in flight; otherwise a form-like review whose list the
      // person is reading, so a stray click on the scrim does not close it.
      purpose={busy ? 'required' : 'form'}
      title={COPY.title(name)}
      subtitle={COPY.subtitle(name)}
      footer={
        <>
          {/* First focus lands on the safe answer of a destructive dialog, not on the help glyph
              that is the first thing in its body. */}
          <Button variant="secondary" onClick={onClose} disabled={busy} autoFocus>
            {deleted ? COPY.done : COPY.cancel}
          </Button>
          <Button
            variant="destructive"
            onClick={purge}
            disabled={loading || busy || !!error || removable.length === 0}
          >
            {busy ? COPY.deleting : COPY.deleteCount(removable.length)}
          </Button>
        </>
      }
      bodyClassName="flex flex-col gap-3 py-3"
    >
      {loading ? (
        <p role="status" className="text-body text-text-muted">
          {COPY.checking}
        </p>
      ) : credentials.length === 0 && !error ? (
        <p className="text-body text-text-muted">{COPY.none}</p>
      ) : (
        <ul className="br-ext-credentials">
          {credentials.map((credential) => (
            <li key={credential.key}>
              <span className="br-ext-credential-key text-label text-text-default">
                {credential.key}
              </span>
              <p className="text-supporting text-text-muted">
                {!credential.stored
                  ? COPY.noValue
                  : credential.used_by.length > 0
                    ? COPY.kept(credential.used_by.join('; '))
                    : COPY.willDelete}
              </p>
            </li>
          ))}
        </ul>
      )}
      {deleted ? (
        <p role="status" className="flex items-center text-supporting text-text-muted">
          {COPY.deleted}
          <InfoTip label={COPY.title(name)} help={COPY.deletedHelp} />
        </p>
      ) : (
        // The consequence stays on screen, at the moment of decision; what stays unchanged is
        // the glyph's help.
        <p className="flex items-center text-supporting text-text-muted">
          {COPY.restart}
          <InfoTip label={COPY.title(name)} help={COPY.help} />
        </p>
      )}
      {error && (
        <p role="alert" className="text-body text-text-danger">
          {error}
        </p>
      )}
    </ModalShell>
  );
}
