import { useCallback, useId, useMemo } from 'react';
import { AccessTab, agentAccessCount, ChatAccessPane, useWorkspaceGrants } from '../access';
import { ChannelHeader, ConnectionBar } from '../channel';
import { Composer } from '../composer/Composer';
import type { CrewMessage } from '../crewApi';
import { uniqueNamesSupported } from '../dialogs';
import { FilesTab } from '../files/FilesTab';
import { AgentTaskPane, DetailsPane } from '../pane';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController, VerifiedView } from '../state/types';
import { Timeline, type TimelineView } from '../timeline';
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
  };
}

/** Who else can post in the selected channel, kept steady while Crew re-verifies. */
function useChannelAgentAccess(crew: CrewController) {
  const grants = useWorkspaceGrants();
  const runs = crew.snapshot ? crew.runs : (crew.lastVerified?.runs ?? crew.runs);
  return useMemo(
    () => agentAccessCount({ grants: grants.grants, runs, channelId: crew.channelId }),
    [grants.grants, runs, crew.channelId]
  );
}

/**
 * One channel (ui-redesign-spec, "Layout" and the wireframes): the 44px channel band, then the
 * channel body — the connection bar, the timeline and the composer in the 760px chat column — and
 * the one non-modal details pane beside it (push) or over the body below the band (cover). The
 * container query in `crew-app.css` decides push or cover; nothing here measures.
 *
 * While a refresh re-verifies, everything draws from the controller's last verified view: the
 * header and the sidebar keep their names, the timeline shows the last messages dimmed and inert,
 * and the composer is replaced by its same-height "Verifying access…" bar. Nothing acts on that
 * copy, and an observation failure drops it (the controller then leaves this screen).
 *
 * Slots are wired here and nowhere else: the Access area's chat note, pane and tab, the files
 * area's tab and attachment rows, the pane's Ask my agent with its "Show task in channel", and the
 * header's agent-access count.
 */
export function ChannelStage({ highlight }: { highlight: TaskHighlight }) {
  const crew = useCrew();
  const titleId = useId();
  const verifying = !crew.snapshot;
  const lastView = verifying ? lastVerifiedTimeline(crew.lastVerified, crew.channelId) : null;
  const access = useChannelAgentAccess(crew);
  const canRename = uniqueNamesSupported(crew.snapshot ?? crew.lastVerified?.snapshot ?? null);
  const note = useComposerNote();
  const { connectionId } = crew;
  const renderAttachments = useCallback(
    (message: CrewMessage) => <MessageFiles connectionId={connectionId} message={message} />,
    [connectionId]
  );

  return (
    <div className="crew-stage">
      <section className="crew-channel" aria-labelledby={titleId}>
        <ChannelHeader
          agentAccess={{ chats: access.chats, tasks: access.tasks }}
          canRename={canRename}
          titleId={titleId}
        />
        <div className="crew-channel-body">
          <ConnectionBar />
          <div
            className="crew-frame-timeline"
            inert={verifying}
            data-verifying={verifying ? 'true' : undefined}
          >
            <Timeline
              view={lastView}
              readOnly={verifying}
              renderAttachments={renderAttachments}
              highlightRunId={highlight.runId}
              onHighlightDone={highlight.done}
            />
          </div>
          <Composer note={note} />
        </div>
      </section>
      <DetailsPane
        tabs={{ files: <FilesTab />, access: <AccessTab /> }}
        chatAccess={<ChatAccessPane />}
        agent={<AgentTaskPane onShowTask={highlight.show} />}
      />
    </div>
  );
}
