import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
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
import { chatAccessRoute, chatAccessRouteState } from './ChatConnectNote';
import { accessCopy } from './copy';
import { InlineConfirm, RevokeResultNote } from './RevokeControls';
import { revokeGrant, type RevokeOutcome } from './useCrewGrants';

// ── What Enter says in a held chat ───────────────────────────────────────────────────────────
// A chat whose Crew access lapsed holds its composer (`access.blocksComposer`), and Enter used to do
// nothing at all (live QA round 1, T-55). The bar is the one place that knows the channel's name, so
// it publishes the sentence here for the composer to show when Enter is pressed. Keyed by chat and
// by the bar that published it, so two panes showing one chat never erase each other's answer.

export interface CrewComposerHold {
  title: string;
  message: string;
}

const holds = new Map<string, Map<object, CrewComposerHold>>();
const holdListeners = new Set<() => void>();

function publishHold(sessionId: string, token: object, hold: CrewComposerHold | null) {
  const forChat = holds.get(sessionId) ?? new Map<object, CrewComposerHold>();
  const current = forChat.get(token);
  if (hold === null) {
    if (!current) return;
    forChat.delete(token);
  } else {
    if (current && current.title === hold.title && current.message === hold.message) return;
    forChat.set(token, hold);
  }
  if (forChat.size === 0) holds.delete(sessionId);
  else holds.set(sessionId, forChat);
  for (const listener of [...holdListeners]) listener();
}

function subscribeHolds(listener: () => void) {
  holdListeners.add(listener);
  return () => {
    holdListeners.delete(listener);
  };
}

function currentHold(sessionId: string | null | undefined): CrewComposerHold | null {
  if (!sessionId) return null;
  const forChat = holds.get(sessionId);
  if (!forChat) return null;
  for (const hold of forChat.values()) return hold;
  return null;
}

/**
 * Why the chat for `sessionId` cannot send, while its Crew access bar shows the access lapsed; else
 * `null`. For the composer's Enter, which otherwise does nothing in a held chat. Never fetches.
 */
export function useCrewComposerHold(sessionId: string | null | undefined): CrewComposerHold | null {
  return useSyncExternalStore(
    subscribeHolds,
    () => currentHold(sessionId),
    () => null
  );
}

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
 * - **Connected:** a small "Crew · #general" chip (it opens the chat's access pane in Crew) and
 *   **Revoke access**, a real button that asks inline first — so the chat itself says it is
 *   connected and where the control that ends it lives.
 * - **Revoked or expired:** a calm notice, "Crew access to #general was removed. This chat has team
 *   content, so it can't continue.", with **Start a new chat** and **Grant access again**, which
 *   opens this chat's consent in Crew in one hop. The chat holds its composer
 *   (`access.blocksComposer`) so the person reads why instead of a model error, and Enter says it
 *   again ({@link useCrewComposerHold}); the daemon refuses the turn either way.
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
  const [holdToken] = useState(() => ({}));
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

  const unconfirmed = outcome?.kind === 'unconfirmed' || access.unconfirmed;
  const lapsed =
    outcome?.kind === 'revoked' || access.state === 'revoked' || access.state === 'expired';
  const holdMessage =
    sessionId && grant && lapsed
      ? access.state === 'expired'
        ? accessCopy.chatBlockedSendExpired(destination)
        : accessCopy.chatBlockedSendRevoked(destination)
      : null;

  useEffect(() => {
    if (!sessionId) return;
    publishHold(
      sessionId,
      holdToken,
      holdMessage ? { title: accessCopy.chatBlockedSendTitle, message: holdMessage } : null
    );
    return () => publishHold(sessionId, holdToken, null);
  }, [sessionId, holdToken, holdMessage]);

  if (!sessionId || !grant) return null;

  const title = sanitizeDisplayText(chatTitle);
  const chat = title && !isMachineIdShaped(title) ? title : null;
  // One hop: Crew opens this chat's access pane itself, rather than a note whose button does.
  const openAccess = () => navigate(chatAccessRoute(sessionId), { state: chatAccessRouteState() });
  const startNewChat = () => navigateWithViewTransition(navigate, '/pair', { newChat: true });

  const revoke = async () => {
    setConfirming(false);
    setRevoking(true);
    const result = await revokeGrant(grant.connection_id, sessionId);
    setRevoking(false);
    setOutcome(result);
  };

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
              <Button type="button" variant="secondary" size="sm" onClick={openAccess}>
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
                  onClick={openAccess}
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
            // A real button, not ghost text beside a chip that also means "manage access".
            <Button
              ref={revokeButton}
              type="button"
              variant="secondary"
              size="sm"
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
