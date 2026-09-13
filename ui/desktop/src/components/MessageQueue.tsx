import React, { useState } from 'react';
import {
  X,
  Send,
  StopSquare,
  MessageSquarePlus,
  GripVertical,
  ChevronDown,
  ChevronUp,
} from './icons/app-icons';
import { Button } from './ui/button';
import type { UserAttachment } from '../types/message';
import { ResourceRefText } from './ResourceRefChip';
import { joinComposerText, splitComposerText } from '../utils/composerRefs';
import { getSteerShortcutText } from '../utils/keyboardShortcuts';
import { SteerUnavailableNote } from './privacy/SteerUnavailableNote';
import { steerUnavailableReason } from './privacy/steerUnavailableCopy';

const STEER_TITLE = 'Add to current turn without stopping';
const STOP_AND_SEND_TITLE = 'Stop current turn, then send as a new turn';

/**
 * Hover text for an "Add now" button.
 *
 * The Cmd/Ctrl+Enter fallback in `ChatInput` steers the FRONT of the queue and
 * only the front, so the chord is advertised on that row alone. Teaching it on
 * every row would name a key that does something else (it would take message
 * one) for every row but the first.
 */
const steerTitle = (isNext: boolean) =>
  isNext ? `${STEER_TITLE} (${getSteerShortcutText()})` : STEER_TITLE;

const SteerActionContent = () => (
  <>
    <MessageSquarePlus className="w-3 h-3" aria-hidden="true" />
    <span className="text-[11px] leading-none">Add now</span>
  </>
);

const StopAndSendActionContent = () => (
  <>
    <span className="inline-flex items-center gap-0.5" aria-hidden="true">
      <StopSquare className="w-3 h-3" />
      <span className="text-[10px] leading-none">→</span>
      <Send className="w-3 h-3" />
    </span>
    <span className="text-[11px] leading-none">Stop &amp; send</span>
  </>
);

interface QueuedMessage {
  id: string;
  content: string;
  attachments?: UserAttachment[];
  timestamp: number;
}

/**
 * Is this message eligible for a soft interrupt?
 *
 * A soft interrupt is plain text — a message with attachments still has to wait
 * for the turn to end (or stop it). Exported because `ChatInput`'s
 * Cmd/Ctrl+Enter fallback steers the front of the queue, and a shortcut that
 * re-derived eligibility would drift from the button it mirrors: the two must
 * ask ONE question. The caller supplies the other half — whether steering is
 * available at all (`onSteerMessage` here, `canSteer` there).
 */
export const canSteerMessage = (message: Pick<QueuedMessage, 'attachments'>): boolean =>
  !message.attachments?.length;

interface MessageQueueProps {
  queuedMessages: QueuedMessage[];
  onRemoveMessage: (id: string) => void;
  onClearQueue: () => void;
  onStopAndSend?: (messageId: string) => void;
  /** BR-61 soft interrupt: hand this message to the running turn without
   * stopping it. Undefined when the session has no steerable turn in flight. */
  onSteerMessage?: (messageId: string) => void;
  onEditMessage?: (messageId: string, newContent: string) => void;
  onTriggerQueueProcessing?: () => void;
  editingMessageIdRef?: React.MutableRefObject<string | null>;
  onReorderMessages?: (reorderedMessages: QueuedMessage[]) => void;
  className?: string;
  isPaused?: boolean;
}

export const MessageQueue: React.FC<MessageQueueProps> = ({
  queuedMessages,
  onRemoveMessage,
  onClearQueue,
  onStopAndSend,
  onSteerMessage,
  onEditMessage,
  onTriggerQueueProcessing,
  editingMessageIdRef,
  onReorderMessages,
  className = '',
  isPaused = false,
}) => {
  const [isExpanded, setIsExpanded] = useState(false);
  const [draggedItem, setDraggedItem] = useState<string | null>(null);
  const [dragOverItem, setDragOverItem] = useState<string | null>(null);
  const [editingMessage, setEditingMessage] = useState<string | null>(null);
  const [editContent, setEditContent] = useState<string>('');

  if (queuedMessages.length === 0) {
    return null;
  }

  /**
   * SD-8. `null` on the desktop; on a browser-served session the sentence that
   * takes the place of every "Add now" in this widget.
   *
   * Read here rather than passed in, on `HostManagedModelNote`'s contract: a
   * prop is a thing a call site can forget, and the one that forgot would leave
   * a button that answers 403 and says nothing — which is the defect.
   */
  const steerRefusal = steerUnavailableReason();

  /**
   * Does the steer apply at all right now? `onSteerMessage` is `undefined`
   * whenever no turn is in flight, so this is also "is there a running turn" —
   * and it is what decides whether the note has anything to explain. A queue
   * sitting in front of an idle agent is missing no control.
   */
  const steerApplies = Boolean(onSteerMessage);

  const isSteerable = (message: QueuedMessage) =>
    steerApplies && !steerRefusal && canSteerMessage(message);

  const handleDragStart = (e: React.DragEvent, messageId: string) => {
    setDraggedItem(messageId);
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/html', messageId);
  };

  const handleDragOver = (e: React.DragEvent, messageId: string) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    setDragOverItem(messageId);
  };

  const handleDragLeave = () => {
    setDragOverItem(null);
  };

  const handleDrop = (e: React.DragEvent, targetMessageId: string) => {
    e.preventDefault();

    if (!draggedItem || !onReorderMessages) return;

    const draggedIndex = queuedMessages.findIndex((msg) => msg.id === draggedItem);
    const targetIndex = queuedMessages.findIndex((msg) => msg.id === targetMessageId);

    if (draggedIndex === -1 || targetIndex === -1 || draggedIndex === targetIndex) {
      setDraggedItem(null);
      setDragOverItem(null);
      return;
    }

    const newMessages = [...queuedMessages];
    const [removed] = newMessages.splice(draggedIndex, 1);
    newMessages.splice(targetIndex, 0, removed);

    onReorderMessages(newMessages);
    setDraggedItem(null);
    setDragOverItem(null);
  };

  const handleDragEnd = () => {
    setDraggedItem(null);
    setDragOverItem(null);
  };

  const formatTimestamp = (timestamp: number) => {
    const now = Date.now();
    const diff = now - timestamp;
    if (diff < 60000) return 'now';
    if (diff < 3600000) return `${Math.floor(diff / 60000)}m`;
    return `${Math.floor(diff / 3600000)}h`;
  };

  const nextMessage = queuedMessages[0];
  const remainingCount = queuedMessages.length - 1;
  // Issue #65 — the queue draws inside the composer, so the same rule holds:
  // never the raw `<biorouter-ref …>` markup. This builds the *string* form,
  // for `title` and aria; the visible row renders the same content through
  // `ResourceRefText`, which draws the references as chips.
  const messageLabel = (message: QueuedMessage) => {
    const attachmentCount = message.attachments?.length ?? 0;
    const { body, refs } = splitComposerText(message.content);
    const named = refs.map((ref) => ref.label?.trim() || ref.value);
    const text = [body.trim(), ...named].filter(Boolean).join(' · ');
    if (text && attachmentCount > 0)
      return `${text} (${attachmentCount} attachment${attachmentCount === 1 ? '' : 's'})`;
    if (text) return text;
    if (attachmentCount > 0)
      return `${attachmentCount} attachment${attachmentCount === 1 ? '' : 's'}`;
    return 'Queued message';
  };

  // Status dot: accent when active/next, muted when paused.
  const statusDot = (
    <span
      className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${isPaused ? 'bg-text-muted' : 'bg-background-accent animate-pulse'}`}
      aria-hidden="true"
    />
  );

  // Compact collapsed bar — single line, sidebar-row sized.
  if (!isExpanded) {
    return (
      <div className={className}>
        <div
          className="flex items-center gap-2 px-3 py-1.5 bg-background-default hover:bg-background-muted transition-colors cursor-pointer"
          onClick={() => setIsExpanded(true)}
          role="button"
          aria-label={`${queuedMessages.length} message${
            queuedMessages.length !== 1 ? 's' : ''
          } queued. Expand queue.`}
        >
          {statusDot}
          <span className="text-[11px] font-medium text-text-muted flex-shrink-0">
            {isPaused ? 'Paused' : 'Next'}
          </span>

          <p
            className="flex-1 min-w-0 text-xs text-text-default truncate"
            title={messageLabel(nextMessage)}
          >
            <ResourceRefText text={nextMessage.content.trim()} />
          </p>

          {remainingCount > 0 && (
            <span className="flex-shrink-0 text-[11px] text-text-muted bg-background-medium border border-border-subtle px-1.5 py-0.5 rounded-md font-medium">
              +{remainingCount}
            </span>
          )}

          {isSteerable(nextMessage) && (
            <Button
              variant="ghost"
              size="sm"
              onClick={(e) => {
                e.stopPropagation();
                onSteerMessage?.(nextMessage.id);
              }}
              className="h-6 px-1.5 gap-1 flex-shrink-0 text-text-muted hover:text-text-default"
              title={steerTitle(true)}
              aria-label="Add this message to the current turn"
            >
              <SteerActionContent />
            </Button>
          )}

          {onStopAndSend && (
            <Button
              variant="ghost"
              size="sm"
              onClick={(e) => {
                e.stopPropagation();
                onStopAndSend(nextMessage.id);
              }}
              className="h-6 px-1.5 gap-1 flex-shrink-0 text-text-muted hover:text-text-default"
              title={STOP_AND_SEND_TITLE}
              aria-label="Stop the current turn and send this message as a new turn"
            >
              <StopAndSendActionContent />
            </Button>
          )}

          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              setIsExpanded(true);
            }}
            className="h-6 w-6 p-0 flex-shrink-0 text-text-muted hover:text-text-default"
            title="Expand queue"
          >
            <ChevronDown className="w-3.5 h-3.5" />
          </Button>
        </div>
        {/* SD-8: the reason the "Add now" above is missing, in the row it is
            missing from. `short` because the collapsed bar is the compact
            shape, and a three-line block under a one-line strip would be the
            note shouting louder than the queue it annotates.

            Mounted on `steerApplies` alone — the note itself renders nothing on
            the desktop, so this condition is "is there a turn to steer", not
            "which surface is this". */}
        {steerApplies && <SteerUnavailableNote short />}
      </div>
    );
  }

  // Expanded list — compact rows, still scannable.
  return (
    <div className={className}>
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-1.5 bg-background-default border-b border-border-subtle">
        {statusDot}
        <span className="text-[11px] font-medium text-text-default flex-shrink-0">
          {isPaused ? 'Queue paused' : 'Message queue'}
        </span>
        <span className="text-[11px] text-text-muted flex-shrink-0">
          {queuedMessages.length} {isPaused ? 'waiting' : 'queued'}
        </span>

        <div className="flex-1" />

        {queuedMessages.length > 1 && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onClearQueue}
            className="h-6 px-2 text-[11px] text-text-muted hover:text-text-danger"
            title="Clear all queued messages"
          >
            Clear
          </Button>
        )}

        <Button
          variant="ghost"
          size="sm"
          onClick={() => setIsExpanded(false)}
          className="h-6 w-6 p-0 text-text-muted hover:text-text-default"
          title="Collapse queue"
        >
          <ChevronUp className="w-3.5 h-3.5" />
        </Button>
      </div>

      {/* Message rows */}
      <div className="px-2 py-1.5 space-y-1 bg-background-default max-h-56 overflow-y-auto">
        {queuedMessages.map((message, index) => (
          <div
            key={message.id}
            className={`group relative flex items-center gap-2 rounded-md px-2 py-1.5 border transition-colors ${draggedItem === message.id ? 'opacity-60 border-border-strong bg-background-medium' : dragOverItem === message.id ? 'border-border-strong bg-background-medium' : 'border-border-subtle bg-background-muted hover:bg-background-medium'}`}
            draggable={onReorderMessages ? true : false}
            onDragStart={(e) => handleDragStart(e, message.id)}
            onDragOver={(e) => handleDragOver(e, message.id)}
            onDragLeave={handleDragLeave}
            onDrop={(e) => handleDrop(e, message.id)}
            onDragEnd={handleDragEnd}
          >
            {/* Drag handle */}
            {onReorderMessages && (
              <div
                className="opacity-0 group-hover:opacity-60 hover:opacity-100 transition-opacity cursor-grab active:cursor-grabbing flex-shrink-0"
                aria-label="Drag to reorder"
              >
                <GripVertical className="w-3.5 h-3.5 text-text-muted" />
              </div>
            )}

            {/* Position indicator */}
            <span
              className={`flex items-center justify-center w-4 h-4 flex-shrink-0 rounded-full text-[11px] font-semibold ${index === 0 && !isPaused ? 'bg-background-accent text-text-on-accent' : 'bg-background-strong text-text-muted'}`}
            >
              {index + 1}
            </span>

            {/* Content / inline editor */}
            <div className="flex-1 min-w-0">
              {editingMessage === message.id ? (
                <div className="space-y-1.5">
                  <textarea
                    value={splitComposerText(editContent).body}
                    onChange={(e) =>
                      setEditContent(
                        joinComposerText(e.target.value, splitComposerText(editContent).refs)
                      )
                    }
                    className="w-full text-xs bg-background-default border border-border-subtle rounded-md px-2 py-1 resize-none focus:border-border-strong"
                    rows={Math.min(Math.ceil(editContent.length / 60), 4)}
                    autoFocus
                  />
                  <div className="flex gap-1.5">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        if (onEditMessage) {
                          onEditMessage(message.id, editContent);
                        }
                        setEditingMessage(null);
                        if (editingMessageIdRef) editingMessageIdRef.current = null;
                        if (onTriggerQueueProcessing) {
                          setTimeout(onTriggerQueueProcessing, 100);
                        }
                        setEditContent('');
                      }}
                      className="h-6 px-2 text-[11px]"
                    >
                      Save
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setEditingMessage(null);
                        if (editingMessageIdRef) editingMessageIdRef.current = null;
                        if (onTriggerQueueProcessing) {
                          setTimeout(onTriggerQueueProcessing, 100);
                        }
                        setEditContent('');
                      }}
                      className="h-6 px-2 text-[11px]"
                    >
                      Cancel
                    </Button>
                  </div>
                </div>
              ) : (
                <p
                  className="text-xs text-text-default truncate cursor-pointer hover:text-text-default"
                  title={`${messageLabel(message)} (Click to edit text)`}
                  onClick={() => {
                    setEditingMessage(message.id);
                    if (editingMessageIdRef) editingMessageIdRef.current = message.id;
                    setEditContent(message.content);
                  }}
                >
                  <ResourceRefText text={message.content.trim()} />
                  {(message.attachments?.length ?? 0) > 0 && (
                    <span className="text-text-muted">
                      {` (${message.attachments!.length} attachment${
                        message.attachments!.length === 1 ? '' : 's'
                      })`}
                    </span>
                  )}
                </p>
              )}
            </div>

            {/* Right-side meta + actions */}
            <div className="flex items-center gap-1 flex-shrink-0">
              <span className="text-[11px] text-text-muted font-mono">
                {formatTimestamp(message.timestamp)}
              </span>

              {isSteerable(message) && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onSteerMessage?.(message.id)}
                  disabled={editingMessage === message.id}
                  className={`h-6 px-1.5 gap-1 text-text-muted hover:text-text-default ${editingMessage === message.id ? 'opacity-30 cursor-not-allowed' : ''}`}
                  title={
                    editingMessage === message.id
                      ? 'Cannot send while editing'
                      : steerTitle(index === 0)
                  }
                  aria-label="Add this message to the current turn"
                >
                  <SteerActionContent />
                </Button>
              )}

              {onStopAndSend && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onStopAndSend(message.id)}
                  disabled={editingMessage === message.id}
                  className={`h-6 px-1.5 gap-1 text-text-muted hover:text-text-default ${editingMessage === message.id ? 'opacity-30 cursor-not-allowed' : ''}`}
                  title={
                    editingMessage === message.id
                      ? 'Cannot send while editing'
                      : STOP_AND_SEND_TITLE
                  }
                  aria-label="Stop the current turn and send this message as a new turn"
                >
                  <StopAndSendActionContent />
                </Button>
              )}

              <Button
                variant="ghost"
                size="sm"
                onClick={() => onRemoveMessage(message.id)}
                className="h-6 w-6 p-0 text-text-muted hover:text-text-danger opacity-0 group-hover:opacity-100 transition-opacity"
                title="Remove this message from queue"
              >
                <X className="w-3 h-3" />
              </Button>
            </div>
          </div>
        ))}
      </div>
      {/* SD-8, once for the whole list rather than once per row: the reason is
          the daemon's, not this message's, so repeating it under every row
          would say one true thing N times. */}
      {steerApplies && <SteerUnavailableNote />}
    </div>
  );
};

export default MessageQueue;
