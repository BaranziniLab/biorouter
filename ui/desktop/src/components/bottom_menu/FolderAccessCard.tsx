import './pickers.css';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { listFolderAccess, revokeFolderAccess, type FolderGrantView } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { isBrowserSurface } from '../../utils/surface';
import { folderAccessLifetime } from '../../utils/folderAccessLifetime';
import { PopoverContent } from '../ui/popover';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { FOLDER_ACCESS_COPY } from './copy';
import type { FolderCard } from './useFolderCard';

/**
 * The folder chip's card (spec 3.7): the folder this chat works in, then every other folder the
 * chat may use, each with Revoke. It opens from the footer's folder chip (`DirSwitcher`), which
 * owns the Popover root and its anchor; `useFolderCard` says when it opens.
 *
 * ⚠ **This is the desktop's and the browser's only way to revoke a folder grant**
 * (docs/security/folder-access.md). It used to be a "Folder access" button beside the chip that
 * opened a dialog; the button is gone, the control is not. Revoke works exactly as it did there:
 * the proof-of-user headers, plus the browser's approval key as `X-Folder-Access-Key` when one is
 * typed, then a fresh read of the list.
 *
 * The popover surface, not the tooltip's inverse one: the card holds buttons and a field, and
 * InfoTip's rules keep controls off the inverse ground.
 */

/**
 * How close the card may come to the window's edges: off the 44px window-drag band at the top
 * (`--chrome-height`, plus 8px), where a click on Revoke would be eaten by the drag region, as
 * for the Tools popover (`TOOLS_POPOVER_COLLISION_PADDING`).
 */
export const FOLDER_CARD_COLLISION_PADDING = { top: 52, right: 8, bottom: 8, left: 8 } as const;

/** A full path: a machine string, so mono, and wrapped anywhere rather than cut. */
function FolderPath({ path }: { path: string }) {
  return (
    <span
      dir="ltr"
      className="br-folder-card__path block break-all font-mono text-code text-text-default"
    >
      {path}
    </span>
  );
}

/** The last answer for a chat. Keyed by chat, so one chat's grants never show under another. */
type GrantList = { sessionId: string; grants: FolderGrantView[] };

/**
 * One chat's grants, read each time the card opens (never on mount or on a hover that has not
 * opened it), and the revoke that acts on them. The last list stays on screen while a fresh read
 * runs, and an answer that arrives for a chat the chip no longer shows, or after unmount, is
 * dropped.
 */
function useFolderGrants(sessionId: string | undefined, open: boolean) {
  const [list, setList] = useState<GrantList | null>(null);
  const [loadFailedFor, setLoadFailedFor] = useState<string | null>(null);
  const [revokeFailedFor, setRevokeFailedFor] = useState<string | null>(null);
  const [revokingFor, setRevokingFor] = useState<string | null>(null);
  const currentSession = useRef(sessionId);
  const mounted = useRef(true);
  const loadSeq = useRef(0);

  useEffect(() => {
    currentSession.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const isCurrent = (forSession: string) =>
    mounted.current && currentSession.current === forSession;

  const load = useCallback(async (forSession: string) => {
    const seq = ++loadSeq.current;
    const stillWanted = () =>
      mounted.current && currentSession.current === forSession && loadSeq.current === seq;
    try {
      const result = await listFolderAccess({
        query: { session_id: forSession },
        headers: await userActionHeaders(),
      });
      if (result.error || !Array.isArray(result.data)) throw new Error('load');
      if (!stillWanted()) return;
      setList({ sessionId: forSession, grants: result.data });
      setLoadFailedFor(null);
    } catch {
      if (!stillWanted()) return;
      // Nothing stale stays listed beside the error: it would offer Revoke on rows the backend
      // may no longer hold.
      setList(null);
      setLoadFailedFor(forSession);
    }
  }, []);

  useEffect(() => {
    if (!open || !sessionId) return;
    // A fresh open starts from the last good list (or "Checking…"), not from an old error.
    setRevokeFailedFor(null);
    setLoadFailedFor(null);
    void load(sessionId);
  }, [open, sessionId, load]);

  const revoke = async (grantId: string, approvalKey: string) => {
    if (!sessionId) return;
    const forSession = sessionId;
    setRevokingFor(forSession);
    setRevokeFailedFor(null);
    try {
      const result = await revokeFolderAccess({
        headers: {
          ...(await userActionHeaders()),
          ...(approvalKey ? { 'X-Folder-Access-Key': approvalKey } : {}),
        },
        body: { sessionId: forSession, grantId },
      });
      if (result.error) throw new Error('revoke');
      await load(forSession);
    } catch {
      if (isCurrent(forSession)) setRevokeFailedFor(forSession);
    } finally {
      if (mounted.current) setRevokingFor((current) => (current === forSession ? null : current));
    }
  };

  const shown = sessionId !== undefined && list?.sessionId === sessionId ? list.grants : null;
  return {
    grants: shown,
    loadFailed: sessionId !== undefined && loadFailedFor === sessionId,
    revokeFailed: sessionId !== undefined && revokeFailedFor === sessionId,
    busy: sessionId !== undefined && revokingFor === sessionId,
    revoke,
  };
}

export interface FolderAccessCardProps {
  /** The card's surface id, for the chip's `aria-controls`. */
  id?: string;
  /** The chat whose grants are listed; `undefined` on Home, which lists none and reads nothing. */
  sessionId: string | undefined;
  /** The chat's working folder, full path, or `null` while it is not known yet. */
  workingDir: string | null;
  /** #44: the chip is read-only once the chat has messages, and the hint says so. */
  locked: boolean;
  /** The open state, refs and handlers from `useFolderCard`. */
  card: FolderCard;
}

export function FolderAccessCard({
  id,
  sessionId,
  workingDir,
  locked,
  card,
}: FolderAccessCardProps) {
  const labelId = useId();
  const approvalKeyId = useId();
  const { grants, loadFailed, revokeFailed, busy, revoke } = useFolderGrants(sessionId, card.open);
  // Typed for one revocation and never kept: cleared after every attempt, when the card closes
  // and when the chip moves to another chat.
  const [approvalKey, setApprovalKey] = useState('');
  const browser = isBrowserSurface();
  // What the chip's click does: nothing to say while the folder is not known,
  // since the chip then only holds this card open.
  const hint = locked
    ? FOLDER_ACCESS_COPY.lockedHint
    : workingDir !== null
      ? FOLDER_ACCESS_COPY.chooseHint
      : null;

  useEffect(() => {
    setApprovalKey('');
  }, [card.open, sessionId]);

  const onRevoke = async (grantId: string) => {
    const key = approvalKey;
    try {
      await revoke(grantId, key);
    } finally {
      setApprovalKey('');
      // A Revoke that disabled itself, or left the list, dropped focus on the page: keep a
      // keyboard user in the card.
      const content = card.contentRef.current;
      const active = document.activeElement;
      if (content?.isConnected && (!active || active === document.body)) content.focus();
    }
  };

  return (
    <PopoverContent
      id={id}
      ref={card.contentRef}
      // Above the anchor, which `useFolderCard` stretches from the chip up to the composer
      // card's top: the card hangs over the transcript, never over the draft or its controls.
      side="top"
      align="start"
      collisionPadding={FOLDER_CARD_COLLISION_PADDING}
      aria-label={FOLDER_ACCESS_COPY.cardLabel}
      data-testid="folder-access-card"
      className="br-folder-card"
      {...card.contentHandlers}
    >
      <section className="br-folder-card__section">
        <p className="text-supporting text-text-muted">{FOLDER_ACCESS_COPY.workingFolder}</p>
        {workingDir === null ? (
          <p className="text-supporting text-text-default">
            {FOLDER_ACCESS_COPY.workingFolderUnknown}
          </p>
        ) : (
          <>
            <FolderPath path={workingDir} />
            <p className="text-supporting text-text-muted">
              {FOLDER_ACCESS_COPY.workingFolderNote}
            </p>
          </>
        )}
      </section>

      {sessionId === undefined ? (
        <p className="text-supporting text-text-muted">{FOLDER_ACCESS_COPY.noChatYet}</p>
      ) : (
        <section className="br-folder-card__section">
          <p id={labelId} className="text-supporting text-text-muted">
            {FOLDER_ACCESS_COPY.otherFolders}
          </p>
          {loadFailed ? (
            <p role="alert" className="text-supporting text-text-danger">
              {FOLDER_ACCESS_COPY.loadFailed}
            </p>
          ) : grants === null ? (
            <p role="status" className="text-supporting text-text-muted">
              {FOLDER_ACCESS_COPY.loading}
            </p>
          ) : grants.length === 0 ? (
            <p className="text-supporting text-text-default">{FOLDER_ACCESS_COPY.none}</p>
          ) : (
            <ul className="br-folder-card__grants" aria-labelledby={labelId}>
              {grants.map((grant) => (
                <li key={grant.id} className="br-folder-card__grant">
                  <div className="br-folder-card__grant-text">
                    <FolderPath path={grant.folder} />
                    <p className="text-supporting text-text-muted">
                      {FOLDER_ACCESS_COPY.grantDetail(grant.access, grant.descendants)}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    aria-label={FOLDER_ACCESS_COPY.revokeLabel(grant.folder)}
                    disabled={busy || (browser && !approvalKey)}
                    onClick={() => void onRevoke(grant.id)}
                  >
                    {FOLDER_ACCESS_COPY.revoke}
                  </Button>
                </li>
              ))}
            </ul>
          )}
          {revokeFailed && (
            <p role="alert" className="text-supporting text-text-danger">
              {FOLDER_ACCESS_COPY.revokeFailed}
            </p>
          )}
          {browser && grants !== null && grants.length > 0 && (
            <div className="br-folder-card__key">
              <label htmlFor={approvalKeyId} className="text-supporting text-text-default">
                {FOLDER_ACCESS_COPY.approvalKey}
              </label>
              <Input
                id={approvalKeyId}
                type="password"
                autoComplete="off"
                machine
                value={approvalKey}
                onChange={(event) => setApprovalKey(event.target.value)}
                aria-describedby={`${approvalKeyId}-help`}
              />
              <span id={`${approvalKeyId}-help`} className="text-supporting text-text-muted">
                {FOLDER_ACCESS_COPY.approvalKeyHelp}
              </span>
            </div>
          )}
          <p className="text-supporting text-text-muted">
            {folderAccessLifetime()} {FOLDER_ACCESS_COPY.onceNotListed}
          </p>
        </section>
      )}

      {hint && <p className="text-supporting text-text-muted">{hint}</p>}
    </PopoverContent>
  );
}
