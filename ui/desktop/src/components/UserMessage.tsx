import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import ImagePreview from './ImagePreview';
import { InlineImage } from './InlineImage';
import { extractImagePaths, removeImagePathsFromText } from '../utils/imageUtils';
import { getTextContent } from '../types/message';
import { Message } from '../api';
import MessageCopyLink from './MessageCopyLink';
import { MessageMeta, MessageMetaAction } from './MessageMeta';
import { ProvenanceChip } from './ProvenanceChip';
import { formatMessageTimestamp } from '../utils/timeUtils';
import { ChevronDown, ChevronUp, Edit, Send } from './icons/app-icons';
import { Button } from './ui/button';
import { ResourceRefChip, ResourceRefText } from './ResourceRefChip';
import { joinComposerText, removeComposerRefAt, splitComposerText } from '../utils/composerRefs';
import { CLAMP_EXPAND_MS, CLAMP_MAX_HEIGHT_PX, describeMessageLength } from '../utils/messageClamp';
import { cn } from '../utils';

/**
 * The long-message clamp has three states, not two, because the expansion has
 * to animate from a height we know to a height we do not:
 *
 *   collapsed  capped at CLAMP_MAX_HEIGHT_PX, clipped, the fade drawn
 *   expanding  cap pinned to the measured content height, so the growth
 *              animates over --dur-med; still clipped, so the growth is visible
 *   open       no cap at all — a 5000-line paste cannot be bounded by any
 *              literal, so the cap is REMOVED rather than raised. The design
 *              specimen's `max-height: 1200px` would silently clip anything
 *              taller than 1200px, which is precisely the message this feature
 *              exists for.
 *
 * Collapsing skips `expanding` entirely: the design specifies collapse as
 * instant, "because you are moving away from it".
 */
type ClampState = 'collapsed' | 'expanding' | 'open';

interface UserMessageProps {
  message: Message;
  onMessageUpdate?: (messageId: string, newContent: string, editType?: 'diverge' | 'edit') => void;
  /**
   * D4: send this message's text as a new message. Offered only for a steer
   * the daemon stored as unanswered, and only while no turn is running —
   * absent otherwise, which hides the control.
   */
  onSendAgain?: (text: string) => void;
  deliveryUnconfirmed?: boolean;
}

export default function UserMessage({
  message,
  onMessageUpdate,
  onSendAgain,
  deliveryUnconfirmed = false,
}: UserMessageProps) {
  const contentRef = useRef<HTMLDivElement | null>(null);
  const bubbleRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [editContent, setEditContent] = useState('');
  const [hasBeenEdited, setHasBeenEdited] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Sticky per message, and component-local on purpose. The transcript keys
  // every message by `message.id` (`ProgressiveMessageList`), so this survives
  // every re-render the list does — including one per streamed token of the
  // reply below it, which is exactly the "never re-collapses while you are
  // reading" the design asks for. It does NOT survive leaving the session and
  // coming back, and should not: a module-level registry of expanded ids would
  // be an unbounded cache that also decides, on your behalf, that a chat you
  // reopened tomorrow should start unfolded.
  const [clampState, setClampState] = useState<ClampState>('collapsed');
  const [expandedHeight, setExpandedHeight] = useState<number | null>(null);
  const clampBodyId = useId();

  // Extract text content from the message
  const textContent = getTextContent(message);

  // Extract image paths from the message
  const imagePaths = extractImagePaths(textContent);

  // Extract structured ImageContent blocks (new sessions only; pre-v-next sessions have no image blocks)
  const imageContentBlocks = useMemo(
    () => message.content.filter((c) => c.type === 'image'),
    [message.content]
  );

  // Remove image paths and injected context blocks from display text
  const displayText = useMemo(
    () =>
      removeImagePathsFromText(textContent, imagePaths)
        .replace(/<info-msg>[\s\S]*?<\/info-msg>(\s*\\?")?/gi, '')
        .trim(),
    [textContent, imagePaths]
  );

  // Issue #65 — the edit box is a second composer, so it follows the same rule:
  // prose in the textarea, references as chips. Without this, "Edit" would be
  // the one place the `<biorouter-ref …>` markup still leaks, and a user tidying
  // their sentence would delete half a tag and silently lose the reference.
  const editRefs = useMemo(() => splitComposerText(editContent).refs, [editContent]);
  const editBody = useMemo(() => splitComposerText(editContent).body, [editContent]);

  // Memoize the timestamp
  const timestamp = useMemo(() => formatMessageTimestamp(message.created), [message.created]);

  // `?? undefined` is load-bearing: the generated field is `MessageProvenance | null`
  // and the chip's prop is optional (`?:`), which TypeScript does not unify.
  const provenance = message.metadata?.provenance ?? undefined;

  // Length alone decides the clamp; the content's shape decides only whether the
  // count is stated in lines and bytes or in words. All of it is pure — see
  // `utils/messageClamp.ts`.
  const { shouldClamp, label: lengthLabel } = useMemo(
    () => describeMessageLength(displayText),
    [displayText]
  );
  const isOpen = clampState !== 'collapsed';
  // `expanding` still clips: the cap is animating, so the fade has to stay until
  // it lands or the bottom line pops into view before the bubble reaches it.
  const isClamped = shouldClamp && clampState !== 'open';

  const toggleClamp = useCallback(() => {
    if (clampState !== 'collapsed') {
      setClampState('collapsed');
      return;
    }
    // Measured here, from the live DOM, rather than in an effect: `scrollHeight`
    // reports the full unclipped height *while* the element is still clipped,
    // which is both the number the animation needs and a number that stops
    // existing the moment the cap comes off.
    const measured = bubbleRef.current?.scrollHeight ?? 0;
    if (measured > 0) {
      setExpandedHeight(measured);
      setClampState('expanding');
    } else {
      // No layout to measure (a hidden pane, or a test renderer that does not
      // lay out). Open directly rather than pinning the bubble to a height of
      // zero, which would hide the message the user just asked to see.
      setClampState('open');
    }
  }, [clampState]);

  useEffect(() => {
    if (clampState !== 'expanding') return;
    // A timer rather than `transitionend`, because `transitionend` is not
    // guaranteed to arrive: the global `prefers-reduced-motion` reset in
    // `main.css` cuts every transition to 0.01ms, and a hidden pane never runs
    // one at all. A message left pinned at a stale pixel height would clip, so
    // the release must not depend on an event. The swap itself is a visual
    // no-op — the element is already at the height it is being released to.
    const timer = window.setTimeout(() => setClampState('open'), CLAMP_EXPAND_MS);
    return () => window.clearTimeout(timer);
  }, [clampState]);

  // Effect to handle message content changes and ensure persistence
  useEffect(() => {
    // If we're not editing, update the edit content to match the current message
    if (!isEditing) {
      setEditContent(displayText);
    }
  }, [message.content, displayText, message.id, isEditing]);

  // Initialize edit mode with current message content
  const initializeEditMode = useCallback(() => {
    setEditContent(displayText);
    setError(null);
    window.electron.logInfo(`Entering edit mode with content: ${displayText}`);
  }, [displayText]);

  // Handle edit button click
  const handleEditClick = useCallback(() => {
    const newEditingState = !isEditing;
    setIsEditing(newEditingState);

    // Initialize edit content when entering edit mode
    if (newEditingState) {
      initializeEditMode();
      window.electron.logInfo(`Edit interface shown for message: ${message.id}`);

      // Focus the textarea after a brief delay to ensure it's rendered
      setTimeout(() => {
        if (textareaRef.current) {
          textareaRef.current.focus();
          textareaRef.current.setSelectionRange(
            textareaRef.current.value.length,
            textareaRef.current.value.length
          );
        }
      }, 50);
    }

    window.electron.logInfo(`Edit state toggled: ${newEditingState} for message: ${message.id}`);
  }, [isEditing, initializeEditMode, message.id]);

  // Handle content changes in edit mode
  const handleContentChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      // The textarea holds the prose; the references ride along untouched.
      const newContent = joinComposerText(e.target.value, editRefs);
      setEditContent(newContent);
      setError(null); // Clear any previous errors
      window.electron.logInfo(`Content changed: ${newContent}`);
    },
    [editRefs]
  );

  const handleRemoveEditReference = useCallback((index: number) => {
    setEditContent((current) => removeComposerRefAt(current, index));
  }, []);

  const handleSave = useCallback(
    (editType: 'diverge' | 'edit' = 'diverge') => {
      if (editContent.trim().length === 0) {
        setError('Message cannot be empty');
        return;
      }

      setIsEditing(false);

      if (editContent.trim() === displayText.trim()) {
        return;
      }

      if (onMessageUpdate && message.id) {
        onMessageUpdate(message.id, editContent, editType);
        setHasBeenEdited(true);
      }
    },
    [editContent, displayText, onMessageUpdate, message.id]
  );

  // Handle cancel action
  const handleCancel = useCallback(() => {
    window.electron.logInfo('Cancel clicked - reverting to original content');
    setIsEditing(false);
    setEditContent(displayText); // Reset to original content
    setError(null);
  }, [displayText]);

  // Handle keyboard events for accessibility
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      window.electron.logInfo(
        `Key pressed: ${e.key}, metaKey: ${e.metaKey}, ctrlKey: ${e.ctrlKey}`
      );

      if (e.key === 'Escape') {
        e.preventDefault();
        handleCancel();
      } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        window.electron.logInfo('Cmd+Enter detected, calling handleSave');
        handleSave();
      }
    },
    [handleCancel, handleSave]
  );

  // Auto-resize textarea based on content
  useEffect(() => {
    if (textareaRef.current && isEditing) {
      textareaRef.current.style.height = 'auto';
      textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 200)}px`;
    }
  }, [editContent, isEditing]);

  return (
    <div className="br-message-meta-scope w-full mt-[16px] opacity-0 animate-[appear_150ms_var(--ease-out)_forwards]">
      <div className="flex flex-col group">
        {/* BR-71 §5: a message injected from another session is labeled in the
            transcript for as long as it exists — including while it is being
            edited, which is why this sits above the isEditing branch. Ordinary
            same-session messages have no provenance and the chip renders null,
            so the wrapper is only mounted when there is something to say. */}
        {provenance && (
          <div className="flex justify-end mb-1">
            <ProvenanceChip provenance={provenance} />
          </div>
        )}
        {isEditing ? (
          // Truly wide, centered, in-place edit box replacing the bubble
          <div className="w-full max-w-4xl mx-auto bg-background-default text-text-default rounded-container border border-border-subtle p-3 my-2">
            {editRefs.length > 0 && (
              <div
                data-testid="edit-reference-rail"
                className="mb-2 flex flex-wrap items-center gap-1.5"
              >
                {editRefs.map((ref, index) => (
                  <ResourceRefChip
                    key={`${ref.kind}:${ref.value}`}
                    refSpan={ref}
                    onRemove={() => handleRemoveEditReference(index)}
                  />
                ))}
              </div>
            )}
            <textarea
              ref={textareaRef}
              value={editBody}
              onChange={handleContentChange}
              onKeyDown={handleKeyDown}
              className="w-full resize-none bg-transparent text-body text-text-default placeholder:text-text-muted border border-border-emphasized rounded-element p-3 transition-colors focus:border-border-strong"
              style={{
                minHeight: '120px',
                maxHeight: '300px',
                fontFamily: 'inherit',
                wordBreak: 'break-word',
                overflowWrap: 'break-word',
              }}
              placeholder="Edit your message..."
              aria-label="Edit message content"
              aria-describedby={error ? `error-${message.id}` : undefined}
            />
            {/* Error message */}
            {error && (
              <div
                id={`error-${message.id}`}
                className="text-text-danger text-supporting mt-2 mb-2"
                role="alert"
                aria-live="polite"
              >
                {error}
              </div>
            )}
            <div className="flex justify-between items-center gap-3 mt-3">
              <div className="text-supporting text-text-muted min-w-0">
                <span className="font-semibold">Edit in place</span> updates this chat.{' '}
                <span className="font-semibold">Diverge</span> creates a new one.
              </div>
              <div className="flex shrink-0 gap-2">
                <Button onClick={handleCancel} variant="ghost" aria-label="Cancel editing">
                  Cancel
                </Button>
                <Button
                  onClick={() => handleSave('edit')}
                  variant="secondary"
                  aria-label="Edit message in place"
                  title="Update the message in this chat"
                >
                  Edit in place
                </Button>
                <Button
                  onClick={() => handleSave('diverge')}
                  aria-label="Diverge with the edited message"
                  title="Create a new chat from the edited message"
                >
                  Diverge
                </Button>
              </div>
            </div>
          </div>
        ) : (
          // Normal message display
          <div className="message flex justify-end w-full">
            <div className="flex-col max-w-[85%] w-fit min-w-0">
              <div className="flex flex-col group min-w-0">
                {/* The user's turn is tinted only so the eye can find the boundary
                    when scrolling. It is NOT an accent surface — a solid coral block
                    shouts on a canvas whose whole thesis is calm (design.md §4.18).
                    The fill IS the boundary, so the border it also carried was the
                    edge stated twice; it goes, and the radius joins the ladder at
                    `--radius-container` (12px) rather than sitting on Tailwind's
                    stock `rounded-xl`. Padding is the specified 10×14. */}
                {/* The clamp lives on THIS div and not on the text child below,
                    because this is the div that carries the fill. The cut is
                    faded with the bubble's OWN fill token, so it reads as "there
                    is more" rather than as a rendering bug and needs no second
                    colour chosen per theme family — and a gradient drawn on the
                    text child would be fading to a colour that is not behind it.
                    The clipping has to sit on the same element for the same
                    reason: clip the text and the tint keeps its full height,
                    leaving an empty coloured tail under the fade. */}
                <div
                  ref={bubbleRef}
                  id={clampBodyId}
                  data-testid="user-message-bubble"
                  style={
                    isClamped
                      ? {
                          maxHeight:
                            clampState === 'collapsed'
                              ? `${CLAMP_MAX_HEIGHT_PX}px`
                              : expandedHeight !== null
                                ? `${expandedHeight}px`
                                : undefined,
                        }
                      : undefined
                  }
                  className={cn(
                    'flex w-fit max-w-full min-w-0 self-end rounded-container bg-background-medium px-3.5 py-2.5',
                    isClamped && [
                      'relative overflow-hidden',
                      "after:pointer-events-none after:absolute after:inset-x-0 after:bottom-0 after:h-14 after:content-['']",
                      'after:bg-[linear-gradient(to_bottom,transparent,var(--background-medium))]',
                    ],
                    // Only the growth animates. Collapsing re-renders without
                    // this class in the same frame as the smaller cap, so it is
                    // instant, which is what the design asks for.
                    clampState === 'expanding' &&
                      'transition-[max-height] duration-[var(--dur-med)] ease-[var(--ease-out)]'
                  )}
                >
                  {/* min-w-0 is required on this flex item: overflow-wrap:break-word
                      (break-words) prevents *visual* overflow but does NOT reduce the
                      element's intrinsic min-content width, so a long unbroken token
                      (e.g. a comma-separated number list with no spaces) keeps the
                      flex item at full-token width and bleeds past the bubble. min-w-0
                      lets the item shrink so the break can happen. */}
                  {/* A sent message keeps its `<biorouter-ref …>` tags — they
                      are what the agent read, what a reload replays and what an
                      edit re-sends — so the transcript draws them as chips
                      rather than letting the user watch their own message come
                      back as XML. Anything the parser refuses stays visible as
                      the text it is, which is honest: the backend ignored it
                      too. */}
                  <div
                    ref={contentRef}
                    className="min-w-0 text-body text-text-default whitespace-pre-wrap break-words"
                  >
                    <ResourceRefText text={displayText} />
                  </div>
                </div>

                {/* The control sits BELOW the bubble, not inside it: it belongs
                    to the message, not to the text. It carries the count,
                    because the count is the whole point — it is what tells you
                    whether expanding is worth it, and a bare "Show more" does
                    not. Geometry from the design specimen's `.pastefoot`: 6px
                    above, 10px between the two, hugging the same edge the bubble
                    hugs. It is a separate row from `MessageMeta` on purpose —
                    "how big is this" and "when was it sent" are different
                    questions, and running them together reads as one string of
                    metadata. */}
                {shouldClamp && (
                  <div className="mt-1.5 flex h-5 items-center justify-end gap-2.5">
                    <MessageMetaAction
                      onClick={toggleClamp}
                      icon={isOpen ? <ChevronUp /> : <ChevronDown />}
                      aria-expanded={isOpen}
                      aria-controls={clampBodyId}
                    >
                      {isOpen ? 'Show less' : 'Show more'}
                    </MessageMetaAction>
                    <span
                      data-testid="message-clamp-count"
                      className="text-supporting text-text-muted tabular-nums"
                    >
                      {lengthLabel}
                    </span>
                  </div>
                )}

                {/* Render images if any */}
                {imagePaths.length > 0 && (
                  <div className="flex flex-wrap gap-2 mt-2">
                    {imagePaths.map((imagePath, index) => (
                      <ImagePreview key={index} src={imagePath} alt={`Pasted image ${index + 1}`} />
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

                {/* D4: a steer the turn accepted and then ended without reading.
                    Stored so the words are never lost, and drawn as what it is
                    — it used to look exactly like a delivered message, sitting
                    unanswered in the middle of the conversation. */}
                {(deliveryUnconfirmed || message.metadata?.steerOutcome === 'unanswered') && (
                  <div
                    data-testid="steer-unanswered"
                    className="mt-1.5 flex items-center justify-end gap-2.5 text-supporting text-text-muted"
                  >
                    <span>
                      {deliveryUnconfirmed
                        ? 'Delivery unconfirmed. Check the transcript before sending again.'
                        : 'Not answered: the turn ended before the agent read this.'}
                    </span>
                    {onSendAgain && (
                      <MessageMetaAction
                        onClick={() => onSendAgain(displayText)}
                        icon={<Send />}
                        aria-label="Send this message again"
                      >
                        Send again
                      </MessageMetaAction>
                    )}
                  </div>
                )}

                <MessageMeta align="end" timestamp={timestamp}>
                  <MessageMetaAction
                    onClick={handleEditClick}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        handleEditClick();
                      }
                    }}
                    icon={<Edit />}
                    aria-label={`Edit message: ${displayText.substring(0, 50)}${displayText.length > 50 ? '...' : ''}`}
                    aria-expanded={isEditing}
                  >
                    Edit
                  </MessageMetaAction>
                  {/* Copy takes the whole thing, collapsed or not: the clamp is
                      a view state, never a content state. Both payloads are
                      already safe — the plain-text one is `displayText` itself,
                      and the HTML one is cloned from `contentRef`, the text div
                      INSIDE the clipped bubble, which holds every line whatever
                      the cap above it says. */}
                  <MessageCopyLink text={displayText} contentRef={contentRef} />
                </MessageMeta>
              </div>
            </div>
          </div>
        )}

        {/* Edited indicator */}
        {hasBeenEdited && !isEditing && (
          <div className="text-supporting text-text-muted mt-1 text-right">Edited</div>
        )}
      </div>
    </div>
  );
}
