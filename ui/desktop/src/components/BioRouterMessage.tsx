import { useMemo, useRef, useState } from 'react';
import ImagePreview from './ImagePreview';
import { InlineImage } from './InlineImage';
import { extractImagePaths, removeImagePathsFromText } from '../utils/imageUtils';
import { formatMessageTimestamp } from '../utils/timeUtils';
import MarkdownContent from './MarkdownContent';
import ToolCallWithResponse from './ToolCallWithResponse';
import {
  getTextContent,
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
import { MessageMeta } from './MessageMeta';
import { ChevronRight } from './icons/app-icons';
import { cn } from '../utils';
import { identifyConsecutiveToolCalls, shouldHideTimestamp } from '../utils/toolCallChaining';
import type { ArtifactSource } from './artifacts/artifactTypes';
import { filePathLookupBeforeMessage } from './artifacts/artifactFileProvenance';

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
  // message recomputing O(n) scans over the whole conversation (which made the
  // list O(n²) per streaming frame). Both fall back to local computation when
  // omitted, so the component still works standalone.
  messageIndex?: number;
  toolCallChains?: ReturnType<typeof identifyConsecutiveToolCalls>;
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
}

export default function BioRouterMessage({
  sessionId,
  message,
  messages,
  toolCallNotifications,
  isStreaming = false,
  turnActive = false,
  messageIndex: messageIndexProp,
  toolCallChains: toolCallChainsProp,
  submitElicitationResponse,
  onOpenArtifact,
  onRunInTerminal,
  workingDir,
}: BioRouterMessageProps) {
  const contentRef = useRef<HTMLDivElement | null>(null);
  // Collapsed by default and sticky per message while it is mounted — a thought
  // you opened stays open as the turn continues below it.
  const [thinkingOpen, setThinkingOpen] = useState(false);

  let textContent = getTextContent(message);

  // Strip injected context blocks that are meant for the model, not the user.
  // The trailing (\s*\\?") also consumes any closing \" the model places after </info-msg>
  // to wrap a quoted echo — which Markdown would otherwise render as a stray \.
  textContent = textContent.replace(/<info-msg>[\s\S]*?<\/info-msg>(\s*\\?")?/gi, '').trim();

  const splitChainOfThought = (text: string): { visibleText: string; cotText: string | null } => {
    const regex = /<think>([\s\S]*?)<\/think>/i;
    const match = text.match(regex);
    if (!match) {
      return { visibleText: text, cotText: null };
    }

    const cotRaw = match[1].trim();
    const visibleText = text.replace(regex, '').trim();

    return {
      visibleText,
      cotText: cotRaw || null,
    };
  };

  const { visibleText, cotText } = splitChainOfThought(textContent);
  const imagePaths = extractImagePaths(visibleText);
  const displayText =
    imagePaths.length > 0 ? removeImagePathsFromText(visibleText, imagePaths) : visibleText;

  // Extract structured ImageContent blocks (new sessions only; pre-v-next sessions have no image blocks)
  const imageContentBlocks = useMemo(
    () => message.content.filter((c) => c.type === 'image'),
    [message.content]
  );

  const timestamp = useMemo(() => formatMessageTimestamp(message.created), [message.created]);
  const toolRequests = getToolRequests(message);
  // Use the index passed by the parent list when available (O(1)); only fall
  // back to the O(n) scan when rendered standalone.
  const messageIndex = messageIndexProp ?? messages.findIndex((msg) => msg.id === message.id);
  const knownFilePaths = useMemo(
    () => filePathLookupBeforeMessage(messages, messageIndex, sessionId, workingDir),
    [messages, messageIndex, sessionId, workingDir]
  );
  const toolConfirmationContent = getToolConfirmationContent(message);
  const elicitationContent = getElicitationContent(message);
  const secretRequestContent = getSecretRequestContent(message);
  // Prefer the parent's single precomputed chain map; only recompute locally
  // (the old O(n)-per-message cost) when not provided.
  const toolCallChains = useMemo(
    () => toolCallChainsProp ?? identifyConsecutiveToolCalls(messages),
    [toolCallChainsProp, messages]
  );
  const hideTimestamp = useMemo(
    () => shouldHideTimestamp(messageIndex, toolCallChains),
    [messageIndex, toolCallChains]
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

  return (
    <div className="biorouter-message flex w-full justify-start min-w-0">
      <div className="flex flex-col w-full min-w-0">
        {/* Chain of thought is a disclosure, not a card. It used to be a native
            `<details>` on a filled, bordered 4px box with the browser's own
            triangle — the one control in the transcript the USER AGENT drew
            rather than the app, so it could not take the app's chevron, its
            32px row, its motion or its type role, and it was the last place a
            marker rotated with no transition. It is now the same disclosure the
            tool rows use: a 32px line, a 16px chevron VISIBLE AT REST (a marker
            that appears on hover teaches nobody that the row opens), and a body
            indented 24px to the left edge that chevron establishes. */}
        {cotText && (
          <div className="mb-2">
            <button
              type="button"
              onClick={() => setThinkingOpen((open) => !open)}
              aria-expanded={thinkingOpen}
              className="flex h-8 cursor-pointer items-center gap-2 rounded-element text-secondary text-text-muted transition-colors hover:text-text-default"
            >
              <ChevronRight
                aria-hidden="true"
                className={cn('size-4 shrink-0 transition-transform', thinkingOpen && 'rotate-90')}
              />
              <span>{thinkingOpen ? 'Hide thinking' : 'Show thinking'}</span>
            </button>
            {thinkingOpen && (
              <div className="pl-6">
                <MarkdownContent content={cotText} />
              </div>
            )}
          </div>
        )}

        {displayText && (
          <div className="flex flex-col group">
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

            {imagePaths.length > 0 && (
              <div className="mt-4">
                {imagePaths.map((imagePath, index) => (
                  <ImagePreview key={index} src={imagePath} />
                ))}
              </div>
            )}

            {/* Render structured ImageContent blocks (new sessions) */}
            {imageContentBlocks.length > 0 && (
              <div className="flex flex-wrap gap-2 mt-4">
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

            {toolRequests.length === 0 && !isStreaming && (
              <MessageMeta timestamp={timestamp}>
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
              </MessageMeta>
            )}
          </div>
        )}

        {toolRequests.length > 0 && (
          <div className={cn(displayText && 'mt-2')}>
            <div className="relative flex flex-col w-full">
              <div className="flex flex-col gap-3">
                {toolRequests.map((toolRequest) => (
                  <div className="biorouter-message-tool" key={toolRequest.id}>
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
                  </div>
                ))}
              </div>
              {!isStreaming && !hideTimestamp && <MessageMeta timestamp={timestamp} />}
            </div>
          </div>
        )}

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
    </div>
  );
}
