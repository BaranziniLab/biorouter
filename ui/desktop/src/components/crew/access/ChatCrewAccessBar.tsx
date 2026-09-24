import { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { AlertTriangle, Users } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { cn } from '../../../utils';
import { navigateWithViewTransition } from '../../../utils/navigationUtils';
import { isMachineIdShaped, sanitizeDisplayText } from '../identity';
import type { ChatCrewAccess } from './chatCrewAccess';
import { accessCopy } from './copy';
import { InlineConfirm, RevokeResultNote } from './RevokeControls';
import { revokeGrant, type RevokeOutcome } from './useCrewGrants';

export interface ChatCrewAccessBarProps {
  /** The chat's Crew access, from `useChatCrewAccess(sessionId)`. */
  access: ChatCrewAccess;
  /** The chat's title, for the revoke question. */
  chatTitle?: string | null;
  /** Layout only. */
  className?: string;
}

/**
 * What an ordinary chat shows above its composer about Crew.
 *
 * - **Connected:** a small "Crew · #general" chip (it opens the chat's access in Crew) and
 *   **Revoke access**, which asks inline first — so the chat itself says it is connected and where
 *   the control that ends it lives.
 * - **Revoked or expired:** a calm notice, "Crew access to #general was removed. This chat has team
 *   content, so it can't continue.", with **Start a new chat** and **Grant access again**. The chat
 *   holds its composer (`access.blocksComposer`) so the person reads why instead of a model error;
 *   the daemon refuses the turn either way.
 * - A revoke that stopped only on this device, or was not revoked at all, says so, with Retry.
 *
 * Renders nothing when the chat has no Crew grant or the lookup failed.
 */
export function ChatCrewAccessBar({ access, chatTitle, className }: ChatCrewAccessBarProps) {
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const [outcome, setOutcome] = useState<RevokeOutcome | null>(null);
  const revokeButton = useRef<HTMLButtonElement>(null);
  const { sessionId, grant, destination } = access;

  useEffect(() => {
    setConfirming(false);
    setOutcome(null);
  }, [sessionId]);

  // A later answer from the daemon replaces what this bar last saw a revoke come to.
  useEffect(() => {
    if (access.state !== 'active')
      setOutcome((current) => (current?.kind === 'revoked' ? null : current));
  }, [access.state]);

  if (!sessionId || !grant) return null;

  const title = sanitizeDisplayText(chatTitle);
  const chat = title && !isMachineIdShaped(title) ? title : null;
  const openCrew = () => navigate(`/crew?sessionId=${encodeURIComponent(sessionId)}`);
  const startNewChat = () => navigateWithViewTransition(navigate, '/pair', { newChat: true });

  const revoke = async () => {
    setConfirming(false);
    setRevoking(true);
    const result = await revokeGrant(grant.connection_id, sessionId);
    setRevoking(false);
    setOutcome(result);
  };

  const unconfirmed = outcome?.kind === 'unconfirmed' || access.unconfirmed;
  const lapsed =
    outcome?.kind === 'revoked' || access.state === 'revoked' || access.state === 'expired';

  if (lapsed) {
    return (
      <div className={cn('flex flex-col gap-2', className)} data-testid="crew-chat-access-bar">
        {unconfirmed ? (
          <RevokeResultNote
            outcome={
              outcome?.kind === 'unconfirmed'
                ? outcome
                : { kind: 'unconfirmed', message: accessCopy.unconfirmed }
            }
            chat={chat}
            retrying={revoking}
            onRetry={() => void revoke()}
          />
        ) : null}
        <Note
          tone="neutral"
          icon={AlertTriangle}
          role="status"
          testId="crew-chat-access-lapsed"
          action={
            <div className="flex flex-wrap items-center gap-2">
              <Button type="button" variant="secondary" size="sm" onClick={startNewChat}>
                {accessCopy.chatNewChat}
              </Button>
              <Button type="button" variant="ghost" size="sm" onClick={openCrew}>
                {accessCopy.chatGrantAgain}
              </Button>
            </div>
          }
        >
          {access.state === 'expired'
            ? accessCopy.chatExpired(destination)
            : accessCopy.chatRevoked(destination)}
        </Note>
      </div>
    );
  }

  if (access.state !== 'active') return null;

  return (
    <div className={cn('flex flex-col gap-2', className)} data-testid="crew-chat-access-bar">
      {outcome ? (
        <RevokeResultNote
          outcome={outcome}
          chat={chat}
          retrying={revoking}
          onRetry={() => void revoke()}
        />
      ) : null}
      {confirming ? (
        <InlineConfirm
          question={accessCopy.confirm(chat, destination)}
          confirmLabel={accessCopy.confirmRevoke}
          cancelLabel={accessCopy.confirmKeep}
          pending={revoking}
          onConfirm={() => void revoke()}
          onCancel={() => {
            setConfirming(false);
            window.setTimeout(() => revokeButton.current?.focus(), 0);
          }}
        />
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <Badge variant="chip" asChild>
                <button
                  type="button"
                  aria-label={accessCopy.chatChipName(destination)}
                  onClick={openCrew}
                  className="tint-interactive cursor-pointer"
                >
                  <Users className="h-3.5 w-3.5" aria-hidden />
                  <bdi>{accessCopy.chatChip(destination)}</bdi>
                </button>
              </Badge>
            </TooltipTrigger>
            <TooltipContent side="top">{accessCopy.chatChipTip(destination)}</TooltipContent>
          </Tooltip>
          {outcome ? null : (
            <Button
              ref={revokeButton}
              type="button"
              variant="ghost"
              size="xs"
              disabled={revoking}
              onClick={() => setConfirming(true)}
            >
              {accessCopy.revokeButton}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
