import React from 'react';
import { MessageSquare, AlertCircle } from '../icons/app-icons';
import { Spinner } from '../ui/spinner';
import { Button } from '../ui/button';
import { ScrollArea } from '../ui/scroll-area';
import MarkdownContent from '../MarkdownContent';
import { ResourceRefChip } from '../ResourceRefChip';
import { splitComposerText } from '../../utils/composerRefs';
import ToolCallWithResponse from '../ToolCallWithResponse';
import ImagePreview from '../ImagePreview';
import { ProvenanceChip } from '../ProvenanceChip';
import {
  getTextContent,
  ToolRequestMessageContent,
  ToolResponseMessageContent,
} from '../../types/message';
import { formatMessageTimestamp } from '../../utils/timeUtils';
import { extractImagePaths, removeImagePathsFromText } from '../../utils/imageUtils';
import { Message } from '../../api';
import { EmptyState } from '../ui/empty-state';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import { filePathLookupBeforeMessage } from '../artifacts/artifactFileProvenance';
import {
  BIOROUTER,
  CHAT_LOAD_ERROR_TITLE,
  LOADING_CHAT,
  NO_MESSAGES,
  NO_MESSAGES_TITLE,
  TRY_AGAIN,
  YOU,
} from './copy';

/**
 * Get tool responses map from messages
 */
export const getToolResponsesMap = (
  messages: Message[],
  messageIndex: number,
  toolRequests: ToolRequestMessageContent[]
) => {
  const responseMap = new Map();

  if (messageIndex >= 0) {
    for (let i = messageIndex + 1; i < messages.length; i++) {
      const responses = messages[i].content
        .filter((c) => c.type === 'toolResponse')
        .map((c) => c as ToolResponseMessageContent);

      for (const response of responses) {
        const matchingRequest = toolRequests.find((req) => req.id === response.id);
        if (matchingRequest) {
          responseMap.set(response.id, response);
        }
      }
    }
  }

  return responseMap;
};

interface SessionMessagesProps {
  messages: Message[];
  sessionId: string;
  isLoading: boolean;
  error: string | null;
  onRetry: () => void;
  /**
   * Where a figure, an app card or a written file goes when the reader clicks
   * it. Required, not optional: this is a transcript, and the artifact side
   * panel is the only surface any of them is ever displayed on. A transcript
   * that cannot open one has nowhere to put it.
   */
  onOpenArtifact: (artifact: ArtifactSource) => void;
  /**
   * The chat's working directory, so a relative path in a tool call resolves to
   * a real file. A shared session carries one; without it `resolveArtifactPath`
   * drops every relative artifact and the transcript silently shows fewer
   * figures than it contains.
   */
  workingDir?: string;
}

/**
 * Common component for displaying session messages
 */
export const SessionMessages: React.FC<SessionMessagesProps> = ({
  messages,
  sessionId,
  isLoading,
  error,
  onRetry,
  onOpenArtifact,
  workingDir,
}) => {
  return (
    // `data-preview-transcript`: the box rung 2 measures as a replay's transcript,
    // so the page header above it counts as the conversation's chrome.
    <ScrollArea className="h-full w-full" data-preview-transcript="">
      <div className="flex flex-col gap-6 pt-4 pb-24">
        {isLoading ? (
          <div className="flex justify-center items-center py-12">
            <Spinner label={LOADING_CHAT} />
          </div>
        ) : error ? (
          // §4.5 — the shared surface, not a fourth hand-rolled error column.
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
        ) : messages?.length > 0 ? (
          messages
            .map((message, index) => {
              const textContent = getTextContent(message);
              // Extract image paths from the message
              const imagePaths = extractImagePaths(textContent);

              // Remove image paths from text for display
              const displayText =
                imagePaths.length > 0
                  ? removeImagePathsFromText(textContent, imagePaths)
                  : textContent;

              // Issue #65 — this is the one surface that renders a user
              // message through `MarkdownContent`, and react-markdown runs
              // here without `rehype-raw`, so it DROPS unknown HTML rather
              // than showing it. A `<biorouter-ref …>` therefore vanished
              // without a trace: worse than raw markup, because a reader
              // reviewing the session could not tell a skill was attached.
              // The prose keeps its markdown; the references are drawn as
              // read-only chips beside it.
              const { body: proseText, refs: messageRefs } = splitComposerText(displayText);
              const knownFilePaths = filePathLookupBeforeMessage(
                messages,
                index,
                sessionId,
                workingDir
              );

              // Get tool requests from the message
              const toolRequests = message.content
                .filter((c) => c.type === 'toolRequest')
                .map((c) => c as ToolRequestMessageContent);

              // Get tool responses map using the helper function
              const toolResponsesMap = getToolResponsesMap(messages, index, toolRequests);

              // Skip pure tool response messages for cleaner display
              const isOnlyToolResponse =
                message.content.length > 0 &&
                message.content.every((c) => c.type === 'toolResponse');

              if (message.role === 'user' && isOnlyToolResponse) {
                return null;
              }

              const isUser = message.role === 'user';

              return (
                /* A message is a row of the document, not a card (principle 4):
                   an author line, then the content. The user's words sit on the
                   same `--background-medium` fill the live chat gives them, so
                   the turn boundary reads the same here as in the chat it came
                   from; the assistant's prose sits on the page. */
                <article key={index} data-role={message.role} className="flex flex-col gap-1.5">
                  <header className="flex items-center gap-2 min-w-0">
                    {/* BR-71 §5: provenance is structural, so it has to travel
                        with the transcript into this view too. A shared chat is
                        the one that leaves the machine, and "You" on a message
                        another agent injected is a misattribution to the human. */}
                    <span className="text-label text-text-default">{isUser ? YOU : BIOROUTER}</span>
                    <ProvenanceChip provenance={message.metadata?.provenance ?? undefined} />
                    <span className="ml-auto shrink-0 text-supporting text-text-muted tabular-nums">
                      {formatMessageTimestamp(message.created)}
                    </span>
                  </header>

                  <div
                    className={
                      isUser
                        ? 'flex flex-col gap-2 rounded-container bg-background-medium px-3.5 py-2.5'
                        : 'flex flex-col gap-2'
                    }
                  >
                    {proseText && (
                      <MarkdownContent
                        content={proseText}
                        onOpenArtifact={onOpenArtifact}
                        workingDir={workingDir}
                        knownFilePaths={knownFilePaths}
                      />
                    )}

                    {messageRefs.length > 0 && (
                      <div className="flex flex-wrap items-center gap-1.5">
                        {messageRefs.map((ref) => (
                          <ResourceRefChip key={`${ref.kind}:${ref.value}`} refSpan={ref} />
                        ))}
                      </div>
                    )}

                    {imagePaths.length > 0 && (
                      <div className="flex flex-wrap gap-2">
                        {imagePaths.map((imagePath, imageIndex) => (
                          <ImagePreview
                            key={imageIndex}
                            src={imagePath}
                            alt={`Image ${imageIndex + 1}`}
                          />
                        ))}
                      </div>
                    )}

                    {/* Tool rows sit straight in the message: no box around
                        them, because each row draws its own line (WS-TOOLS'
                        TranscriptRow) and a box inside a box is what principle 4
                        rules out. No entrance animation either: a saved
                        transcript mounts at rest. */}
                    {toolRequests.length > 0 && (
                      <div className="flex flex-col gap-1">
                        {toolRequests.map((toolRequest) => (
                          <ToolCallWithResponse
                            // In a saved transcript a request with no response
                            // was broken or cancelled.
                            isCancelledMessage={toolResponsesMap.get(toolRequest.id) == undefined}
                            key={toolRequest.id}
                            toolRequest={toolRequest}
                            toolResponse={toolResponsesMap.get(toolRequest.id)}
                            onOpenArtifact={onOpenArtifact}
                            workingDir={workingDir}
                          />
                        ))}
                      </div>
                    )}
                  </div>
                </article>
              );
            })
            .filter(Boolean) // Filter out null entries
        ) : (
          <EmptyState
            icon={MessageSquare}
            title={NO_MESSAGES_TITLE}
            description={NO_MESSAGES}
            compact
          />
        )}
      </div>
    </ScrollArea>
  );
};
