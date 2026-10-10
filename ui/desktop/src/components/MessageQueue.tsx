import React, { useState } from 'react';
import {
  ArrowUp,
  ChevronDown,
  ChevronUp,
  GripVertical,
  MessageSquarePlus,
  X,
} from './icons/app-icons';
import { Button } from './ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';
import type { UserAttachment } from '../types/message';
import { ResourceRefText } from './ResourceRefChip';
import { joinComposerText, splitComposerText } from '../utils/composerRefs';
import { getSteerAriaKeyShortcuts, getSteerShortcutText } from '../utils/keyboardShortcuts';
import { SteerUnavailableNote } from './privacy/SteerUnavailableNote';
import { steerUnavailableReason } from './privacy/steerUnavailableCopy';
import { COMPOSER_COPY } from './composer/copy';
import { cn } from '../utils';
import './composer/composer.css';

const COPY = COMPOSER_COPY.queue;

/**
 * Tooltip for an "Add now" button.
 *
 * The Cmd/Ctrl+Enter fallback in `ChatInput` steers the FRONT of the queue and
 * only the front, so the chord is named on that row alone (in the tooltip and
 * in `aria-keyshortcuts`). Naming it on every row would name a key that does
 * something else (it would take message one) for every row but the first.
 */
const steerTooltip = (isNext: boolean) =>
  isNext ? `${COPY.addNowTooltip} (${getSteerShortcutText()})` : COPY.addNowTooltip;

interface QueuedMessage {
  id: string;
  content: string;
  attachments?: UserAttachment[];
  timestamp: number;
}

/**
 * Is this message eligible for a soft interrupt?
 *
 * A soft interrupt is plain text: a message with attachments still has to wait
 * for the turn to end (or stop it). Exported because `ChatInput`'s
 * Cmd/Ctrl+Enter fallback steers the front of the queue, and a shortcut that
 * re-derived eligibility would drift from the button it mirrors: the two must
 * ask ONE question. The caller supplies the other half, whether steering is
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

/** A 24px ghost icon button whose name is also its tooltip, unless `tooltip` says more. */
function QueueAction({
  label,
  tooltip,
  onClick,
  disabled,
  danger,
  keyShortcuts,
  children,
}: {
  label: string;
  tooltip?: string;
  onClick: (event: React.MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
  danger?: boolean;
  keyShortcuts?: string;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* The span keeps the tooltip reachable while the button is disabled. */}
        <span className="inline-flex">
          <Button
            type="button"
            variant="ghost"
            size="xs"
            shape="round"
            aria-label={label}
            aria-keyshortcuts={keyShortcuts}
            disabled={disabled}
            onClick={onClick}
            className={cn(
              'text-text-muted hover:text-text-default',
              danger && 'hover:text-text-danger'
            )}
          >
            {children}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>{tooltip ?? label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The messages waiting for the running turn, at the top of the composer card.
 *
 * Codex's queue: one "Queued · N" line, then each message as `↳ text`. Folded,
 * it is one line holding the next message and its two actions; unfolded, every
 * row reveals its actions on hover or focus.
 */
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
   * a button that answers 403 and says nothing, which is the defect.
   */
  const steerRefusal = steerUnavailableReason();

  /**
   * Does the steer apply at all right now? `onSteerMessage` is `undefined`
   * whenever no turn is in flight, so this is also "is there a running turn",
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

  const attachmentCount = (message: QueuedMessage) => message.attachments?.length ?? 0;

  const endEditing = () => {
    setEditingMessage(null);
    if (editingMessageIdRef) editingMessageIdRef.current = null;
    if (onTriggerQueueProcessing) {
      setTimeout(onTriggerQueueProcessing, 100);
    }
    setEditContent('');
  };

  const startEditing = (message: QueuedMessage) => {
    setEditingMessage(message.id);
    if (editingMessageIdRef) editingMessageIdRef.current = message.id;
    setEditContent(message.content);
  };

  /**
   * The message as one line. Issue #65: the queue draws inside the composer, so
   * the same rule holds: never the raw `<biorouter-ref …>` markup.
   * `ResourceRefText` draws the references as chips.
   */
  const messageText = (message: QueuedMessage) => (
    <>
      <ResourceRefText text={message.content.trim()} />
      {attachmentCount(message) > 0 && (
        <span className="text-text-muted">
          {message.content.trim() ? ' · ' : ''}
          {COPY.attachments(attachmentCount(message))}
        </span>
      )}
    </>
  );

  /** The two ways a queued message can reach the running turn early. */
  const turnActions = (message: QueuedMessage, isNext: boolean) => {
    const editing = editingMessage === message.id;
    return (
      <>
        {isSteerable(message) && (
          <QueueAction
            label={COPY.addNowLabel}
            tooltip={editing ? COPY.cannotSendWhileEditing : steerTooltip(isNext)}
            keyShortcuts={isNext ? getSteerAriaKeyShortcuts() : undefined}
            disabled={editing}
            onClick={(e) => {
              e.stopPropagation();
              onSteerMessage?.(message.id);
            }}
          >
            <MessageSquarePlus className="size-3.5" aria-hidden />
          </QueueAction>
        )}
        {onStopAndSend && (
          <QueueAction
            label={COPY.stopAndSendLabel}
            tooltip={editing ? COPY.cannotSendWhileEditing : COPY.stopAndSendTooltip}
            disabled={editing}
            onClick={(e) => {
              e.stopPropagation();
              onStopAndSend(message.id);
            }}
          >
            <ArrowUp className="size-3.5" aria-hidden />
          </QueueAction>
        )}
      </>
    );
  };

  const countLabel = `${isPaused ? COPY.paused : COPY.header} · ${queuedMessages.length}`;

  if (!isExpanded) {
    const nextMessage = queuedMessages[0];
    return (
      <div className={cn('br-queue', className)} data-testid="message-queue">
        {/* The whole line unfolds the queue for a pointer; the chevron is the
            keyboard's way in, and the name tests and screen readers use. */}
        <div className="br-queue-line" onClick={() => setIsExpanded(true)}>
          <span className="br-queue-count">{countLabel}</span>
          <span className="br-queue-arrow" aria-hidden>
            ↳
          </span>
          <p className="br-queue-text">{messageText(nextMessage)}</p>
          <div className="br-queue-actions" data-visible="">
            {turnActions(nextMessage, true)}
            <QueueAction
              label={COPY.expand(queuedMessages.length)}
              tooltip={COMPOSER_COPY.queue.header}
              onClick={(e) => {
                e.stopPropagation();
                setIsExpanded(true);
              }}
            >
              <ChevronDown className="size-3.5" aria-hidden />
            </QueueAction>
          </div>
        </div>
        {/* SD-8: the reason the "Add now" above is missing, in the row it is
            missing from. Mounted on `steerApplies` alone: the note itself
            renders nothing on the desktop, so this condition is "is there a
            turn to steer", not "which surface is this". */}
        {steerApplies && <SteerUnavailableNote short />}
      </div>
    );
  }

  return (
    <div className={cn('br-queue', className)} data-testid="message-queue">
      <div className="br-queue-line">
        <span className="br-queue-count">{countLabel}</span>
        <span className="flex-1" />
        <div className="br-queue-actions" data-visible="">
          {queuedMessages.length > 1 && (
            <Button
              type="button"
              variant="ghost"
              size="xs"
              onClick={onClearQueue}
              className="text-supporting text-text-muted hover:text-text-danger"
            >
              {COPY.clear}
            </Button>
          )}
          <QueueAction label={COPY.collapse} onClick={() => setIsExpanded(false)}>
            <ChevronUp className="size-3.5" aria-hidden />
          </QueueAction>
        </div>
      </div>

      <ul className="br-queue-list">
        {queuedMessages.map((message, index) => (
          <li
            key={message.id}
            className="br-queue-row"
            data-dragging={draggedItem === message.id ? '' : undefined}
            data-drag-over={
              dragOverItem === message.id && draggedItem !== message.id ? '' : undefined
            }
            draggable={onReorderMessages ? true : false}
            onDragStart={(e) => handleDragStart(e, message.id)}
            onDragOver={(e) => handleDragOver(e, message.id)}
            onDragLeave={handleDragLeave}
            onDrop={(e) => handleDrop(e, message.id)}
            onDragEnd={handleDragEnd}
          >
            {onReorderMessages ? (
              <span className="br-queue-grip" aria-label={COPY.dragToReorder} role="img">
                <GripVertical className="size-3.5" />
              </span>
            ) : (
              <span className="br-queue-arrow" aria-hidden>
                ↳
              </span>
            )}

            {editingMessage === message.id ? (
              <div className="flex min-w-0 flex-1 flex-col gap-1.5 py-1">
                <textarea
                  value={splitComposerText(editContent).body}
                  onChange={(e) =>
                    setEditContent(
                      joinComposerText(e.target.value, splitComposerText(editContent).refs)
                    )
                  }
                  className="w-full resize-none rounded-element border border-border-subtle bg-background-default px-2 py-1 text-secondary focus:border-border-strong"
                  rows={Math.max(1, Math.min(Math.ceil(editContent.length / 60), 4))}
                  autoFocus
                />
                <div className="flex gap-1">
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    className="text-supporting"
                    onClick={() => {
                      onEditMessage?.(message.id, editContent);
                      endEditing();
                    }}
                  >
                    {COPY.save}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="xs"
                    className="text-supporting"
                    onClick={endEditing}
                  >
                    {COPY.cancel}
                  </Button>
                </div>
              </div>
            ) : (
              // The text is the edit control: a button, so the keyboard reaches
              // it, named by what it holds.
              <button
                type="button"
                className="br-queue-text br-queue-edit"
                onClick={() => startEditing(message)}
              >
                {messageText(message)}
              </button>
            )}

            <div className="br-queue-actions">
              {turnActions(message, index === 0)}
              <QueueAction label={COPY.remove} danger onClick={() => onRemoveMessage(message.id)}>
                <X className="size-3.5" aria-hidden />
              </QueueAction>
            </div>
          </li>
        ))}
      </ul>
      {/* SD-8, once for the whole list rather than once per row: the reason is
          the daemon's, not this message's, so repeating it under every row
          would say one true thing N times. */}
      {steerApplies && <SteerUnavailableNote />}
    </div>
  );
};

export default MessageQueue;
