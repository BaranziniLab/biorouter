import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation } from 'react-router-dom';
import { sessionGrantState } from '../api/grants';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { channelName, connectionNames, identityCopy } from '../identity';
import { useCrew, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { channelLabels, chatTitleOf } from './accessRows';
import { useKnownChatTitle } from './ChatAccessPane';
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
//
// Live QA round 3:
// - Q3-08: "Connect in Crew" from an offline chat arrives before any verified view exists (the
//   connection is down). The intent waits for the first verified view of the GRANT'S connection —
//   moving to that connection first when Crew shows another — and only then moves to the grant's
//   channel, so a reconnect lands on the chat's channel rather than the last one visited.
// - Q3-28: `/crew` in a chat with no grant used to stop at "Connect this chat to #general?
//   [Review access]", although typing `/crew` already said what the person wanted. That arrival
//   carries no intent of its own; its history entry stands in for one, and it opens the consent at
//   once — only when the chat has no grant. Allow is still the consent.

const CHAT_ACCESS_INTENT = 'crewOpenChatAccess';
/**
 * Route state beside the intent: the saved connection "Connect in Crew" asks Crew to connect on
 * arrival (Q3-08). The controller reads it; the click that put it here is the person's.
 */
export const CREW_CONNECT_ROUTE_STATE = 'crewConnect';

/** Intents already honoured, so Back to this entry, or the note remounting, never reopens it. */
const consumedIntents = new Set<string>();
/** Intents whose move to the grant's channel was already asked for: it is asked once. */
const movedIntents = new Set<string>();
/** Intents whose move to the grant's connection was already asked for: it is asked once. */
const movedConnectionIntents = new Set<string>();
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

/**
 * The intent a `/crew` arrival stands for: one per history entry and chat, so Back to the entry, or
 * the note remounting, never reopens the pane (Q3-28).
 */
function commandIntentId(locationKey: string, sessionId: string): string {
  return `crew-command\n${locationKey}\n${sessionId}`;
}

/** For tests: forget every intent this window honoured, so each test arrives afresh. */
export function forgetChatAccessIntents(): void {
  consumedIntents.clear();
  movedIntents.clear();
  movedConnectionIntents.clear();
  openAttempts.clear();
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
 * Route state for "Connect in Crew" (Q3-08): the Chat access intent, plus the saved connection
 * Crew connects on arrival, as the person's own connect.
 */
export function chatConnectRouteState(connectionId: string): Record<string, string> {
  return { ...chatAccessRouteState(), [CREW_CONNECT_ROUTE_STATE]: connectionId };
}

/**
 * What {@link chatConnectRouteState} asked of Crew, read back from route state: the connection to
 * connect and the intent it rides with (so it is honoured once per intent), or `null`.
 */
export function crewConnectRequestOf(
  state: unknown
): { intentId: string; connectionId: string } | null {
  const intentId = intentIdOf(state);
  if (!intentId || !state || typeof state !== 'object') return null;
  const connectionId = (state as Record<string, unknown>)[CREW_CONNECT_ROUTE_STATE];
  return typeof connectionId === 'string' && connectionId ? { intentId, connectionId } : null;
}

/**
 * The note above the Crew composer when a chat sent the person here with `/crew`
 * (`?sessionId=`): what this chat can do in Crew right now, and the one control that changes it
 * (ui-redesign-spec, "Revoke", the chat-connect note table). "“Plot review”" stands for the chat's
 * title when this window knows it, else "This chat" (Q3-30).
 *
 * | Grant state | Note | Action |
 * |---|---|---|
 * | Loading | Checking this chat's access… | — |
 * | None | Connect “Plot review” to #methods? | Review access |
 * | Active here | “Plot review” can read and post in #methods. | Manage access |
 * | Active elsewhere | “Plot review” already uses #raw-data. | Manage access |
 * | Revoked / Expired | Crew access for “Plot review” was revoked. / …expired. | Grant again |
 *
 * Every action opens the Chat access pane; none of them grants or revokes by itself. Arriving with
 * {@link chatAccessRouteState} presses that action once, as soon as there is one; arriving by
 * `/crew` presses it only for a chat with no grant (Q3-28). Renders nothing without `?sessionId=`,
 * nor while the Chat access pane shows this chat (Q4-14).
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
    selectConnection,
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
  const explicitIntent = intentIdOf(location.state);
  // `/crew` carries no intent: the history entry it made stands in for one (Q3-28).
  const commandIntent =
    explicitIntent || !grantSessionId ? null : commandIntentId(location.key, grantSessionId);
  const intentId = explicitIntent ?? commandIntent;

  const grant = grantSessionId
    ? (grants.grants.find((item) => item.session_id === grantSessionId) ?? null)
    : null;
  const cachedTitle = useKnownChatTitle(grantSessionId);
  const chat = (grant ? chatTitleOf(grant) : null) ?? cachedTitle;
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
      text = accessCopy.noteNone(chat, here);
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
      text = accessCopy.noteRevoked(chat);
      action = { label: accessCopy.noteGrantAgain };
    } else if (state === 'expired') {
      text = accessCopy.noteExpired(chat);
      action = { label: accessCopy.noteGrantAgain };
    } else {
      const sameConnection = grant.connection_id === connectionId;
      if (sameConnection && channel && grant.channel_id === channel.id) {
        text = accessCopy.noteActive(chat, here ?? channelName(channel));
      } else if (sameConnection) {
        text = accessCopy.noteActiveElsewhere(
          chat,
          labels.get(grant.channel_id) ?? accessCopy.unknownChannel
        );
      } else {
        text = accessCopy.noteActiveOtherWorkspace(
          chat,
          workspaces.get(grant.connection_id) ?? identityCopy.unnamedWorkspace
        );
      }
      action = { label: accessCopy.noteManage };
    }
  }

  // Not while the grant state is still being read, nor when the read failed: the one hop opens the
  // pane the note's own button would, and there is no button until the note knows which.
  const actionable = action !== null;
  // What `/crew` opens by itself: the consent of a chat that has no grant at all.
  const offersConsent = actionable && !grant;
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  const paneOpen =
    ui.pane?.mode === 'chat-access' && (ui.pane.sessionId ?? grantSessionId) === grantSessionId;
  // The saved connection the chat's grant is on, when Crew shows another one (Q3-08).
  const grantConnectionElsewhere =
    grant &&
    grant.connection_id !== connectionId &&
    connections.some((item) => item.id === grant.connection_id)
      ? grant.connection_id
      : null;
  // The channel the chat's grant is on, when this view can show it: the grant's own connection, and
  // a channel of the verified snapshot that is not archived.
  const grantChannel =
    grant &&
    grant.connection_id === connectionId &&
    snapshot?.channels.some((item) => item.id === grant.channel_id && !item.archived)
      ? grant.channel_id
      : null;
  const channelNow = channel?.id ?? null;
  // Read through refs: the controller's `selectChannel` and `selectConnection` are new functions
  // every render.
  const select = useRef(selectChannel);
  select.current = selectChannel;
  const selectWorkspace = useRef(selectConnection);
  selectWorkspace.current = selectConnection;
  // The channel the previous pass saw. A pass whose channel moved may still be followed, in the same
  // update, by the reset that closes the pane, so it never spends the intent.
  const lastChannel = useRef<string | null | undefined>(undefined);
  const [recheck, setRecheck] = useState(0);
  useEffect(() => {
    const settled = lastChannel.current === channelNow;
    lastChannel.current = channelNow;
    if (!intentId || !grantSessionId || !actionable || consumedIntents.has(intentId)) return;
    // `/crew` answers a chat that has no grant with its consent, and nothing else: a chat that has
    // one keeps the note and its button. Decided once, on the first answer the note can act on.
    if (!explicitIntent && !offersConsent) {
      consumedIntents.add(intentId);
      return;
    }
    // The grant's own connection first (Q3-08), asked once: after that, a view that stays on
    // another connection is the person's choice, and the pane opens where they are.
    if (grantConnectionElsewhere && !movedConnectionIntents.has(intentId)) {
      movedConnectionIntents.add(intentId);
      selectWorkspace.current(grantConnectionElsewhere);
      return;
    }
    // Nothing moves and nothing opens before a verified view of that connection exists: an
    // arrival from an offline chat waits here until Crew is connected and has verified it.
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
    explicitIntent,
    grantSessionId,
    actionable,
    offersConsent,
    verified,
    grantConnectionElsewhere,
    grantChannel,
    channelNow,
    paneOpen,
    recheck,
    openPane,
  ]);

  // The pane open for this chat already asks, or says, everything the note would, one control
  // away: the note under the timeline made two prompts for one decision (live QA round 4, Q4-14).
  // The pane is always for the channel shown (a channel switch closes it), so this is the same
  // chat and channel. The intent above still runs: it is what keeps the pane open.
  if (!grantSessionId || text === null || paneOpen) return null;

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
