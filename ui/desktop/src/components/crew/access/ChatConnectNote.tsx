import { useEffect, useMemo } from 'react';
import { useLocation } from 'react-router-dom';
import { sessionGrantState } from '../api/grants';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { channelName, connectionNames, identityCopy } from '../identity';
import { useCrew, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { channelLabels } from './accessRows';
import { accessCopy } from './copy';
import { useCrewGrants } from './useCrewGrants';

export interface ChatConnectNoteProps {
  /** Layout only. */
  className?: string;
}

// ── One hop from the ordinary chat ───────────────────────────────────────────────────────────
// The chat's own Crew controls ("Grant access again", the "Crew · #general" chip) used to land on
// this note, whose button then opened the pane: three screens to undo one mistaken revoke (live QA
// round 1, T-55). They now navigate with this route state, and the note opens the pane itself once
// it knows the chat's grant state, exactly as its own button would. Opening the pane grants
// nothing: Allow and Revoke stay the person's clicks.

const CHAT_ACCESS_INTENT = 'crewOpenChatAccess';

/** Intents already honoured, so Back to this entry, or the note remounting, never reopens it. */
const consumedIntents = new Set<string>();

function newIntentId(): string {
  return typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function intentIdOf(state: unknown): string | null {
  if (!state || typeof state !== 'object') return null;
  const id = (state as Record<string, unknown>)[CHAT_ACCESS_INTENT];
  return typeof id === 'string' && id ? id : null;
}

/** Where a chat's Crew controls go: Crew, with this chat's note (and so its access) in view. */
export function chatAccessRoute(sessionId: string): string {
  return `/crew?sessionId=${encodeURIComponent(sessionId)}`;
}

/** Route state for {@link chatAccessRoute} that opens the Chat access pane on arrival. */
export function chatAccessRouteState(): Record<string, string> {
  return { [CHAT_ACCESS_INTENT]: newIntentId() };
}

/**
 * The note above the Crew composer when a chat sent the person here with `/crew`
 * (`?sessionId=`): what this chat can do in Crew right now, and the one control that changes it
 * (ui-redesign-spec, "Revoke", the chat-connect note table).
 *
 * | Grant state | Note | Action |
 * |---|---|---|
 * | Loading | Checking this chat's access… | — |
 * | None | Connect this chat to #methods? | Review access |
 * | Active here | This chat can read and post in #methods. | Manage access |
 * | Active elsewhere | This chat already uses #raw-data. | Manage access |
 * | Revoked / Expired | This chat's Crew access was revoked. / …expired. | Grant again |
 *
 * Every action opens the Chat access pane; none of them grants or revokes by itself. Arriving with
 * {@link chatAccessRouteState} presses that action once, as soon as there is one. Renders nothing
 * without `?sessionId=`.
 */
export function ChatConnectNote({ className }: ChatConnectNoteProps) {
  const {
    grantSessionId,
    connections,
    connectionsState,
    connectionId,
    channel,
    snapshot,
    openPane,
    subscribeSurfaceReset,
  } = useCrew();
  const ids = useMemo(() => connections.map((item) => item.id), [connections]);
  const grants = useCrewGrants(ids, {
    enabled: Boolean(grantSessionId),
    cacheScope: subscribeSurfaceReset,
  });
  const { refetch } = grants;
  useCrewSurfaceReset((reason) => {
    if (grantSessionId && reason === 'refresh') refetch();
  });
  const labels = useMemo(() => channelLabels(snapshot), [snapshot]);
  const workspaces = useMemo(() => connectionNames(connections), [connections]);
  const location = useLocation();
  const intentId = intentIdOf(location.state);

  const grant = grantSessionId
    ? (grants.grants.find((item) => item.session_id === grantSessionId) ?? null)
    : null;
  const here = channel ? channelName(channel) : null;

  let text: string | null = null;
  let action: { label: string; name?: string } | null = null;
  let retry = false;
  if (!grantSessionId) {
    text = null;
  } else if (connectionsState !== 'loaded' || (!grant && grants.status === 'loading')) {
    text = accessCopy.noteChecking;
  } else if (!grant && grants.anyFailed) {
    text = grants.error ?? accessCopy.listFailed;
    retry = true;
  } else if (!grant) {
    if (here) {
      text = accessCopy.noteNone(here);
      action = { label: accessCopy.noteReview, name: accessCopy.noteReviewName };
    }
  } else {
    const state = sessionGrantState(grant);
    if (state === 'revoked') {
      text = accessCopy.noteRevoked;
      action = { label: accessCopy.noteGrantAgain };
    } else if (state === 'expired') {
      text = accessCopy.noteExpired;
      action = { label: accessCopy.noteGrantAgain };
    } else {
      const sameConnection = grant.connection_id === connectionId;
      if (sameConnection && channel && grant.channel_id === channel.id) {
        text = accessCopy.noteActive(here ?? channelName(channel));
      } else if (sameConnection) {
        text = accessCopy.noteActiveElsewhere(
          labels.get(grant.channel_id) ?? accessCopy.unknownChannel
        );
      } else {
        text = accessCopy.noteActiveOtherWorkspace(
          workspaces.get(grant.connection_id) ?? identityCopy.unnamedWorkspace
        );
      }
      action = { label: accessCopy.noteManage };
    }
  }

  // Not while the grant state is still being read, nor when the read failed: the one hop opens the
  // pane the note's own button would, and there is no button until the note knows which.
  const actionable = action !== null;
  useEffect(() => {
    if (!intentId || !grantSessionId || !actionable || consumedIntents.has(intentId)) return;
    consumedIntents.add(intentId);
    openPane({ mode: 'chat-access', sessionId: grantSessionId });
  }, [intentId, grantSessionId, actionable, openPane]);

  if (!grantSessionId || text === null) return null;

  const openAccess = () => openPane({ mode: 'chat-access', sessionId: grantSessionId });

  return (
    <Note
      tone={retry ? 'warning' : 'neutral'}
      role={retry ? 'alert' : 'status'}
      className={className}
      testId="crew-chat-connect-note"
      action={
        retry ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-label={accessCopy.listRetryName}
            onClick={refetch}
          >
            {accessCopy.retry}
          </Button>
        ) : action ? (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            aria-label={action.name}
            onClick={openAccess}
          >
            {action.label}
          </Button>
        ) : undefined
      }
    >
      {text}
    </Note>
  );
}
