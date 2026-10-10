import React from 'react';
import { type SharedSessionDetails } from '../../sharedSessions';
import { SessionMessages } from './SessionViewComponents';
import { formatMessageTimestamp } from '../../utils/timeUtils';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { PageHeader } from '../Layout/PageHeader';
import ArtifactViewer from '../artifacts/ArtifactViewer';
import { useArtifactPanel } from '../artifacts/useArtifactPanel';
import { useIsMobile } from '../../hooks/use-mobile';
import { ReadableContent } from '../Layout/ReadableContent';
import { SHARED_CHAT, SHARED_CHAT_INFO, billedTokens, messageCount, ranIn } from './copy';

interface SharedSessionViewProps {
  session: SharedSessionDetails | null;
  isLoading: boolean;
  error: string | null;
  onRetry: () => void;
}

/**
 * A chat someone shared, read-only. It wears the same 44px band as every other
 * page (spec 3.4): the chat's title, what this page is in an InfoTip, and the
 * facts beside the title in muted tabular figures. The band replaced two
 * stacked headers (a "Shared chat" strip and a 24px title over a two-line mono
 * metadata block).
 */
const SharedSessionView: React.FC<SharedSessionViewProps> = ({
  session,
  isLoading,
  error,
  onRetry,
}) => {
  // The same panel the live chat and the saved-session page mount. A shared
  // transcript is the one that leaves the machine, and it gets the identical
  // figure surface.
  const artifactPanel = useArtifactPanel({ isMobile: useIsMobile(), allowWindowResize: false });
  const { splitPaneRef, artifact: presentedArtifact, openArtifact } = artifactPanel;

  const loaded = !isLoading && session !== null;
  const facts = loaded
    ? [
        SHARED_CHAT,
        session.messages[0]?.created ? formatMessageTimestamp(session.messages[0].created) : null,
        messageCount(session.message_count),
        session.total_tokens !== null ? billedTokens(session.total_tokens.toLocaleString()) : null,
      ].filter(Boolean)
    : [SHARED_CHAT];
  const info =
    loaded && session.working_dir
      ? `${SHARED_CHAT_INFO} ${ranIn(session.working_dir)}`
      : SHARED_CHAT_INFO;

  return (
    <MainPanelLayout removeTopPadding>
      {/* The live chat's split: a column holding the band and the transcript,
          and the panel beside it. The column and body flatten in a preview
          layout rather than re-parent, so the panel is never remounted; rung 2
          measures this box and places the pieces by `data-preview-area`. */}
      <div
        ref={splitPaneRef}
        {...artifactPanel.splitPaneProps}
        className="relative flex flex-1 min-h-0 min-w-0"
      >
        <div data-preview-area="column" className="flex min-w-0 flex-1 flex-col">
          <div data-preview-area="header" className="flex-shrink-0">
            <PageHeader
              title={session?.description || SHARED_CHAT}
              info={info}
              adornment={facts.join(' · ')}
            />
          </div>
          <div data-preview-area="body" className="flex min-h-0 flex-1 flex-col">
            {/* A real reading column on the CHAT measure, like its local twin
                `SessionHistoryView` and the live chat, so the three cannot
                drift. It is part of the shell, not of the loaded chat, so the
                content does not jump from pane-wide to columned as it loads. */}
            <div data-preview-area="transcript" className="flex min-h-0 flex-1 flex-col">
              <ReadableContent size="chat" className="flex min-h-0 flex-1 flex-col px-6">
                <SessionMessages
                  messages={session?.messages || []}
                  sessionId={session ? `shared:${session.share_token}` : 'shared:loading'}
                  isLoading={isLoading}
                  error={error}
                  onRetry={onRetry}
                  onOpenArtifact={openArtifact}
                  workingDir={session?.working_dir}
                />
              </ReadableContent>
            </div>
          </div>
        </div>

        {/* Read-only: no `onRenderError`, so no repair listener, and nothing
            auto-opens. */}
        {presentedArtifact && <ArtifactViewer {...artifactPanel.viewerProps} />}
      </div>
    </MainPanelLayout>
  );
};

export default SharedSessionView;
