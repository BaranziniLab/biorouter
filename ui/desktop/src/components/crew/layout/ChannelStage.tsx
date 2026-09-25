import { useCallback, useId, useMemo } from 'react';
import { AccessTab, agentAccessCount, ChatAccessPane, useWorkspaceGrants } from '../access';
import { ChannelHeader, ConnectionBar } from '../channel';
import { Composer } from '../composer/Composer';
import type { CrewMessage } from '../crewApi';
import { uniqueNamesSupported } from '../dialogs';
import { AttachmentIndexProvider } from '../files/attachmentIndex';
import { CrewFileDropZone } from '../files/FileDropZone';
import { FilesTab } from '../files/FilesTab';
import { AgentTaskPane, DetailsPane } from '../pane';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewSessionGrant } from '../api/grants';
import type { CrewController, VerifiedView } from '../state/types';
import {
  Timeline,
  type AttachmentSlotState,
  type OwnAgentChat,
  type TimelineView,
} from '../timeline';
import { useComposerNote } from './ComposerNote';
import { MessageFiles } from './MessageFiles';
import type { TaskHighlight } from './useTaskHighlight';

/**
 * The timeline's view of the last verified copy (presentation only). Null when that copy is of
 * another channel, or its channel is gone.
 */
export function lastVerifiedTimeline(
  view: VerifiedView | null,
  channelId: string
): TimelineView | null {
  if (!view || view.channelId !== channelId) return null;
  const channel = view.snapshot.channels.find((item) => item.id === channelId);
  if (!channel) return null;
  return {
    snapshot: view.snapshot,
    channel,
    messages: view.messages,
    messagesLoaded: true,
    runs: view.runs,
    labels: view.labels,
    historyBefore: null,
    people: view.people ?? null,
  };
}

/** Who else can post in the selected channel, kept steady while Crew re-verifies. */
function useChannelAgentAccess(crew: CrewController, grants: CrewSessionGrant[]) {
  const runs = crew.snapshot ? crew.runs : (crew.lastVerified?.runs ?? crew.runs);
  return useMemo(
    () => agentAccessCount({ grants, runs, channelId: crew.channelId }),
    [grants, runs, crew.channelId]
  );
}

/**
 * The viewer's own chats with Crew access, by the run each posts as, so the timeline can head
 * their posts "Your agent · {chat title}" (Q3-22). Built only from this device's own grant list
 * (`useWorkspaceGrants`): another person's chat is never in it, so their agent's posts keep
 * reading "{name}'s agent". A grant the daemon lists without a title adds nothing.
 */
export function ownAgentChatsFrom(
  grants: readonly CrewSessionGrant[],
  connectionId: string
): ReadonlyMap<string, OwnAgentChat> {
  const chats = new Map<string, OwnAgentChat>();
  for (const grant of grants) {
    const title = typeof grant.session_name === 'string' ? grant.session_name.trim() : '';
    if (!title || grant.connection_id !== connectionId) continue;
    chats.set(grant.run_id, { title, sessionId: grant.session_id });
  }
  return chats;
}

/**
 * One channel (ui-redesign-spec, "Layout" and the wireframes): the 44px channel band, the connection
 * bar, then the channel body — the timeline and the composer in the 760px chat column — and the one
 * non-modal details pane beside it (push) or over the body below the band and the bar (cover). The
 * container query in `crew-app.css` decides push or cover; nothing here measures.
 *
 * The connection bar is its own row, outside `.crew-channel-body`, and that placement is the point:
 * it is where an error lands when the surface that caused it is not on screen (the sidebar, a menu,
 * the observer, Stop in the pane's Access tab), and a covering pane hides the body it covers. Inside
 * the body, those errors were rendered once and seen by no one while the pane was open.
 *
 * While a refresh re-verifies, everything draws from the controller's last verified view: the
 * header and the sidebar keep their names, the timeline shows the last messages dimmed and inert,
 * and the composer is replaced by its same-height "Verifying access…" bar. Nothing acts on that
 * copy, and an observation failure drops it (the controller then leaves this screen).
 *
 * Slots are wired here and nowhere else: the Access area's chat note, pane and tab, the files
 * area's tab and attachment rows (Tab stops only on the active row, Q3-05), the pane's Ask my
 * agent with its "Show task in channel" (and the id the composer's toggle names in its
 * `aria-controls`, Q3-23), the header's agent-access count, and the viewer's own chats with
 * access, which head their agents' posts (Q3-22). The attachment index the files area keeps for
 * the channel (same-named files, a file already shared, Q3-13) is provided here around all of it.
 *
 * The whole body — timeline and composer — is one file drop zone. It has no target of its own: the
 * composer registers its upload with it (`useCrewDropTarget`), so a file dropped on the messages
 * goes where one dropped on the composer does, through the native share confirmation, and the
 * zone takes nothing while the composer cannot (verifying, archived).
 *
 * While a Crew dialog or the sign-in dialog is open, the timeline's slot is `inert` and
 * `aria-hidden`: the messages and their buttons are behind the dialog, and a screen reader's
 * cursor must not wander out of it into them (Q2-13). A dialog's own focus trap and `hideOthers`
 * cover the rest of the page; this is the belt to that brace, because `hideOthers` keeps any
 * element with an `aria-live` attribute, which the log carries while it opens.
 */
export function ChannelStage({ highlight }: { highlight: TaskHighlight }) {
  const crew = useCrew();
  const titleId = useId();
  const verifying = !crew.snapshot;
  const behindDialog = crew.ui.dialog !== null || crew.signIn.open;
  const lastView = verifying ? lastVerifiedTimeline(crew.lastVerified, crew.channelId) : null;
  const { grants } = useWorkspaceGrants();
  const access = useChannelAgentAccess(crew, grants);
  const agentPaneId = useId();
  const canRename = uniqueNamesSupported(
    crew.snapshot ?? crew.lastVerified?.snapshot ?? null,
    crew.capabilities
  );
  const note = useComposerNote();
  const { connectionId } = crew;
  const renderAttachments = useCallback(
    (message: CrewMessage, slot: AttachmentSlotState) => (
      <MessageFiles connectionId={connectionId} message={message} active={slot.active} />
    ),
    [connectionId]
  );
  const ownAgentChats = useMemo(
    () => ownAgentChatsFrom(grants, connectionId),
    [grants, connectionId]
  );

  // One attachment index for the channel view: the timeline's cards, the Files tab's and the
  // composer's same-file note all read the same one (Q3-13).
  return (
    <AttachmentIndexProvider>
      <div className="crew-stage">
        <section className="crew-channel" aria-labelledby={titleId}>
          <ChannelHeader
            agentAccess={{ chats: access.chats, tasks: access.tasks }}
            canRename={canRename}
            titleId={titleId}
          />
          <ConnectionBar className="crew-channel-bar" />
          <div className="crew-channel-body">
            <CrewFileDropZone className="crew-frame-drop">
              <div
                className="crew-frame-timeline"
                inert={verifying || behindDialog}
                aria-hidden={behindDialog ? true : undefined}
                data-verifying={verifying ? 'true' : undefined}
              >
                <Timeline
                  view={lastView}
                  readOnly={verifying}
                  renderAttachments={renderAttachments}
                  ownAgentChats={ownAgentChats}
                  highlightRunId={highlight.runId}
                  onHighlightDone={highlight.done}
                />
              </div>
              <Composer note={note} agentPaneId={agentPaneId} />
            </CrewFileDropZone>
          </div>
        </section>
        <DetailsPane
          tabs={{ files: <FilesTab />, access: <AccessTab /> }}
          chatAccess={<ChatAccessPane />}
          agent={
            <div id={agentPaneId} className="crew-frame-agent-pane">
              <AgentTaskPane onShowTask={highlight.show} />
            </div>
          }
        />
      </div>
    </AttachmentIndexProvider>
  );
}
