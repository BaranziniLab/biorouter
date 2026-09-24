import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from 'react';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { ArrowUp, Bot, Loader2 } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { channelSlug } from '../identity';
import { crewActionCopy } from '../state/copy';
import type { DraftFile, DraftReference } from '../state/types';
import { useCrew, useCrewErrorSlot, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { filesCopy } from '../files/copy';
import {
  CrewFileDropZone,
  useCrewDropTarget,
  type CrewDropTarget,
  type DroppedFiles,
} from '../files/FileDropZone';
import { useCrewUpload } from '../files/useCrewUpload';
import { AttachMenu } from './AttachMenu';
import { ComposerChips } from './ComposerChips';
import { composerCopy } from './copy';
import './composer.css';

/** The daemon's attachment limit (`local_files::MAX_SIZE`): 1 GiB. */
export const CREW_ATTACHMENT_LIMIT = 1024 * 1024 * 1024;

/**
 * The preload's path lookup, when this surface has one. `''` means the `File` has no file
 * behind it (a pasted screenshot, or a browser surface that cannot know).
 */
function localPath(file: File): string | undefined {
  const lookup = (window as { electron?: { getPathForFile?: (file: File) => string } }).electron
    ?.getPathForFile;
  if (typeof lookup !== 'function') return undefined;
  try {
    return lookup(file);
  } catch {
    return undefined;
  }
}

/** True when the main process can open the secure Crew picker here. */
function hasSecurePicker(): boolean {
  return (
    typeof (window as { electron?: { crewSelectTransferFile?: unknown } }).electron
      ?.crewSelectTransferFile === 'function'
  );
}

export interface ComposerProps {
  /**
   * The one standing note above the card, when nothing more urgent is showing: the chat-connect
   * note, an ownership offer, the host's institution note, the first-join name suggestion. The
   * layout decides which; the composer's own send and upload failures outrank it.
   */
  note?: ReactNode;
  /** The textarea, for a layout that moves focus here (after joining, after creating a channel). */
  inputRef?: Ref<HTMLTextAreaElement>;
}

/**
 * Crew's composer: BioRouter's chat composer card, class for class, in a 760px column.
 *
 * - The card holds the chips row (only when non-empty), the textarea (`aria-label` "Message
 *   #name", pinned; one row growing to 40vh) and the controls: Attach, Ask my agent, Send.
 * - Enter sends; Shift+Enter, IME composition (`isComposing`, `keyCode 229`) and key repeat do
 *   not. The handler is the legacy composer's, moved verbatim.
 * - Send is `secondary` until there is something to send, then the accent. While posting it
 *   shows the spinner and stays the same node; `send()` itself is single flight (C15).
 * - The send is not optimistic: the draft stays until the broker answers, and a failure is
 *   shown above the card ("Couldn't send." and the daemon's words in their own node) while the
 *   draft and its idempotency key wait for another press.
 * - Without a verified snapshot the card is replaced by a bar of the same height, "Verifying
 *   access…", and the textarea is not mounted (C13); the draft stays in the controller. An
 *   archived channel shows "This channel is archived." instead.
 * - Files: the Attach menu, a drop anywhere on the channel (or on the composer when the layout
 *   gives no wider zone) and a pasted file all go through the one upload path: the secure
 *   main-process picker and the exact legacy `beginTransfer` payload. The renderer never reads
 *   a file or hands a path to anything.
 *
 * React authorizes nothing here: the daemon and broker decide every post and every upload.
 */
export function Composer({ note, inputRef }: ComposerProps) {
  const controller = useCrew();
  const {
    connectionId,
    channelId,
    channel,
    snapshot,
    observedPrivacy,
    draft,
    setBody,
    addAttachment,
    removeAttachment,
    removeReference,
    send,
    isPending,
    error,
    openPane,
    openDialog,
  } = controller;
  const ownsError = useCrewErrorSlot('composer');
  const expectedMode =
    observedPrivacy?.connectionId === connectionId ? observedPrivacy.mode : undefined;
  const upload = useCrewUpload({
    connectionId,
    channelId,
    expectedMode,
    onReady: addAttachment,
  });
  const [dropHint, setDropHint] = useState('');
  const latestUpload = useRef(upload);
  latestUpload.current = upload;

  // Losing the verified view (an observation failure, a disconnect) may clear the draft, so an
  // upload still on its way stops being headed for it: it waits under "Uploaded, not sent".
  // Another channel or connection does the same inside `useCrewUpload`. A refresh keeps it.
  useCrewSurfaceReset((reason) => {
    if (reason === 'protected-cleared') {
      latestUpload.current.forget();
      setDropHint('');
    }
  });

  // The observer clears the draft when the verified privacy scope changes under it (a new
  // connection or workspace policy epoch, or a new mode). An upload started under the old scope
  // must not land in the draft that was cleared for that reason.
  const scope =
    snapshot && expectedMode
      ? [connectionId, expectedMode, observedPrivacy?.policyEpoch, snapshot.workspace.policy_epoch]
          .map(String)
          .join('\n')
      : '';
  const lastScope = useRef('');
  useEffect(() => {
    if (!scope) return;
    if (lastScope.current && lastScope.current !== scope) latestUpload.current.forget();
    lastScope.current = scope;
  }, [scope]);

  const verified = Boolean(snapshot);
  const archived = Boolean(channel?.archived);
  const name = channelSlug(channel);
  const accepting = verified && !archived && channel !== null;

  /**
   * A drop or a paste. The `File` objects are looked at only for what the picker cannot say
   * in advance (a folder, a file over the limit, data with no file behind it); then the one
   * upload path opens the secure picker, with a note naming the file to choose. The picker is
   * the only source of a file capability, so a drop can never share a file by itself.
   */
  const takeFiles = useCallback(async ({ files, hasFolder }: DroppedFiles) => {
    const current = latestUpload.current;
    const [first] = files;
    if (!first || current.choosing) return;
    if (hasFolder) return current.reportError(filesCopy.folderRefused);
    if (first.size > CREW_ATTACHMENT_LIMIT)
      return current.reportError(filesCopy.tooLarge(first.name));
    if (hasSecurePicker() && localPath(first) === '')
      return current.reportError(filesCopy.notSaved);
    setDropHint(
      [filesCopy.chooseInWindow(first.name), files.length > 1 ? filesCopy.oneAtATime : '']
        .filter(Boolean)
        .join(' ')
    );
    try {
      await current.upload();
    } finally {
      setDropHint('');
    }
  }, []);
  const dropTarget = useMemo<CrewDropTarget | null>(
    () => (accepting ? { channelName: name, onFiles: (dropped) => void takeFiles(dropped) } : null),
    [accepting, name, takeFiles]
  );
  const enclosed = useCrewDropTarget(dropTarget);

  if (!channel && verified) return null;

  const composerError = ownsError && error?.source === 'composer' ? error : null;
  const notes = (
    <ComposerNote
      error={composerError?.message ?? null}
      uploadError={upload.error}
      dropHint={dropHint}
      note={note}
    />
  );

  let body: ReactNode;
  if (!verified) {
    body = (
      <div className="crew-composer-bar crew-composer-bar-verifying" data-testid="crew-verifying">
        <Loader2 className="size-4 animate-spin" aria-hidden />
        <span>{composerCopy.verifying}</span>
      </div>
    );
  } else if (archived) {
    body = (
      <div className="crew-composer-bar crew-composer-bar-archived">{composerCopy.archived}</div>
    );
  } else {
    body = (
      <ComposerCard
        name={name}
        body={draft.body}
        setBody={setBody}
        attachments={draft.attachments}
        references={draft.references}
        upload={upload}
        onRemoveAttachment={removeAttachment}
        onRemoveReference={removeReference}
        onSend={() => void send()}
        posting={isPending('send')}
        onAskAgent={() => openPane({ mode: 'agent' })}
        onSharePath={() => openDialog({ kind: 'share-path' })}
        onPasteFiles={(files) => void takeFiles({ files, hasFolder: false })}
        inputRef={inputRef}
      />
    );
  }

  const content = (
    <div
      className="crew-composer"
      data-state={!verified ? 'verifying' : archived ? 'archived' : 'ready'}
    >
      <div className="crew-composer-column">
        {notes}
        {body}
      </div>
    </div>
  );
  if (enclosed) return content;
  return (
    <CrewFileDropZone target={dropTarget} className="crew-composer-drop">
      {content}
    </CrewFileDropZone>
  );
}

/**
 * The one note directly above the card, in priority order: the send failure, an upload
 * failure, the picker a drop just opened, then the layout's standing note. One at a time, so
 * nothing stacks above the composer.
 */
function ComposerNote({
  error,
  uploadError,
  dropHint,
  note,
}: {
  error: string | null;
  uploadError: string;
  dropHint: string;
  note: ReactNode;
}) {
  let content: ReactNode = null;
  if (error === crewActionCopy.sendTransferRecordKept) {
    content = (
      <Note tone="warning" role="status">
        {composerCopy.postedMetadata}
      </Note>
    );
  } else if (error) {
    content = (
      <Note tone="danger" role="alert">
        <span>{composerCopy.sendErrorLead}</span> <span>{error}</span>
      </Note>
    );
  } else if (uploadError) {
    content = (
      <Note tone="danger" role="alert">
        {uploadError}
      </Note>
    );
  } else if (dropHint) {
    content = <Note role="status">{dropHint}</Note>;
  } else if (note) {
    content = note;
  }
  return content ? <div className="crew-composer-note">{content}</div> : null;
}

interface ComposerCardProps {
  name: string;
  body: string;
  setBody(body: string): void;
  attachments: readonly DraftFile[];
  references: readonly DraftReference[];
  upload: ReturnType<typeof useCrewUpload>;
  onRemoveAttachment(id: string): void;
  onRemoveReference(id: string): void;
  onSend(): void;
  posting: boolean;
  onAskAgent(): void;
  onSharePath(): void;
  onPasteFiles(files: File[]): void;
  inputRef?: Ref<HTMLTextAreaElement>;
}

function assignRef<T>(ref: Ref<T> | undefined, value: T | null) {
  if (typeof ref === 'function') ref(value);
  else if (ref) (ref as { current: T | null }).current = value;
}

function ComposerCard({
  name,
  body,
  setBody,
  attachments,
  references,
  upload,
  onRemoveAttachment,
  onRemoveReference,
  onSend,
  posting,
  onAskAgent,
  onSharePath,
  onPasteFiles,
  inputRef,
}: ComposerCardProps) {
  const textarea = useRef<HTMLTextAreaElement | null>(null);
  const setTextarea = useCallback(
    (node: HTMLTextAreaElement | null) => {
      textarea.current = node;
      assignRef(inputRef, node);
    },
    [inputRef]
  );
  const hasContent = Boolean(body.trim()) || attachments.length > 0 || references.length > 0;

  // Auto-grow: one row at rest, up to the stylesheet's 40vh cap, then it scrolls. Not animated
  // (the spec lists composer auto-grow among what must not move). Where there is no layout
  // (jsdom) the measured height is 0, and the inline height is left unset.
  useLayoutEffect(() => {
    const node = textarea.current;
    if (!node) return;
    node.style.height = '0px';
    const measured = node.scrollHeight;
    node.style.height = measured > 0 ? `${measured}px` : '';
  }, [body]);

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (
      e.key !== 'Enter' ||
      e.shiftKey ||
      e.nativeEvent.isComposing ||
      e.nativeEvent.keyCode === 229
    )
      return;
    e.preventDefault();
    if (!e.repeat) onSend();
  };
  const onPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(event.clipboardData?.files ?? []);
    if (files.length === 0) return;
    event.preventDefault();
    onPasteFiles(files);
  };

  return (
    <div
      className={cn(
        'biorouter-composer-card crew-composer-card relative flex min-w-0 flex-col rounded-container',
        'bg-background-default border border-border-subtle/60 py-2.5 pr-3 pl-4'
      )}
    >
      <ComposerChips
        attachments={attachments}
        references={references}
        uploads={upload.chips}
        onRemoveAttachment={onRemoveAttachment}
        onRemoveReference={onRemoveReference}
        onPauseUpload={(transfer) => void upload.pause(transfer)}
        onResumeUpload={(transfer) => void upload.resume(transfer)}
      />
      <textarea
        ref={setTextarea}
        className="crew-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
        aria-label={composerCopy.label(name)}
        placeholder={composerCopy.placeholder(name)}
        rows={1}
        value={body}
        // Read-only, not disabled, while the post is in flight: focus stays here, and nothing
        // typed now can be wiped by the success that clears what was sent.
        readOnly={posting}
        aria-busy={posting || undefined}
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
      />
      <div className="crew-composer-controls">
        <AttachMenu
          onUpload={() => void upload.upload()}
          onSharePath={onSharePath}
          uploading={upload.choosing}
        />
        <Button type="button" variant="ghost" size="sm" onClick={onAskAgent}>
          <Bot aria-hidden />
          {composerCopy.askAgent}
        </Button>
        <span className="crew-composer-spacer" />
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="crew-composer-send">
              <Button
                type="button"
                shape="round"
                variant={hasContent ? 'default' : 'secondary'}
                data-variant={hasContent ? 'default' : 'secondary'}
                aria-label={composerCopy.send}
                aria-busy={posting || undefined}
                disabled={!hasContent && !posting}
                className={cn(!hasContent && 'cursor-not-allowed disabled:opacity-100')}
                onClick={() => {
                  if (!posting) onSend();
                  // After a send the person keeps typing: focus goes back to the text, not to a
                  // button that is about to be disabled by the emptied draft.
                  textarea.current?.focus();
                }}
              >
                {posting ? (
                  <Loader2 className="size-4 animate-spin" aria-hidden />
                ) : (
                  <ArrowUp className="size-4" aria-hidden />
                )}
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>
            {posting ? composerCopy.sending : composerCopy.sendTooltip}
          </TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}
