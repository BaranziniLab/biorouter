import { QuotedTextSelection, useTextSelection } from './QuotedTextSelection';
import { useMemo, useRef, useState } from 'react';
import ImagePreview from './ImagePreview';
import { InlineImage } from './InlineImage';
import { extractImagePaths, removeImagePathsFromText } from '../utils/imageUtils';
import { formatMessageTimestamp } from '../utils/timeUtils';
import MarkdownContent from './MarkdownContent';
import ToolCallWithResponse from './ToolCallWithResponse';
import { ToolCallRows } from './ToolCallRows';
import {
  getToolRequests,
  getToolResponses,
  getToolConfirmationContent,
  getElicitationContent,
  getSecretRequestContent,
  NotificationEvent,
} from '../types/message';
import { Message } from '../api';
import ToolCallConfirmation from './ToolCallConfirmation';
import ElicitationRequest from './ElicitationRequest';
import SecretRequestCard from './SecretRequestCard';
import MessageCopyLink from './MessageCopyLink';
import MessageDivergeLink from './MessageDivergeLink';
import { MessageActions } from './MessageMeta';
import { Brain } from './icons/app-icons';
import { TranscriptRow } from './TranscriptRow';
import { transcriptCopy } from './transcript/copy';
import { cn } from '../utils';
import { presentStoredTurnError, TurnErrorRow } from './conversation/ChatTurnError';
import { splitAssistantText } from './transcript/transcriptRhythm';
import { ThinkingClock, thoughtTook, useThoughtDuration } from './transcript/thinkingClock';
import type { ArtifactSource } from './artifacts/artifactTypes';
import {
  filePathLookupFromPaths,
  filePathsBeforeMessage,
} from './artifacts/artifactFileProvenance';

interface BioRouterMessageProps {
  sessionId: string;
  message: Message;
  messages: Message[];
  metadata?: string[];
  toolCallNotifications: Map<string, NotificationEvent[]>;
  isStreaming?: boolean; // Whether this message is currently being streamed
  /**
   * Whether the chat TURN is still running. Distinct from `isStreaming`, which
   * is only true for the last message: a tool call keeps executing after its
   * message stops being last, and tool status must follow the turn, not the
   * array position.
   */
  turnActive?: boolean;
  // Precomputed by the parent list once per render and passed down to avoid each
  // message recomputing an O(n) scan over the whole chat (which made the list
  // O(n²) per streaming frame). Falls back to a local scan when omitted, so the
  // component still works standalone.
  messageIndex?: number;
  submitElicitationResponse?: (
    elicitationId: string,
    userData: Record<string, unknown>
  ) => Promise<void>;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
  /**
   * Hand a shell code block to this chat's in-app terminal, or `null` on a
   * surface that has none.
   *
   * REQUIRED and nullable rather than optional, the same discipline
   * `onOpenArtifact` carries next to it: a read-only transcript legitimately
   * has no terminal, and it should have to SAY so. An optional prop would let a
   * live chat lose the wiring in a refactor and look exactly like one of the
   * surfaces that never had it.
   */
  onRunInTerminal: ((command: string) => boolean) | null;
  /**
   * The row was appended while the person watched (`useLiveRows`), so a
   * failure it reports is announced. A row read back from history is not.
   */
  live?: boolean;
}

export default function BioRouterMessage({
  sessionId,
  message,
  messages,
  toolCallNotifications,
  isStreaming = false,
  turnActive = false,
  messageIndex: messageIndexProp,
  submitElicitationResponse,
  onOpenArtifact,
  onRunInTerminal,
  workingDir,
  live = false,
}: BioRouterMessageProps) {
  const contentRef = useRef<HTMLDivElement | null>(null);
  // Collapsed by default and sticky per message while it is mounted — a thought
  // you opened stays open as the turn continues below it.
  const [thinkingOpen, setThinkingOpen] = useState(false);

  // Injected context blocks (`<info-msg>`, model-only) are stripped and the
  // chain of thought is split off, by the same function the list's rhythm
  // reads, so the two can never disagree about whether a message starts with
  // prose.
  const { visibleText, cotText } = splitAssistantText(message);
  const toolRequests = getToolRequests(message);
  const toolConfirmationContent = getToolConfirmationContent(message);
  const elicitationContent = getElicitationContent(message);
  const secretRequestContent = getSecretRequestContent(message);
  // The thought is all there is so far: the answer has not started, and the
  // model has not moved on to a tool call or a question either. A thought that
  // is followed by a running tool is over ("Thought"); the tool row beside it
  // is what is working.
  const thinkingNow =
    isStreaming &&
    !visibleText &&
    toolRequests.length === 0 &&
    toolConfirmationContent === undefined &&
    elicitationContent === undefined &&
    secretRequestContent === undefined;
  // "Thinking · 8s" while it lasts, "Thought · 8s" after, when this view
  // watched it happen; a thought read from history has no measured length.
  const thought = useThoughtDuration(!!cotText && thinkingNow);
  const thinkingMeta =
    thinkingNow && thought.since !== undefined ? (
      <ThinkingClock since={thought.since} />
    ) : (
      thoughtTook(thought.tookMs)
    );
  const imagePaths = extractImagePaths(visibleText);
  const displayText =
    imagePaths.length > 0 ? removeImagePathsFromText(visibleText, imagePaths) : visibleText;

  // Extract structured ImageContent blocks (new sessions only; pre-v-next sessions have no image blocks)
  const imageContentBlocks = useMemo(
    () => message.content.filter((c) => c.type === 'image'),
    [message.content]
  );

  const timestamp = useMemo(() => formatMessageTimestamp(message.created), [message.created]);
  // Use the index passed by the parent list when available (O(1)); only fall
  // back to the O(n) scan when rendered standalone.
  const messageIndex = messageIndexProp ?? messages.findIndex((msg) => msg.id === message.id);
  // Keyed on WHAT the lookup answers, never on `messages`. `messages` is a new
  // array on every streamed chunk, so a lookup built from it was a new function
  // per chunk for every message in the transcript: MarkdownContent's memo never
  // held, and every finished message re-rendered its markdown on every token
  // (which, while its renderers were an inline map, also remounted each fenced
  // block and its Copy button mid-click). The files named before a finished
  // message do not change while the next reply streams, so neither does this.
  // NUL separates the paths because no path can contain one.
  const knownFilePathsKey = useMemo(
    () => filePathsBeforeMessage(messages, messageIndex, sessionId, workingDir).join('\0'),
    [messages, messageIndex, sessionId, workingDir]
  );
  const knownFilePaths = useMemo(
    () => filePathLookupFromPaths(knownFilePathsKey ? knownFilePathsKey.split('\0') : []),
    [knownFilePathsKey]
  );
  const hasToolConfirmation = toolConfirmationContent !== undefined;
  const hasElicitation = elicitationContent !== undefined;

  const toolResponsesMap = useMemo(() => {
    const responseMap = new Map();

    // BR-53: a message with no tool requests can never match a later tool
    // response, so skip the whole-conversation scan entirely. This memo ran
    // once per text-only message on every streamed token — O(n²) per frame —
    // purely to build an empty map. Guard on `toolRequests` first so the common
    // text-only message pays nothing.
    if (toolRequests.length > 0 && messageIndex !== undefined && messageIndex >= 0) {
      const requestIds = new Set(toolRequests.map((req) => req.id));
      for (let i = messageIndex + 1; i < messages.length; i++) {
        for (const response of getToolResponses(messages[i])) {
          if (requestIds.has(response.id)) {
            responseMap.set(response.id, response);
          }
        }
      }
    }

    return responseMap;
  }, [messages, messageIndex, toolRequests]);

  const quotedSelection = useTextSelection(contentRef, `${sessionId}:${message.id}`);

  // The daemon's own turn-ending notice ("Ran into this error: …") is stored as
  // an assistant message. It draws as the error row the live failure draws,
  // never as prose holding a raw provider response (vision V2-07).
  const storedError = useMemo(
    () =>
      toolRequests.length === 0 && message.content.every((content) => content.type === 'text')
        ? presentStoredTurnError(displayText)
        : null,
    [displayText, message.content, toolRequests.length]
  );
  if (storedError) {
    return (
      <div className="biorouter-message flex w-full justify-start min-w-0">
        <TurnErrorRow presentation={storedError} alert={live} testId="stored-turn-error" />
      </div>
    );
  }

  return (
    <div className="biorouter-message flex w-full justify-start min-w-0">
      <div className="flex flex-col w-full min-w-0">
        {/* Chain of thought is a transcript row, the same line a tool call is
            (WS-TOOLS' TranscriptRow): a 16px Brain, "Thinking" while the
            thought is all there is so far and "Thought" once the answer has
            started, and a chevron that opens the thought indented 24px under
            the label. It stays open while mounted once opened, so a thought
            you are reading does not shut as the turn continues below it. The
            time it took follows the label ("Thought · 8s"). */}
        {cotText && (
          <TranscriptRow
            className="mb-2"
            icon={Brain}
            glyph="thinking"
            label={thinkingNow ? transcriptCopy.activity.thinking : transcriptCopy.activity.thought}
            meta={thinkingMeta}
            running={thinkingNow}
            body="plain"
            open={thinkingOpen}
            onOpenChange={setThinkingOpen}
          >
            <MarkdownContent content={cotText} />
          </TranscriptRow>
        )}

        {displayText && (
          // The answer is the actions' scope: hovering or focusing anything in
          // it shows the floating cluster at its top right (Crew's row
          // actions). It comes BEFORE the words in the DOM so Tab reaches it
          // first; nothing in the flow is reserved for it.
          <div className="br-message-actions-scope flex flex-col">
            {toolRequests.length === 0 && !isStreaming && (
              <MessageActions timestamp={timestamp}>
                {message.content.every((content) => content.type === 'text') && (
                  <>
                    <MessageCopyLink text={displayText} contentRef={contentRef} />
                    <MessageDivergeLink
                      sessionId={sessionId}
                      truncateAfterMs={message.created}
                      truncateAfterId={message.id ?? undefined}
                    />
                  </>
                )}
              </MessageActions>
            )}
            <QuotedTextSelection
              source={{
                sessionId,
                title: 'Agent response',
                locator: `conversation:${sessionId}/message:${message.id}`,
              }}
              selection={quotedSelection}
            >
              <div ref={contentRef} className="w-full">
                <MarkdownContent
                  content={displayText}
                  onOpenArtifact={isStreaming ? undefined : onOpenArtifact}
                  workingDir={workingDir}
                  knownFilePaths={knownFilePaths}
                  // Withheld while the message is still streaming, exactly as
                  // onOpenArtifact is: a code fence that has not closed yet holds
                  // half a command, and half a command is a different command.
                  onRunInTerminal={isStreaming ? undefined : (onRunInTerminal ?? undefined)}
                />
              </div>
            </QuotedTextSelection>

            {imagePaths.length > 0 && (
              <div className="mt-2 flex flex-col gap-2">
                {imagePaths.map((imagePath, index) => (
                  <ImagePreview key={index} src={imagePath} />
                ))}
              </div>
            )}

            {/* Render structured ImageContent blocks (new sessions) */}
            {imageContentBlocks.length > 0 && (
              <div className="flex flex-wrap gap-2 mt-2">
                {imageContentBlocks.map((block, idx) =>
                  block.type === 'image' ? (
                    <InlineImage
                      key={idx}
                      kind="data"
                      data={block.data}
                      mimeType={block.mimeType}
                    />
                  ) : null
                )}
              </div>
            )}
          </div>
        )}

        {/* Tool rows are lines 4px apart (a 32px pitch for 28px rows), and the
            block sits 8px under any prose above it. A tool-only message keeps
            no time row of its own: the answer that ends the turn carries it. */}
        {toolRequests.length > 0 && (
          <div className={cn('flex w-full flex-col gap-1', displayText && 'mt-2')}>
            <ToolCallRows
              toolRequests={toolRequests}
              toolResponses={toolResponsesMap}
              turnActive={turnActive}
              renderCall={(toolRequest) => (
                <ToolCallWithResponse
                  sessionId={sessionId}
                  isCancelledMessage={false}
                  toolRequest={toolRequest}
                  toolResponse={toolResponsesMap.get(toolRequest.id)}
                  notifications={toolCallNotifications.get(toolRequest.id)}
                  isStreamingMessage={isStreaming}
                  turnActive={turnActive}
                  onOpenArtifact={onOpenArtifact}
                  workingDir={workingDir}
                />
              )}
            />
          </div>
        )}

        {/* The cards that ask the person to act. They are blocks of the turn,
            so they sit 8px under anything above them in this message. */}
        {(hasToolConfirmation ||
          (hasElicitation && submitElicitationResponse) ||
          secretRequestContent) && (
          <div
            className={cn(
              'flex flex-col gap-2',
              // The thinking row brings its own 8px (`mb-2`).
              (displayText || toolRequests.length > 0) && 'mt-2'
            )}
          >
            {hasToolConfirmation && (
              <ToolCallConfirmation
                sessionId={sessionId}
                isCancelledMessage={false}
                isClicked={false}
                actionRequiredContent={toolConfirmationContent}
              />
            )}

            {hasElicitation && submitElicitationResponse && (
              <ElicitationRequest
                isCancelledMessage={false}
                isClicked={false}
                actionRequiredContent={elicitationContent}
                onSubmit={submitElicitationResponse}
              />
            )}

            {/* Issue #117. Unlike every other card here it takes NO submit
              callback: it answers the daemon directly, because routing a
              credential back through the composer would put it in the
              transcript. */}
            {secretRequestContent && (
              <SecretRequestCard
                isCancelledMessage={false}
                actionRequiredContent={secretRequestContent}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
}
