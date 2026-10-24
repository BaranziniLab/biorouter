import { useId, useState } from 'react';
import { listFolderAccess, revokeFolderAccess, type FolderGrantView } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { isBrowserSurface } from '../../utils/surface';
import { Button } from '../ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '../ui/dialog';

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

  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        setOpen(value);
        setApprovalKey('');
        if (value) void load();
      }}
    >
      <DialogTrigger asChild>
        <Button variant="ghost" size="sm" className="text-text-muted">
          Folder access
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Folder access for this chat</DialogTitle>
          <DialogDescription>
            Additional folders approved for this chat include their descendants. Grants expire when
            revoked, the chat is deleted, or the app restarts. Allow once grants are used only for
            their pending operation and are not listed here.
          </DialogDescription>
        </DialogHeader>
        {error && <p role="alert">{error}</p>}
        {busy && <p role="status">Updating folder access…</p>}
        {!busy && !error && grants.length === 0 && <p>No additional folders are authorized.</p>}
        <ul className="space-y-3">
          {grants.map((grant) => (
            <li key={grant.id} className="flex items-center gap-3">
              <div className="min-w-0 flex-1">
                <p className="break-all font-mono text-sm">{grant.folder}</p>
                <p className="text-sm text-text-muted">
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
          <div className="flex flex-col gap-1 text-sm">
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
      </DialogContent>
    </Dialog>
  );
}
