import React, { useState, useEffect } from 'react';
import {
  MessageSquareText,
  Share2,
  Copy,
  Check,
  AlertCircle,
  Globe,
  Play,
} from '../icons/app-icons';
import { resumeSession } from '../../sessions';
import { Button } from '../ui/button';
import { Spinner } from '../ui/spinner';
import { toastError } from '../../toasts';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import { ScrollArea } from '../ui/scroll-area';
import { formatMessageTimestamp } from '../../utils/timeUtils';
import { createSharedSession } from '../../sharedSessions';
import { billedSessionTokenEstimate, formatBilledTokenEstimate } from '../../utils/billedTokens';
import ProgressiveMessageList from '../ProgressiveMessageList';
import ArtifactViewer from '../artifacts/ArtifactViewer';
import { useArtifactPanel } from '../artifacts/useArtifactPanel';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import { useIsMobile } from '../../hooks/use-mobile';
import { SearchView } from '../conversation/SearchView';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { exportSession, Message, Session } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { PrivacyBadge } from '../ui/PrivacyBadge';
import { DeclassifySessionDialog } from './DeclassifySessionDialog';
import { DECLASSIFY_NEEDS_HOST_SHORT, declassifyBrowserReason } from './declassifyOnBrowser';
import { subscribeSessionRowChanges } from '../../utils/sessionRowSync';
import { useNavigation } from '../../hooks/useNavigation';
import { ReadableContent } from '../Layout/ReadableContent';
import { ModalShell } from '../ModalShell';
import { EmptyState } from '../ui/empty-state';
import { useTransientFlag } from '../../hooks/useTransientFlag';
import {
  BACK,
  CHAT_LOAD_ERROR_TITLE,
  COPY_LINK,
  COPY_LINK_FAILED,
  COPY_LINK_FAILED_MSG,
  DONE,
  LINK_COPIED,
  LOADING_CHAT,
  MAKE_CHAT_PUBLIC,
  NO_MESSAGES,
  NO_MESSAGES_TITLE,
  RESUME,
  RESUME_FAILED,
  SHARE,
  SHARE_DIALOG_SUBTITLE,
  SHARE_DIALOG_TITLE,
  SHARE_FAILED,
  SHARE_NOT_SET_UP,
  SHARE_READ_FAILED,
  TRY_AGAIN,
  billedTokens,
  messageCount,
  ranIn,
} from './copy';
import './history.css';

const isUserMessage = (message: Message): boolean => {
  if (message.role === 'assistant') {
    return false;
  }
  // ⚠ Third spelling of the same dead variant. Nothing in `crates/` constructs
  // `toolConfirmationRequest`; a real card is an `actionRequired` whose
  // `actionType` is `toolConfirmation`. Leaving one copy of the dead name in
  // the tree is how the next divergence gets written.
  return !message.content.every(
    (c) => c.type === 'actionRequired' && c.data.actionType === 'toolConfirmation'
  );
};

const filterMessagesForDisplay = (messages: Message[]): Message[] => {
  return messages;
};

/** Why a chat could not be shared: the daemon's own sentence when it refused, else one of ours. */
function shareRefusalMessage(refusal: unknown): string {
  if (typeof refusal === 'string' && refusal.trim()) return refusal.trim();
  if (refusal instanceof Error && refusal.message.trim()) return refusal.message.trim();
  return SHARE_READ_FAILED;
}

/**
 * The transcript to share, read through the daemon's export door rather than taken from this page.
 * Sharing posts the transcript to another server, so it passes the rule an export does: a chat a
 * Crew grant restricts is refused there, because its channel context must not leave with it
 * (CROSSCUT-2), and the refusal's sentence is thrown for the person to read.
 */
async function transcriptToShare(sessionId: string): Promise<Message[]> {
  let exported: string;
  try {
    const response = await exportSession({
      path: { session_id: sessionId },
      headers: await userActionHeaders(),
      throwOnError: true,
    });
    exported = response.data;
  } catch (refusal) {
    throw new Error(shareRefusalMessage(refusal));
  }
  let conversation: unknown;
  try {
    conversation = (JSON.parse(exported) as { conversation?: unknown }).conversation;
  } catch {
    conversation = undefined;
  }
  if (!Array.isArray(conversation)) throw new Error(shareRefusalMessage(undefined));
  return conversation as Message[];
}

interface SessionHistoryViewProps {
  session: Session;
  isLoading: boolean;
  error: string | null;
  onBack: () => void;
  onRetry: () => void;
  showActionButtons?: boolean;
}

const SessionMessages: React.FC<{
  messages: Message[];
  isLoading: boolean;
  error: string | null;
  onRetry: () => void;
  sessionId: string;
  workingDir?: string;
  onOpenArtifact: (artifact: ArtifactSource) => void;
}> = ({ messages, isLoading, error, onRetry, sessionId, workingDir, onOpenArtifact }) => {
  const filteredMessages = filterMessagesForDisplay(messages);

  return (
    // `data-preview-transcript`: the box rung 2 measures as this replay's
    // transcript, so the band above it counts as the conversation's chrome.
    <ScrollArea className="h-full w-full" data-preview-transcript="">
      <div className="pb-24 pt-6">
        {isLoading ? (
          <div className="flex justify-center items-center py-12">
            <Spinner label={LOADING_CHAT} />
          </div>
        ) : error ? (
          // §4.5: the same shared surface the list view's error uses.
          <EmptyState
            icon={AlertCircle}
            title={CHAT_LOAD_ERROR_TITLE}
            description={error}
            actions={
              <Button onClick={onRetry} variant="secondary">
                {TRY_AGAIN}
              </Button>
            }
          />
        ) : filteredMessages?.length > 0 ? (
          /* ⚠ NO measure of its own. This was `max-w-4xl mx-auto w-full` (the
             896px replay column) nested inside the page's reading column, so the
             transcript had two ceilings and neither was the live chat's. The
             outer `ReadableContent size="chat"` is the measure (design of record
             §4.4); a second `max-w-*` here would silently win again, and
             `styles/measures.test.ts` fails on one. Find in this transcript is
             SearchView, the same overlay the live chat uses. */
          <SearchView>
            <ProgressiveMessageList
              messages={filteredMessages}
              // The REAL session id: every consumer that scopes work by id (the
              // scroll broadcast, Branch) must address this chat.
              chat={{ sessionId }}
              toolCallNotifications={new Map()}
              isUserMessage={isUserMessage} // Use the same function as BaseChat
              onOpenArtifact={onOpenArtifact}
              // No terminal on this surface: a saved transcript is a record, and
              // a shell code block in it is history, not an offer. Null rather
              // than omitted, so the absence is a decision.
              onRunInTerminal={null}
              workingDir={workingDir}
              batchSize={15} // Same as BaseChat default
              batchDelay={30} // Same as BaseChat default
              showLoadingThreshold={30} // Same as BaseChat default
            />
          </SearchView>
        ) : (
          <EmptyState
            icon={MessageSquareText}
            title={NO_MESSAGES_TITLE}
            description={NO_MESSAGES}
            compact
          />
        )}
      </div>
    </ScrollArea>
  );
};

/** A spinner in the 16px glyph slot of a band action, while that action runs. */
const BusyGlyph = () => <Spinner size={16} />;

const SessionHistoryView: React.FC<SessionHistoryViewProps> = ({
  session,
  isLoading,
  error,
  onBack,
  onRetry,
  showActionButtons = true,
}) => {
  const [isShareModalOpen, setIsShareModalOpen] = useState(false);
  const [shareLink, setShareLink] = useState<string>('');
  const [isSharing, setIsSharing] = useState(false);
  const [isCopied, markCopied] = useTransientFlag(2000);
  const [canShare, setCanShare] = useState(false);
  // Issue #56 §12.1's second entry point. The session page is where a user goes
  // to answer "what is in this chat?", so it is the other place the answer "no
  // longer anything private" belongs. Same dialog as History's row menu, so the
  // two cannot come to ask for different confirmations.
  const [declassifyOpen, setDeclassifyOpen] = useState(false);
  /**
   * SD-8 — is this page served to a browser, where declassification cannot work
   * at all? See `declassifyOnBrowser.ts`. Read from the DOM marker rather than
   * held as state, for the reason `ModelsBottomBar` gives: the surface cannot
   * change while the renderer runs.
   */
  const declassifyOnHost = declassifyBrowserReason();
  const [tier, setTier] = useState(session.privacy_tier);
  // The same panel the live chat mounts, from the same hook — a saved figure is
  // displayed exactly as a fresh one is, and there is no second renderer here.
  // `allowWindowResize` is false: opening a page must never resize the user's
  // window, and unlike a chat this surface is not somewhere they are working.
  const artifactPanel = useArtifactPanel({ isMobile: useIsMobile(), allowWindowResize: false });
  const { splitPaneRef, artifact: presentedArtifact, openArtifact } = artifactPanel;
  useEffect(() => setTier(session.privacy_tier), [session.privacy_tier]);
  // …and follow a declassification made anywhere else — another window's
  // History row, or this chat's own page open twice. The `session` prop is read
  // once when the page opens, so without this the badge above a chat that is
  // no longer private would stay private until the page was reopened.
  useEffect(
    () =>
      subscribeSessionRowChanges(({ sessionId, privacy_tier }) => {
        if (sessionId === session.id) setTier(privacy_tier);
      }),
    [session.id]
  );

  const messages = session.conversation || [];
  const billedTokenEstimate = billedSessionTokenEstimate(session);

  const setView = useNavigation();

  useEffect(() => {
    const savedSessionConfig = localStorage.getItem('session_sharing_config');
    if (savedSessionConfig) {
      try {
        const config = JSON.parse(savedSessionConfig);
        if (config.enabled && config.baseUrl) {
          setCanShare(true);
        }
      } catch (error) {
        console.error('Error parsing session sharing config:', error);
      }
    }
  }, []);

  const handleShare = async () => {
    setIsSharing(true);

    try {
      const savedSessionConfig = localStorage.getItem('session_sharing_config');
      if (!savedSessionConfig) {
        throw new Error('Chat sharing is not configured.');
      }

      const config = JSON.parse(savedSessionConfig);
      if (!config.enabled || !config.baseUrl) {
        throw new Error('Chat sharing is off, or its base URL is missing.');
      }

      const shareToken = await createSharedSession(
        config.baseUrl,
        session.working_dir,
        await transcriptToShare(session.id),
        session.name || 'Shared chat',
        billedTokenEstimate?.lowerBound ? null : (billedTokenEstimate?.value ?? null)
      );

      const shareableLink = `biorouter://sessions/${shareToken}`;
      setShareLink(shareableLink);
      setIsShareModalOpen(true);
    } catch (error) {
      console.error('Error sharing session:', error);
      toastError({
        title: SHARE_FAILED,
        msg: error instanceof Error ? error.message : SHARE_READ_FAILED,
      });
    } finally {
      setIsSharing(false);
    }
  };

  const handleCopyLink = () => {
    navigator.clipboard
      .writeText(shareLink)
      .then(() => {
        markCopied();
      })
      .catch((err) => {
        console.error('Failed to copy link:', err);
        toastError({ title: COPY_LINK_FAILED, msg: COPY_LINK_FAILED_MSG });
      });
  };

  const handleResumeSession = () => {
    try {
      resumeSession(session, setView);
    } catch (error) {
      toastError({
        title: RESUME_FAILED,
        msg: error instanceof Error ? error.message : String(error),
      });
    }
  };

  /*
   * The band's actions: ghost round 32px icons with tooltips, Resume the one
   * `secondary` among them (spec 3.4). Share stays focusable while it is
   * unavailable (`aria-disabled`, not `disabled`), because `buttonVariants`
   * gives a disabled button `pointer-events: none` and the tooltip that says
   * WHY sharing is off could then never open.
   */
  const actions = showActionButtons ? (
    <>
      <PageHeaderAction
        icon={isSharing ? BusyGlyph : Share2}
        label={SHARE}
        tooltip={canShare ? SHARE : SHARE_NOT_SET_UP}
        aria-disabled={!canShare || isSharing ? true : undefined}
        aria-busy={isSharing || undefined}
        className="br-transcript-action"
        onClick={() => {
          if (canShare && !isSharing) void handleShare();
        }}
      />
      {tier === 'private' &&
        (declassifyOnHost !== null ? (
          /* SD-8's second entry point: a line of text, not a disabled control,
             because this surface can never declassify (the browser has no proof
             of a person). The short line stays visible; the full reason is on
             `title`, where the global tooltip enhancer also makes it the span's
             spoken name. A refusal is never hover-only. */
          <span
            data-testid="declassify-browser-note"
            title={declassifyOnHost}
            className="whitespace-nowrap px-1 text-supporting text-text-muted"
          >
            {DECLASSIFY_NEEDS_HOST_SHORT}
          </span>
        ) : (
          <PageHeaderAction
            icon={Globe}
            label={MAKE_CHAT_PUBLIC}
            onClick={() => setDeclassifyOpen(true)}
          />
        ))}
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="secondary"
            shape="round"
            aria-label={RESUME}
            className="no-drag"
            onClick={handleResumeSession}
          >
            <Play aria-hidden="true" />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">{RESUME}</TooltipContent>
      </Tooltip>
    </>
  ) : null;

  // The facts beside the title, once the chat has loaded: when it started, how
  // long it is, what it cost. Sans and tabular, like every count in the app.
  const facts = !isLoading
    ? [
        messages[0]?.created ? formatMessageTimestamp(messages[0].created) : null,
        messageCount(session.message_count),
        billedTokenEstimate ? billedTokens(formatBilledTokenEstimate(billedTokenEstimate)) : null,
      ].filter(Boolean)
    : [];

  return (
    <>
      <MainPanelLayout removeTopPadding>
        {/* The horizontal split the artifact panel needs, built like the live
            chat's: a column holding the band and the transcript, the panel
            beside it. In the side layout the panel's own 44px strip continues
            this band, and in the stacked one the sheet sits between the band
            and the transcript. `splitPaneRef` goes here because rung 2 measures
            this box; the `data-preview-area` marks are how the grid places the
            pieces (`main.css`, RUNG 2), and the column and body flatten rather
            than re-parent, so the panel is never remounted. */}
        <div
          ref={splitPaneRef}
          {...artifactPanel.splitPaneProps}
          className="relative flex flex-1 min-h-0 min-w-0"
        >
          <div data-preview-area="column" className="flex min-w-0 flex-1 flex-col">
            {/* The privacy marker is in the band from the first frame, not in a
                line that renders only once the chat has loaded: a marker that
                arrives a beat late can be missed by someone reading fast, which
                is the failure R10 exists to prevent. It reads the LOCAL tier, so
                a declassification made from this band clears it at once. */}
            <div data-preview-area="header" className="flex-shrink-0">
              <PageHeader
                title={session.name}
                onBack={onBack}
                backLabel={BACK}
                info={session.working_dir ? ranIn(session.working_dir) : undefined}
                adornment={
                  tier || facts.length > 0 ? (
                    <span className="flex min-w-0 items-center gap-2">
                      {tier ? <PrivacyBadge tier={tier} /> : null}
                      {facts.length > 0 ? (
                        <span className="truncate">{facts.join(' · ')}</span>
                      ) : null}
                    </span>
                  ) : undefined
                }
                actions={!isLoading ? actions : undefined}
              />
            </div>
            <div data-preview-area="body" className="flex min-h-0 flex-1 flex-col">
              {/* ⚠ `size="chat"`, and it is the ONLY measure on this surface:
                  the same `--measure-chat` the live chat's composer reads. */}
              <div data-preview-area="transcript" className="flex min-h-0 flex-1 flex-col">
                <ReadableContent size="chat" className="flex min-h-0 flex-1 flex-col px-6">
                  <SessionMessages
                    messages={messages}
                    isLoading={isLoading}
                    error={error}
                    onRetry={onRetry}
                    sessionId={session.id}
                    workingDir={session.working_dir}
                    onOpenArtifact={openArtifact}
                  />
                </ReadableContent>
              </div>
            </div>
          </div>

          {/* No `onRenderError`: a saved transcript has no live turn to hand a
              broken figure back to, so ArtifactViewer installs no repair
              listener. And nothing auto-opens here; the panel appears when the
              reader clicks a card, never because the page loaded. */}
          {presentedArtifact && <ArtifactViewer {...artifactPanel.viewerProps} />}
        </div>
      </MainPanelLayout>

      <ModalShell
        open={isShareModalOpen}
        onOpenChange={setIsShareModalOpen}
        size="md"
        title={SHARE_DIALOG_TITLE}
        subtitle={SHARE_DIALOG_SUBTITLE}
        footer={
          <Button variant="secondary" onClick={() => setIsShareModalOpen(false)}>
            {DONE}
          </Button>
        }
      >
        {/* The link is a machine string: mono, in a well, with one Copy action. */}
        <div className="flex items-center gap-2 rounded-element bg-background-well py-1 pl-3 pr-1">
          <code className="min-w-0 flex-1 truncate text-code text-text-default">{shareLink}</code>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                shape="round"
                aria-label={isCopied ? LINK_COPIED : COPY_LINK}
                onClick={handleCopyLink}
                disabled={isCopied}
              >
                {isCopied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
              </Button>
            </TooltipTrigger>
            <TooltipContent side="top">{isCopied ? LINK_COPIED : COPY_LINK}</TooltipContent>
          </Tooltip>
        </div>
      </ModalShell>

      {declassifyOpen && (
        <DeclassifySessionDialog
          session={session}
          onClose={() => setDeclassifyOpen(false)}
          onDeclassified={() => {
            setTier('public');
            setDeclassifyOpen(false);
          }}
        />
      )}
    </>
  );
};

export default SessionHistoryView;
