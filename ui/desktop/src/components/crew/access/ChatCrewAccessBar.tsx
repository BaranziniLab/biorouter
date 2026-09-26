import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'react-toastify';
import { AlertTriangle, Users } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { cn } from '../../../utils';
import { navigateWithViewTransition } from '../../../utils/navigationUtils';
import { isMachineIdShaped, sanitizeDisplayText } from '../identity';
import type { ChatCrewAccess } from './chatCrewAccess';
import { chatAccessRoute, chatAccessRouteState, chatConnectRouteState } from './ChatConnectNote';
import { accessCopy } from './copy';
import {
  InlineConfirm,
  RevocationConfirmedNote,
  RevokeResultNote,
  useConfirmedAfterWait,
} from './RevokeControls';
import { revokeGrant, type RevokeOutcome } from './useCrewGrants';

// ── What Enter says in a held chat ───────────────────────────────────────────────────────────
// A chat whose Crew access lapsed holds its composer (`access.blocksComposer`), and Enter used to do
// nothing at all (live QA round 1, T-55). The bar is the one place that knows the channel's name, so
// it publishes the sentence here for the composer to show when Enter is pressed. Keyed by chat and
// by the bar that published it, so two panes showing one chat never erase each other's answer.

export interface CrewComposerHold {
  title: string;
  message: string;
  /**
   * The held composer's placeholder: Send is grey, and the empty box says why before Enter does
   * (live QA round 4, Q4-15).
   */
  placeholder: string;
  /** The one toast id this hold is shown under ({@link crewHoldToastId}). */
  toastId: string;
}

/**
 * The toast id of a hold's "Can't send": fixed for the hold, so pressing Enter twice shows one
 * toast, and the bar can take it down again. It is the id `toastWarning` gives a warning with this
 * title and message (its dedupe key), which is how the composer shows it today.
 */
export function crewHoldToastId(title: string, message: string): string {
  return `warning:${title}:${message}`;
}

/** Take down the hold's toast: the reason no longer stands, or its chat is not on screen. */
function dismissHoldToast(hold: CrewComposerHold | null) {
  if (hold) toast.dismiss(hold.toastId);
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
    if (current && current.toastId === hold.toastId) return;
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
 * - **Offline:** the grant stands but its Crew connection is down, so the next turn would fail as
 *   a model error (Q2-08). A neutral note says so, with **Connect in Crew**, which connects and
 *   lands on the chat's channel (Q3-08), and **Revoke access**, which needs no connection: the
 *   grant stops on this device at once and the workspace confirms it by itself later (F3). Nothing
 *   is held. The chat notices the outage while it is
 *   watched: `useChatCrewAccess` re-reads the connections while it holds a grant (Q3-04). When the
 *   cause is the network, which the daemon retries by itself, it says Crew will reconnect when the
 *   network is back, with **Connect now** (Q4-06).
 * - **Revoked or expired:** a calm notice, "Crew access to #general was removed, so this chat
 *   can't continue. …", with **Start a new chat** and **Grant access again**, which opens this
 *   chat's consent in Crew in one hop. The chat holds its composer (`access.blocksComposer`) so the
 *   person reads why instead of a model error, and Enter says it again
 *   ({@link useCrewComposerHold}); the daemon refuses the turn either way.
 * - **A finished task:** "This task is finished. Its access to #general ended when it finished."
 *   in a neutral note with no warning glyph, and only **Start a new chat**: nothing went wrong, and
 *   a task's access is not granted again from its chat (Q2-09).
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

  // A later answer from the daemon replaces what this bar last saw a revoke come to — including a
  // 503 the daemon has since confirmed with the workspace by itself (F3).
  useEffect(() => {
    if (access.state !== 'active')
      setOutcome((current) => (current?.kind === 'revoked' ? null : current));
  }, [access.state]);
  useEffect(() => {
    if (access.revocationConfirmed)
      setOutcome((current) => (current?.kind === 'unconfirmed' ? null : current));
  }, [access.revocationConfirmed]);

  const unconfirmed = outcome?.kind === 'unconfirmed' || access.unconfirmed;
  const confirmedAfterWait = useConfirmedAfterWait(
    grant?.run_id ?? null,
    unconfirmed,
    access.revocationConfirmed
  );
  const settingsChanged = access.state === 'expired' && access.expiredBecause === 'settings';
  const finished = access.state === 'finished';
  const lapsed =
    outcome?.kind === 'revoked' ||
    access.state === 'revoked' ||
    access.state === 'expired' ||
    finished;
  const holdMessage =
    sessionId && grant && lapsed
      ? finished
        ? accessCopy.chatBlockedSendTaskFinished(destination)
        : settingsChanged
          ? accessCopy.chatBlockedSendSettingsChanged(destination)
          : access.state === 'expired'
            ? accessCopy.chatBlockedSendExpired(destination)
            : accessCopy.chatBlockedSendRevoked(destination)
      : null;

  // The composer shows the hold's toast on Enter. When the reason goes — access is back, or this
  // bar and its chat leave the screen — the toast goes with it, rather than following the person
  // into Crew (Q2-74).
  useEffect(() => {
    if (!sessionId || !holdMessage) return;
    const hold: CrewComposerHold = {
      title: accessCopy.chatBlockedSendTitle,
      message: holdMessage,
      placeholder: finished
        ? accessCopy.chatBlockedPlaceholderTaskFinished
        : accessCopy.chatBlockedPlaceholder,
      toastId: crewHoldToastId(accessCopy.chatBlockedSendTitle, holdMessage),
    };
    publishHold(sessionId, holdToken, hold);
    return () => {
      publishHold(sessionId, holdToken, null);
      dismissHoldToast(hold);
    };
  }, [sessionId, holdToken, holdMessage, finished]);

  if (!sessionId || !grant) return null;

  const title = sanitizeDisplayText(chatTitle);
  const chat = title && !isMachineIdShaped(title) ? title : null;
  const leaveChat = () => dismissHoldToast(currentHold(sessionId));
  // One hop: Crew opens this chat's access pane itself, rather than a note whose button does.
  const openAccess = () => {
    leaveChat();
    navigate(chatAccessRoute(sessionId), { state: chatAccessRouteState() });
  };
  // One click, as its label says (live QA round 3, Q3-08): Crew connects the grant's connection on
  // arrival — the press here is the person's own connect, so it is user-initiated — and lands on
  // this chat's channel with its access pane open. Connecting is not a consent: the grant already
  // stands, and nothing here grants or widens it.
  const connectInCrew = () => {
    leaveChat();
    navigate(chatAccessRoute(sessionId), { state: chatConnectRouteState(grant.connection_id) });
  };
  const startNewChat = () => {
    leaveChat();
    navigateWithViewTransition(navigate, '/pair', { newChat: true });
  };

  const revoke = async () => {
    setConfirming(false);
    setRevoking(true);
    const result = await revokeGrant(grant.connection_id, sessionId, grant);
    setRevoking(false);
    setOutcome(result);
    // Read the grant again now rather than trusting the announcement to reach this chat's lookup:
    // the composer's hold follows the lookup (Q2-73).
    access.refetch();
  };

  if (finished) {
    return (
      <div className={cn('flex flex-col gap-2', className)} data-testid="crew-chat-access-bar">
        <Note
          tone="neutral"
          role="status"
          testId="crew-chat-access-finished"
          action={
            <Button type="button" variant="secondary" size="sm" onClick={startNewChat}>
              {accessCopy.chatNewChat}
            </Button>
          }
        >
          {accessCopy.chatTaskFinished(destination)}
        </Note>
      </div>
    );
  }

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
            confirmation={access.connectionUp ? 'confirming' : 'offline'}
            onRetry={() => void revoke()}
          />
        ) : confirmedAfterWait.shown ? (
          <RevocationConfirmedNote onDismiss={confirmedAfterWait.dismiss} />
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
          {settingsChanged
            ? accessCopy.chatSettingsChanged(destination)
            : access.state === 'expired'
              ? accessCopy.chatExpired(destination)
              : accessCopy.chatRevoked(destination)}
        </Note>
      </div>
    );
  }

  // What a revoke came to, and its inline question: the same in the offline bar and the live one.
  const outcomeNote = outcome ? (
    <RevokeResultNote
      outcome={outcome}
      chat={chat}
      retrying={revoking}
      confirmation={access.connectionUp ? 'confirming' : 'offline'}
      onRetry={() => void revoke()}
    />
  ) : null;
  const revokeQuestion = (
    <InlineConfirm
      question={accessCopy.confirm(chat, destination)}
      detail={accessCopy.confirmStops}
      confirmLabel={accessCopy.confirmRevoke}
      cancelLabel={accessCopy.confirmKeep}
      pending={revoking}
      onConfirm={() => void revoke()}
      onCancel={() => {
        setConfirming(false);
        window.setTimeout(() => revokeButton.current?.focus(), 0);
      }}
    />
  );
  // A real button, not ghost text beside a chip that also means "manage access".
  const revokeControl = (
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
  );

  if (access.state === 'offline') {
    // A network drop is dialled again by the daemon itself once the network is back (Q4-01), so
    // the bar says so and its button is an offer, not a requirement (Q4-06). Every other cause —
    // sign-in, a host key, a membership that ended, a Disconnect — keeps "until you connect".
    //
    // Revoke stays here while offline (final polish, observation (a)): it needs no connection.
    // The daemon stops the grant on this device at once (a 503, "Stopped on this device") and
    // confirms it with the workspace by itself once the connection is back (F3). Without it the
    // chat offered only Connect, and stopping the chat meant going to Crew first.
    const network = access.offlineCause === 'network';
    return (
      <div className={cn('flex flex-col gap-2', className)} data-testid="crew-chat-access-bar">
        {outcomeNote}
        <Note
          tone="neutral"
          role="status"
          testId="crew-chat-access-offline"
          action={
            <div className="flex flex-wrap items-center gap-2">
              <Button type="button" variant="secondary" size="sm" onClick={connectInCrew}>
                {network ? accessCopy.chatConnectNow : accessCopy.chatConnectInCrew}
              </Button>
              {confirming || outcome ? null : revokeControl}
            </div>
          }
        >
          {network ? accessCopy.chatOfflineNetwork : accessCopy.chatOffline(destination)}
        </Note>
        {confirming ? revokeQuestion : null}
      </div>
    );
  }

  if (access.state !== 'active') return null;

  return (
    <div className={cn('flex flex-col gap-2', className)} data-testid="crew-chat-access-bar">
      {outcomeNote}
      {confirming ? (
        revokeQuestion
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
          {outcome ? null : revokeControl}
        </div>
      )}
    </div>
  );
}
