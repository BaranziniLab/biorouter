import { useId, useState } from 'react';
import { listFolderAccess, revokeFolderAccess, type FolderGrantView } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { isBrowserSurface } from '../../utils/surface';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';

export function FolderAccess({ sessionId }: { sessionId: string }) {
  const approvalKeyId = useId();
  const [open, setOpen] = useState(false);
  const [grants, setGrants] = useState<FolderGrantView[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [approvalKey, setApprovalKey] = useState('');

  const load = async () => {
    setBusy(true);
    setError('');
    try {
      const result = await listFolderAccess({
        query: { session_id: sessionId },
        headers: await userActionHeaders(),
      });
      if (result.error || !Array.isArray(result.data)) throw new Error('load');
      setGrants(result.data);
    } catch {
      setError('Folder access could not be loaded. Try again.');
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: string) => {
    setBusy(true);
    setError('');
    try {
      const result = await revokeFolderAccess({
        headers: {
          ...(await userActionHeaders()),
          ...(approvalKey ? { 'X-Folder-Access-Key': approvalKey } : {}),
        },
        body: { sessionId, grantId: id },
      });
      if (result.error) throw new Error('revoke');
      await load();
    } catch {
      setError('Access could not be revoked. The grant is still listed; try again.');
    } finally {
      setApprovalKey('');
      setBusy(false);
    }
  };

  const changeOpen = (value: boolean) => {
    setOpen(value);
    setApprovalKey('');
    if (value) void load();
  };

  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="text-text-muted"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => changeOpen(true)}
      >
        Folder access
      </Button>
      <ModalShell
        open={open}
        onOpenChange={changeOpen}
        size="lg"
        purpose="form"
        title="Folder access for this chat"
        subtitle={
          <>
            Additional folders approved for this chat include their descendants. Grants expire when
            revoked, the chat is deleted, or the app restarts. Allow once grants are used only for
            their pending operation and are not listed here.
          </>
        }
        scrollBody
        bodyClassName="space-y-4 text-body"
      >
        {error && <p role="alert">{error}</p>}
        {busy && <p role="status">Updating folder access…</p>}
        {!busy && !error && grants.length === 0 && <p>No additional folders are authorized.</p>}
        <ul className="space-y-3">
          {grants.map((grant) => (
            <li key={grant.id} className="flex items-center gap-3">
              <div className="min-w-0 flex-1">
                <p className="break-all text-body">{grant.folder}</p>
                <p className="text-supporting text-text-muted">
                  {grant.access} · {grant.descendants ? 'includes descendants' : 'exact files'}
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                disabled={busy || (isBrowserSurface() && !approvalKey)}
                onClick={() => void revoke(grant.id)}
              >
                Revoke
              </Button>
            </li>
          ))}
        </ul>
        {isBrowserSurface() && (
          <div className="flex flex-col gap-1 text-body">
            <label htmlFor={approvalKeyId}>Folder approval key</label>
            <input
              id={approvalKeyId}
              type="password"
              autoComplete="off"
              value={approvalKey}
              onChange={(event) => setApprovalKey(event.target.value)}
              aria-describedby={`${approvalKeyId}-help`}
            />
            <span id={`${approvalKeyId}-help`} className="text-text-muted">
              Use the separate key configured with biorouter serve --folder-access-approval. It is
              cleared after each revocation.
            </span>
          </div>
        )}
      </ModalShell>
    </>
  );
}
