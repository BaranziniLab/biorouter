import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation } from 'react-router-dom';
import { sessionGrantState } from '../api/grants';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { channelName, connectionNames, identityCopy } from '../identity';
import { useCrew, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { channelLabels } from './accessRows';
import { accessCopy } from './copy';
import { isUnconfirmedRevocation, useCrewGrants } from './useCrewGrants';

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
//
// Live QA round 2 (Q2-10) found the pane still did not open, and Crew showing #general for a chat
// whose grant is on #methods. Two causes, both fixed here:
// - Crew showed its first channel, not the grant's. The note now moves to the grant's channel,
//   once per intent, before it opens the pane.
// - The intent was spent the moment the pane was asked to open, and a surface reset in the same
//   update (a channel settling, the connection arriving) closed it again. The intent is now spent
//   only once the pane has been seen open on a verified view that did not move in that update; a
//   reset before then opens it again.

const CHAT_ACCESS_INTENT = 'crewOpenChatAccess';

/** Intents already honoured, so Back to this entry, or the note remounting, never reopens it. */
const consumedIntents = new Set<string>();
/** Intents whose move to the grant's channel was already asked for: it is asked once. */
const movedIntents = new Set<string>();
/**
 * How often each intent asked for the pane. A reset in the same update can close it (the reason it
 * is asked again), but only a few times on any real arrival: past this, the intent is given up
 * rather than fought over with whatever keeps closing the pane.
 */
const openAttempts = new Map<string, number>();
const MAX_OPEN_ATTEMPTS = 4;

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
    observedPrivacy,
    ui,
    openPane,
    selectChannel,
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
    if (
      state !== 'active' &&
      grant.kind === 'task' &&
      !isUnconfirmedRevocation(grant.connection_id, grant.session_id)
    ) {
      // A task's access ends with the task (Q2-09): nothing to grant again from here.
      text = accessCopy.noteTaskFinished;
    } else if (state === 'revoked') {
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
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  const paneOpen =
    ui.pane?.mode === 'chat-access' && (ui.pane.sessionId ?? grantSessionId) === grantSessionId;
  // The channel the chat's grant is on, when this view can show it: the grant's own connection, and
  // a channel of the verified snapshot that is not archived.
  const grantChannel =
    grant &&
    grant.connection_id === connectionId &&
    snapshot?.channels.some((item) => item.id === grant.channel_id && !item.archived)
      ? grant.channel_id
      : null;
  const channelNow = channel?.id ?? null;
  // Read through a ref: the controller's `selectChannel` is a new function every render.
  const select = useRef(selectChannel);
  select.current = selectChannel;
  // The channel the previous pass saw. A pass whose channel moved may still be followed, in the same
  // update, by the reset that closes the pane, so it never spends the intent.
  const lastChannel = useRef<string | null | undefined>(undefined);
  const [recheck, setRecheck] = useState(0);
  useEffect(() => {
    const settled = lastChannel.current === channelNow;
    lastChannel.current = channelNow;
    if (!intentId || !grantSessionId || !actionable || consumedIntents.has(intentId)) return;
    if (!verified) return;
    if (grantChannel && channelNow !== grantChannel && !movedIntents.has(intentId)) {
      movedIntents.add(intentId);
      select.current(grantChannel);
      return;
    }
    if (!paneOpen) {
      const attempts = openAttempts.get(intentId) ?? 0;
      if (attempts >= MAX_OPEN_ATTEMPTS) {
        consumedIntents.add(intentId);
        return;
      }
      openAttempts.set(intentId, attempts + 1);
      openPane({ mode: 'chat-access', sessionId: grantSessionId });
      // Look again after this update: a reset batched with the open closes the pane without
      // anything this effect reads changing.
      setRecheck((value) => value + 1);
      return;
    }
    if (!settled) {
      setRecheck((value) => value + 1);
      return;
    }
    consumedIntents.add(intentId);
  }, [
    intentId,
    grantSessionId,
    actionable,
    verified,
    grantChannel,
    channelNow,
    paneOpen,
    recheck,
    openPane,
  ]);

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
