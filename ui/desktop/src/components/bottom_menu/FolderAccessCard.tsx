import './pickers.css';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { listFolderAccess, revokeFolderAccess, type FolderGrantView } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { isBrowserSurface } from '../../utils/surface';
import { folderAccessLifetime } from '../../utils/folderAccessLifetime';
import { InfoTip, useInfoTipId } from '../ui/info-tip';
import { Input } from '../ui/input';
import { FOLDER_ACCESS_COPY } from './copy';
import {
  FooterCard,
  FooterCardAction,
  FooterCardActions,
  FooterCardDetail,
  FooterCardList,
  FooterCardNote,
  FooterCardPath,
  FooterCardRow,
  FooterCardValue,
} from './FooterCard';
import { FooterCardHelpSection } from './FooterCardHelpSection';
import type { FooterCard as FooterCardState } from './useFooterCard';

/**
 * The folder chip's card (spec 3.7), one of the footer's three `FooterCard`s: the folder this
 * chat works in (with Choose folder… while the chat is empty), then every other folder the chat
 * may use, each with Revoke. It opens from the footer's folder chip (`DirSwitcher`), which owns
 * the Popover root and its anchor; `useFooterCard` says when it opens.
 *
 * Only the readings are drawn; what explains them waits in an InfoTip beside each label (owner,
 * 2026-11-13: one line up front, the detail on hover or on click). Which tools ask for folder
 * approval is the Working folder label's, on Home and in a chat alike; when a session grant ends
 * and that one-time grants are not listed is the Other approved folders label's; and in a
 * browser, where the approval key comes from is the key field's, which keeps it as the field's
 * accessible description.
 *
 * ⚠ **This is the desktop's and the browser's only way to revoke a folder grant**
 * (docs/security/folder-access.md). It used to be a "Folder access" button beside the chip that
 * opened a dialog; the button is gone, the control is not. Revoke works exactly as it did there:
 * the proof-of-user headers, plus the browser's approval key as `X-Folder-Access-Key` when one is
 * typed, then a fresh read of the list.
 */

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
  id: string;
  /**
   * The chat whose grants are listed. `undefined` on Home and `''` in a new chat that has no
   * session yet (`#/pair` before its first message): neither can hold a grant, so the card lists
   * no other folders and reads nothing for them.
   */
  sessionId: string | undefined;
  /** The chat's working folder, full path, or `null` while it is not known yet. */
  workingDir: string | null;
  /** #44: the folder is fixed once the chat has messages, and the card says so. */
  locked: boolean;
  /**
   * Opens the native folder chooser. Given only while the chat is empty and its folder is known
   * (`DirSwitcher`, the one place that decides it): the card offers Choose folder… exactly when it
   * is given. The locked chip never passes it (#44).
   */
  onChooseFolder?: () => void;
  /** The open state, refs and handlers from `useFooterCard`. */
  card: FooterCardState;
}

export function FolderAccessCard({
  id,
  sessionId,
  workingDir,
  locked,
  onChooseFolder,
  card,
}: FolderAccessCardProps) {
  const labelId = useId();
  const approvalKeyId = useId();
  const approvalKeyHelpId = useInfoTipId();
  // One test of "is this a chat" for the section and its read alike. They used to differ: the
  // section showed for any defined id while the read skipped an empty one, so a new chat's card
  // said "Checking folder access…" for as long as it stayed open.
  const chatId = sessionId || undefined;
  const { grants, loadFailed, revokeFailed, busy, revoke } = useFolderGrants(chatId, card.open);
  // Typed for one revocation and never kept: cleared after every attempt, when the card closes
  // and when the chip moves to another chat.
  const [approvalKey, setApprovalKey] = useState('');
  const browser = isBrowserSurface();

  useEffect(() => {
    setApprovalKey('');
  }, [card.open, chatId]);

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
    <FooterCard
      card={card}
      id={id}
      label={FOLDER_ACCESS_COPY.cardLabel}
      align="start"
      testId="folder-access-card"
    >
      <FooterCardHelpSection
        label={FOLDER_ACCESS_COPY.workingFolder}
        about={FOLDER_ACCESS_COPY.aboutWorkingFolder}
        help={FOLDER_ACCESS_COPY.whoAsks}
      >
        {workingDir === null ? (
          <FooterCardValue>{FOLDER_ACCESS_COPY.workingFolderUnknown}</FooterCardValue>
        ) : (
          <FooterCardPath path={workingDir} />
        )}
        {locked && <FooterCardNote>{FOLDER_ACCESS_COPY.lockedNote}</FooterCardNote>}
        {/* `DirSwitcher` alone decides whether the folder can be chosen (#44), so the card has
            one rule to follow rather than a second copy of it that nothing could test. */}
        {onChooseFolder && (
          <FooterCardActions>
            <FooterCardAction onClick={onChooseFolder}>
              {FOLDER_ACCESS_COPY.chooseFolder}
            </FooterCardAction>
          </FooterCardActions>
        )}
      </FooterCardHelpSection>

      {chatId !== undefined && (
        <FooterCardHelpSection
          label={FOLDER_ACCESS_COPY.otherFolders}
          labelId={labelId}
          about={FOLDER_ACCESS_COPY.aboutOtherFolders}
          help={`${folderAccessLifetime()} ${FOLDER_ACCESS_COPY.onceNotListed}`}
        >
          {loadFailed ? (
            <FooterCardNote role="alert" tone="danger">
              {FOLDER_ACCESS_COPY.loadFailed}
            </FooterCardNote>
          ) : grants === null ? (
            <FooterCardNote role="status">{FOLDER_ACCESS_COPY.loading}</FooterCardNote>
          ) : grants.length === 0 ? (
            <FooterCardValue>{FOLDER_ACCESS_COPY.none}</FooterCardValue>
          ) : (
            <FooterCardList labelledBy={labelId}>
              {grants.map((grant) => (
                <FooterCardRow
                  key={grant.id}
                  action={
                    <FooterCardAction
                      aria-label={FOLDER_ACCESS_COPY.revokeLabel(grant.folder)}
                      disabled={busy || (browser && !approvalKey)}
                      onClick={() => void onRevoke(grant.id)}
                    >
                      {FOLDER_ACCESS_COPY.revoke}
                    </FooterCardAction>
                  }
                >
                  <FooterCardPath path={grant.folder} />
                  <FooterCardDetail>
                    {FOLDER_ACCESS_COPY.grantDetail(grant.access, grant.descendants)}
                  </FooterCardDetail>
                </FooterCardRow>
              ))}
            </FooterCardList>
          )}
          {revokeFailed && (
            <FooterCardNote role="alert" tone="danger">
              {FOLDER_ACCESS_COPY.revokeFailed}
            </FooterCardNote>
          )}
          {browser && grants !== null && grants.length > 0 && (
            <div className="br-footer-card__key">
              {/* A card label like every other: the muted 12/16 of a section's label. Its help is
                  the InfoTip beside it, never inside the <label>, and the field's description. */}
              <div className="flex min-w-0 items-center">
                <label htmlFor={approvalKeyId} className="br-footer-card__label">
                  {FOLDER_ACCESS_COPY.approvalKey}
                </label>
                <InfoTip
                  id={approvalKeyHelpId}
                  label={FOLDER_ACCESS_COPY.aboutApprovalKey}
                  help={FOLDER_ACCESS_COPY.approvalKeyHelp}
                />
              </div>
              <Input
                id={approvalKeyId}
                type="password"
                autoComplete="off"
                machine
                value={approvalKey}
                onChange={(event) => setApprovalKey(event.target.value)}
                aria-describedby={approvalKeyHelpId}
              />
            </div>
          )}
        </FooterCardHelpSection>
      )}
    </FooterCard>
  );
}
