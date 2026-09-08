import React, { useState, useEffect } from 'react';
import {
  Calendar,
  MessageSquareText,
  Folder,
  Share2,
  Sparkles,
  Copy,
  Check,
  Target,
  LoaderCircle,
  AlertCircle,
} from '../icons/app-icons';
import { resumeSession } from '../../sessions';
import { Button } from '../ui/button';
import { toastError } from '../../toasts';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ScrollArea } from '../ui/scroll-area';
import { formatMessageTimestamp } from '../../utils/timeUtils';
import { createSharedSession } from '../../sharedSessions';
import { billedSessionTokenEstimate, formatBilledTokenEstimate } from '../../utils/billedTokens';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import ProgressiveMessageList from '../ProgressiveMessageList';
import ArtifactViewer from '../artifacts/ArtifactViewer';
import { useArtifactPanel } from '../artifacts/useArtifactPanel';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import { useIsMobile } from '../../hooks/use-mobile';
import { SearchView } from '../conversation/SearchView';
import BackButton from '../ui/BackButton';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { Message, Session } from '../../api';
import { PrivacyBadge } from '../ui/PrivacyBadge';
import { DeclassifySessionDialog } from './DeclassifySessionDialog';
import { useNavigation } from '../../hooks/useNavigation';
import { ReadableContent } from '../Layout/ReadableContent';
import { MODAL_SIZE } from '../ModalShell';
import { EmptyState } from '../ui/empty-state';

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

interface SessionHistoryViewProps {
  session: Session;
  isLoading: boolean;
  error: string | null;
  onBack: () => void;
  onRetry: () => void;
  showActionButtons?: boolean;
}

// Custom SessionHeader component similar to SessionListView style
const SessionHeader: React.FC<{
  onBack: () => void;
  children: React.ReactNode;
  title: string;
  /**
   * Sits beside the title, NOT in the metadata row below it.
   *
   * The metadata row renders only once the conversation has loaded; the title
   * renders immediately. A privacy marker that appears a beat after the page
   * does is a marker you can miss by reading fast, which is precisely the
   * failure R10 exists to prevent.
   */
  titleAdornment?: React.ReactNode;
  actionButtons?: React.ReactNode;
}> = ({ onBack, children, title, titleAdornment, actionButtons }) => {
  return (
    /* `-mx-6 … px-6` cancels the reading column's own inset so the hairline
       runs the full width of that column, then puts the inset back on the
       content. The pair must always match the `px-*` on the `ReadableContent`
       below — they were `8` while the column was on the page measure. */
    <div className="biorouter-page-header -mx-6 flex flex-col px-6 pb-8">
      <div className="flex items-center pt-0 mb-1">
        <BackButton onClick={onBack} />
      </div>
      {/* §4.2 — `text-title` carries the 24/600/-0.01em the three utilities
          beside it were spelling out by hand. */}
      <div className="flex min-w-0 items-center gap-2 mb-4 pt-6">
        <h1 className="text-title min-w-0 break-words">{title}</h1>
        {titleAdornment}
      </div>
      <div className="flex items-center">{children}</div>
      {actionButtons && <div className="flex items-center space-x-3 mt-4">{actionButtons}</div>}
    </div>
  );
};

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
    <ScrollArea className="h-full w-full">
      <div className="pb-24 pt-8">
        <div className="flex flex-col space-y-6">
          {isLoading ? (
            <div className="flex justify-center items-center py-12">
              <LoaderCircle className="animate-spin h-8 w-8 text-text-default" />
            </div>
          ) : error ? (
            // §4.5 — the same shared surface the list view's error uses, so the
            // two halves of one feature stop speaking different error dialects.
            <EmptyState
              icon={AlertCircle}
              title="Couldn't load this chat"
              description={error}
              actions={
                <Button onClick={onRetry} variant="outline">
                  Try again
                </Button>
              }
            />
          ) : filteredMessages?.length > 0 ? (
            /* ⚠ NO measure of its own. This was `max-w-4xl mx-auto w-full` —
               the 896px replay column — nested inside the page's own reading
               column, so the transcript had two ceilings and neither was the
               one the live chat uses. The outer `ReadableContent size="chat"`
               is the measure now (design of record §4.4, done 2026-09-07); a
               second `max-w-*` here would silently take precedence again and
               `styles/measures.test.ts` fails on one. */
            <SearchView placeholder="Search history...">
              <ProgressiveMessageList
                messages={filteredMessages}
                // The REAL session id. This was the string 'session-preview',
                // which is nobody's session: every consumer that scopes work
                // by id — the scroll broadcast, Branch, an MCP app card —
                // silently addressed a chat that does not exist.
                chat={{ sessionId }}
                toolCallNotifications={new Map()}
                // No `append`. It used to be `() => {}`, which is TRUTHY, so
                // read-only surfaces advertised send-a-prompt controls that
                // did nothing when clicked. Absent means absent.
                isUserMessage={isUserMessage} // Use the same function as BaseChat
                onOpenArtifact={onOpenArtifact}
                // No terminal on this surface, and no chat to open one in: a
                // saved transcript is a record, and a shell code block in it
                // is history, not an offer. Explicitly null rather than
                // omitted, so the absence is a decision and not an oversight
                // — the same reason `append` is absent above.
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
              title="No messages in this chat"
              description="This chat was created but nothing was ever said in it."
              compact
            />
          )}
        </div>
      </div>
    </ScrollArea>
  );
};

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
  const [isCopied, setIsCopied] = useState(false);
  const [canShare, setCanShare] = useState(false);
  // Issue #56 §12.1's second entry point. The session page is where a user goes
  // to answer "what is in this chat?", so it is the other place the answer "no
  // longer anything private" belongs. Same dialog as History's row menu, so the
  // two cannot come to ask for different confirmations.
  const [declassifyOpen, setDeclassifyOpen] = useState(false);
  const [tier, setTier] = useState(session.privacy_tier);
  // The same panel the live chat mounts, from the same hook — a saved figure is
  // displayed exactly as a fresh one is, and there is no second renderer here.
  // `allowWindowResize` is false: opening a page must never resize the user's
  // window, and unlike a chat this surface is not somewhere they are working.
  const artifactPanel = useArtifactPanel({ isMobile: useIsMobile(), allowWindowResize: false });
  const { splitPaneRef, artifact: presentedArtifact, openArtifact } = artifactPanel;
  useEffect(() => setTier(session.privacy_tier), [session.privacy_tier]);

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
        messages,
        session.name || 'Shared chat',
        billedTokenEstimate?.lowerBound ? null : (billedTokenEstimate?.value ?? null)
      );

      const shareableLink = `biorouter://sessions/${shareToken}`;
      setShareLink(shareableLink);
      setIsShareModalOpen(true);
    } catch (error) {
      console.error('Error sharing session:', error);
      toastError({
        title: 'Failed to share chat',
        msg: error instanceof Error ? error.message : 'Unknown error',
      });
    } finally {
      setIsSharing(false);
    }
  };

  const handleCopyLink = () => {
    navigator.clipboard
      .writeText(shareLink)
      .then(() => {
        setIsCopied(true);
        setTimeout(() => setIsCopied(false), 2000);
      })
      .catch((err) => {
        console.error('Failed to copy link:', err);
        toastError({
          title: 'Failed to copy link',
          msg: 'The chat link could not be copied to the clipboard.',
        });
      });
  };

  const handleResumeSession = () => {
    try {
      resumeSession(session, setView);
    } catch (error) {
      toastError({
        title: 'Could not open this chat',
        msg: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const actionButtons = showActionButtons ? (
    <>
      {/* V7 — the Share button carries no `className`. It hand-painted the
          disabled look (`cursor-not-allowed opacity-50`) that
          `buttonVariants`' base already supplies as
          `disabled:pointer-events-none disabled:opacity-50`, keyed off the same
          `disabled` prop. ⚠ That base rule also means the tooltip explaining
          WHY sharing is unavailable has never fired — a pointer-events-none
          trigger receives no hover — and deleting the override does not change
          that either way. Restoring it needs a wrapper the trigger can sit on,
          which is a behaviour change and not this PR's. */}
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            onClick={handleShare}
            disabled={!canShare || isSharing}
            size="sm"
            variant="outline"
          >
            {isSharing ? (
              <>
                <LoaderCircle className="w-4 h-4 mr-2 animate-spin" />
                Sharing...
              </>
            ) : (
              <>
                <Share2 className="w-4 h-4" />
                Share
              </>
            )}
          </Button>
        </TooltipTrigger>
        {!canShare ? (
          <TooltipContent>
            {/* Deliberately does NOT name a settings path: chat sharing has
                no mounted settings section, so the old "Settings > Session >
                Session Sharing" sent the user somewhere that does not exist. */}
            <p>Chat sharing is not set up on this install.</p>
          </TooltipContent>
        ) : null}
      </Tooltip>
      <Button onClick={handleResumeSession} size="sm" variant="outline">
        <Sparkles className="w-4 h-4" />
        Resume
      </Button>
      {tier === 'private' && (
        <Button onClick={() => setDeclassifyOpen(true)} size="sm" variant="outline">
          Make public
        </Button>
      )}
    </>
  ) : null;

  return (
    <>
      <MainPanelLayout>
        {/* The horizontal split the artifact panel needs. It wraps
            ReadableContent rather than sitting inside it: the readable measure
            is a ceiling on PROSE, and a panel inside it would eat the column it
            is meant to sit beside. `splitPaneRef` goes here because rung 2
            measures this box — the one the transcript and the panel share. */}
        <div ref={splitPaneRef} className="relative flex flex-1 min-h-0 min-w-0">
          {/* ⚠ `size="chat"`, and it is the ONLY measure on this surface. The
              transcript used to sit in a second, narrower box inside this one
              (`max-w-4xl` — the 896px "replay fork"), so a saved conversation
              was drawn at a width the live chat never uses. §4.4 of the design
              of record retires it: one column, one number, and it is the same
              `--measure-chat` the composer reads. */}
          <ReadableContent size="chat" className="flex-1 flex flex-col min-h-0 px-6">
            <SessionHeader
              onBack={onBack}
              title={session.name}
              // The full pill, not the dense dot: this page has room, and it is
              // the surface a user opens to answer "what is in this chat?". It
              // reads the LOCAL tier, so a declassification made from the button
              // beside it clears the badge without waiting for a refetch — an
              // action whose only visible effect arrives on the next page load
              // reads as an action that did nothing.
              titleAdornment={tier ? <PrivacyBadge tier={tier} /> : null}
              actionButtons={!isLoading ? actionButtons : null}
            >
              <div className="flex flex-col">
                {!isLoading ? (
                  <>
                    <div className="flex items-center text-text-muted text-supporting gap-5 font-mono tabular-nums">
                      <span className="flex items-center">
                        <Calendar className="w-4 h-4 mr-1" />
                        {formatMessageTimestamp(messages[0]?.created)}
                      </span>
                      <span className="flex items-center">
                        <MessageSquareText className="w-4 h-4 mr-1" />
                        {session.message_count}
                      </span>
                      {billedTokenEstimate && (
                        <span
                          className="flex items-center"
                          title={
                            billedTokenEstimate.lowerBound
                              ? 'At least this many tokens; only last-turn usage is available for this older chat'
                              : 'Billed tokens across every turn, including recorded cache usage'
                          }
                        >
                          <Target className="w-4 h-4 mr-1" />
                          <span className="sr-only">Billed tokens: </span>
                          {formatBilledTokenEstimate(billedTokenEstimate)}
                        </span>
                      )}
                    </div>
                    <div className="flex items-center text-text-muted text-supporting mt-1 font-mono">
                      <span className="flex items-center">
                        <Folder className="w-4 h-4 mr-1" />
                        {session.working_dir}
                      </span>
                    </div>
                  </>
                ) : (
                  // V6 — a status line takes `text-supporting`, the role the
                  // metadata it stands in for uses.
                  <div className="flex items-center text-supporting text-text-muted">
                    <LoaderCircle className="w-4 h-4 mr-2 animate-spin" />
                    <span>Loading chat details...</span>
                  </div>
                )}
              </div>
            </SessionHeader>

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

          {/* No `onRenderError`: a saved transcript has no live turn to hand a
              broken figure back to, so ArtifactViewer installs no repair
              listener. And nothing auto-opens here — the panel appears when the
              reader clicks a card, never because the page loaded. */}
          {presentedArtifact && <ArtifactViewer {...artifactPanel.viewerProps} />}
        </div>
      </MainPanelLayout>

      <Dialog open={isShareModalOpen} onOpenChange={setIsShareModalOpen}>
        {/* V8 — the `MODAL_SIZE` ladder, not `sm:max-w-md`. That alias is 448px,
            a fourth width beside the ladder's 400/480/640, and nothing chose
            it: it is `DialogContent`'s own `sm:max-w-lg` typed one rung down. */}
        <DialogContent className={MODAL_SIZE.md}>
          <DialogHeader>
            <DialogTitle className="flex justify-center items-center gap-2">
              <Share2 className="w-6 h-6 text-text-default" />
              Share chat (beta)
            </DialogTitle>
            <DialogDescription>
              Share this link to give others a read-only view of this chat.
            </DialogDescription>
          </DialogHeader>

          <div className="py-4">
            <div className="relative rounded-container border border-border-subtle px-3 py-2 flex items-center bg-background-medium">
              <code className="text-code text-text-default overflow-x-hidden break-all pr-8 w-full">
                {shareLink}
              </code>
              <Button
                shape="pill"
                variant="ghost"
                className="absolute right-2 top-1/2 -translate-y-1/2"
                onClick={handleCopyLink}
                disabled={isCopied}
              >
                {isCopied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                <span className="sr-only">Copy</span>
              </Button>
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setIsShareModalOpen(false)}>
              Cancel
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

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
