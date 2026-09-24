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
import { ArrowUp, Bot, Loader2, X } from '../../icons/app-icons';
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
   * layout decides which. The composer's answers to what the person just did (a send or upload
   * failure, the picker a drop opened) outrank it, and the upload ones never linger: see
   * {@link composerNote}.
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
  const ownInput = useRef<HTMLTextAreaElement | null>(null);
  const setInput = useCallback(
    (node: HTMLTextAreaElement | null) => {
      ownInput.current = node;
      assignRef(inputRef, node);
    },
    [inputRef]
  );

  // An upload failure answers one action and stops being news when the person moves on: any
  // edit to the draft (typing, removing a chip, a finished upload landing in it) clears it, so
  // it cannot keep the layout's note (and its action) out of the slot. Keyed on the draft's
  // content rather than its object, which a controller may rebuild without changing anything.
  const draftKey = [
    draft.body,
    ...draft.attachments.map((file) => file.id),
    ...draft.references.map((reference) => reference.id),
  ].join('\u0000');
  useEffect(() => {
    latestUpload.current.dismissError();
  }, [draftKey]);
  const sendNow = () => {
    latestUpload.current.dismissError();
    void send();
  };
  const dismissUploadError = () => {
    latestUpload.current.dismissError();
    // The dismiss control leaves with its note: hand focus to the text rather than the page.
    ownInput.current?.focus();
  };

  // Losing the verified view (an observation failure, a disconnect) may clear the draft, so an
  // upload still on its way stops being headed for it: it waits under "Uploaded, not sent".
  // `forget()` also clears an upload failure, which described the view that is gone. Another
  // channel or connection does the same inside `useCrewUpload`. A refresh keeps both.
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

  const composerError = ownsError && error?.source === 'composer' ? error : null;
  const notes = composerNote({
    error: composerError?.message ?? null,
    uploadError: upload.error,
    onDismissUpload: dismissUploadError,
    dropHint,
    note,
  });

  // Verified, but no channel to write in: no card. A failure this surface owns still shows,
  // because an error registered to the composer must render somewhere, exactly once.
  if (!channel && verified)
    return notes ? (
      <div className="crew-compose" data-state="no-channel">
        <div className="crew-compose-column">{notes}</div>
      </div>
    ) : null;

  let body: ReactNode;
  if (!verified) {
    body = (
      <div className="crew-compose-bar crew-compose-bar-verifying" data-testid="crew-verifying">
        <Loader2 className="size-4 animate-spin" aria-hidden />
        <span>{composerCopy.verifying}</span>
      </div>
    );
  } else if (archived) {
    body = (
      <div className="crew-compose-bar crew-compose-bar-archived">{composerCopy.archived}</div>
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
        onSend={sendNow}
        posting={isPending('send')}
        onAskAgent={() => openPane({ mode: 'agent' })}
        onSharePath={() => openDialog({ kind: 'share-path' })}
        onPasteFiles={(files) => void takeFiles({ files, hasFolder: false })}
        inputRef={setInput}
      />
    );
  }

  const content = (
    <div
      className="crew-compose"
      data-state={!verified ? 'verifying' : archived ? 'archived' : 'ready'}
    >
      <div className="crew-compose-column">
        {notes}
        {body}
      </div>
    </div>
  );
  if (enclosed) return content;
  return (
    <CrewFileDropZone target={dropTarget} className="crew-compose-drop">
      {content}
    </CrewFileDropZone>
  );
}

/**
 * The one note directly above the card, in priority order: the send failure, an upload
 * failure, the picker a drop just opened, then the layout's standing note. One at a time, so
 * nothing stacks above the composer.
 *
 * The upload failure sits above the layout's note, beside the send failure, for the same
 * reason the spec puts the send failure first: it is the answer to what the person just did.
 * Below the note it would never show at all where it matters most, since the chat-connect note
 * stands in every grant state for as long as the route carries `?sessionId=`: a refused
 * folder, a pasted screenshot or the pinned privacy refusal would each look like a press that
 * did nothing. What keeps it from hiding the note is that it never lingers. It has its own
 * dismiss control, and it clears when the person edits the draft or sends, when the verified
 * privacy mode or scope changes, on a protected-state reset, and on another channel. The drop
 * hint lasts only while the picker it names is open.
 */
function composerNote({
  error,
  uploadError,
  onDismissUpload,
  dropHint,
  note,
}: {
  error: string | null;
  uploadError: string;
  onDismissUpload(): void;
  dropHint: string;
  note: ReactNode;
}): ReactNode {
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
      <Note
        tone="danger"
        role="alert"
        action={
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                shape="round"
                size="xs"
                aria-label={composerCopy.dismissUploadError}
                className="size-5"
                onClick={onDismissUpload}
              >
                <X aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{composerCopy.dismissUploadError}</TooltipContent>
          </Tooltip>
        }
      >
        {uploadError}
      </Note>
    );
  } else if (dropHint) {
    content = <Note role="status">{dropHint}</Note>;
  } else if (note) {
    content = note;
  }
  return content ? <div className="crew-compose-note">{content}</div> : null;
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
  /**
   * A pasted file goes to the upload path; pasted words stay words. Copying cells or rich text
   * from another app often puts a rendered picture of it on the clipboard beside the text, with
   * no file behind the picture, so the text wins unless a pasted item is a file on this
   * computer (a file copied in the Finder or Explorer).
   */
  const onPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(event.clipboardData?.files ?? []);
    if (files.length === 0) return;
    const onDisk = files.filter((file) => Boolean(localPath(file)));
    const text = event.clipboardData?.getData?.('text/plain') ?? '';
    if (onDisk.length === 0 && text.trim()) return;
    event.preventDefault();
    onPasteFiles(onDisk.length > 0 ? onDisk : files);
  };

  return (
    <div
      className={cn(
        'biorouter-composer-card crew-compose-card relative flex min-w-0 flex-col rounded-container',
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
        className="crew-compose-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
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
      <div className="crew-compose-controls">
        <AttachMenu
          onUpload={() => void upload.upload()}
          onSharePath={onSharePath}
          uploading={upload.choosing}
        />
        <Button type="button" variant="ghost" size="sm" onClick={onAskAgent}>
          <Bot aria-hidden />
          {composerCopy.askAgent}
        </Button>
        <span className="crew-compose-spacer" />
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="crew-compose-send">
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
          <TooltipContent>{posting ? composerCopy.sending : composerCopy.send}</TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}
