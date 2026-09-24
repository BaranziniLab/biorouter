import { useMemo } from 'react';
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
 * Every action opens the Chat access pane; none of them grants or revokes by itself. Renders nothing
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

  if (!grantSessionId) return null;

  const openAccess = () => openPane({ mode: 'chat-access', sessionId: grantSessionId });
  const grant = grants.grants.find((item) => item.session_id === grantSessionId) ?? null;
  const here = channel ? channelName(channel) : null;

  let text: string;
  let action: { label: string; name?: string } | null = null;
  let retry = false;
  if (connectionsState !== 'loaded' || (!grant && grants.status === 'loading')) {
    text = accessCopy.noteChecking;
  } else if (!grant && grants.anyFailed) {
    text = grants.error ?? accessCopy.listFailed;
    retry = true;
  } else if (!grant) {
    if (!here) return null;
    text = accessCopy.noteNone(here);
    action = { label: accessCopy.noteReview, name: accessCopy.noteReviewName };
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
