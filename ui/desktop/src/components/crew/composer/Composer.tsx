import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type FocusEvent,
  type KeyboardEvent,
  type MutableRefObject,
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
import { POST_NOTE_CODES, postDestination } from '../state/crewSend';
import type { CrewActionError, DraftFile, DraftReference } from '../state/types';
import { useCrew, useCrewErrorSlot, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { postedLabel, useAttachmentIndexVersion } from '../files/attachmentIndex';
import { filesCopy } from '../files/copy';
import { visibleFileText } from '../files/fileName';
import {
  CrewFileDropZone,
  useCrewDropTarget,
  type CrewDropTarget,
  type DroppedFiles,
} from '../files/FileDropZone';
import { canShareDroppedFiles, useCrewUpload, type CrewShareNames } from '../files/useCrewUpload';
import { serverLabel, workspaceTitle } from '../sidebar/sidebarView';
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

/**
 * The folder a local file is in, from its path: display text for the picker note only (Q2-16).
 * Never sent anywhere.
 */
function folderName(path: string | undefined): string {
  if (!path) return '';
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length >= 2 ? parts[parts.length - 2] : '';
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
  /**
   * The element Ask my agent opens, for its `aria-controls`. The layout wraps the agent pane in
   * it; absent, the toggle still says whether it is open (`aria-expanded`).
   */
  agentPaneId?: string;
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
 *   shown above the card ("Couldn't send." and the reason in its own node) while the draft and
 *   its idempotency key wait for another press. The text stays writable while the post is on its
 *   way, and what is typed then stays: success takes out only what was sent (QA M8).
 * - A send failure belongs to the channel and the draft it answered (QA M5): it shows only in
 *   that channel's composer, goes at the draft's next edit, and comes back with the draft when the
 *   person returns to the channel. A post whose outcome is unknown says it is checking, then that
 *   it was sent or could not be confirmed (QA R-4).
 * - Without a verified snapshot the card is replaced by a bar of the same height, "Verifying
 *   access…", and the textarea is not mounted (C13); the draft stays in the controller. An
 *   archived channel shows "This channel is archived." instead.
 * - Files: the Attach menu opens the secure main-process picker. A drop anywhere on the channel
 *   (or on the composer when the layout gives no wider zone) and a pasted file ask for ONE
 *   confirmation in a native Share / Cancel dialog the main process shows (D-DROP): the preload
 *   resolves the dropped `File` itself, and only Share yields the file capability. Both then
 *   start the upload with the exact legacy `beginTransfer` payload. The renderer never reads a
 *   file or hands a path to anything.
 *
 * React authorizes nothing here: the daemon and broker decide every post and every upload.
 */
export function Composer({ note, inputRef, agentPaneId }: ComposerProps) {
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
    closePane,
    openDialog,
    ui,
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
  // Several files dropped at once (DW-18): the note naming the one Crew took, until closed. Unlike
  // an upload failure it does not go with any change to the draft: the file it names lands in the
  // draft when its upload completes, which for a small file is within one transfer poll, and the
  // note went with it. So it goes on the person's own edits (typing, removing a chip), a send, its
  // dismiss control, a new drop or paste, and the surface resets listed below.
  const [extraFiles, setExtraFiles] = useState('');
  const latestUpload = useRef(upload);
  latestUpload.current = upload;
  // What the native share confirmation calls this channel and its workspace: display text only.
  const shareNames = useRef<CrewShareNames>({ channelName: '', workspaceName: '' });
  shareNames.current = {
    channelName: channelSlug(channel),
    workspaceName: workspaceTitle(snapshot, controller.connections, connectionId),
  };
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
  // A send failure answers the draft it was about (QA M5): the draft's next edit takes it away too,
  // as it does an upload failure. Only a failure that was already on show before the edit goes: one
  // that arrives with the draft (the person came back to a channel and its draft brought its
  // failure back) is news.
  const destination = postDestination(connectionId, channelId);
  const shownComposerError = useRef<CrewActionError | null>(null);
  useEffect(() => {
    latestUpload.current.dismissError();
    const shown = shownComposerError.current;
    if (shown) controller.dismissErrorIfShown?.(shown);
  }, [draftKey]); // eslint-disable-line react-hooks/exhaustive-deps -- runs per edit of the draft only
  const sendNow = () => {
    latestUpload.current.dismissError();
    setExtraFiles('');
    void send();
  };
  // The person's own edits: what the note about several files goes with (an upload landing is not).
  const editBody = (value: string) => {
    setExtraFiles('');
    setBody(value);
  };
  const removeOwnAttachment = (id: string) => {
    setExtraFiles('');
    removeAttachment(id);
  };
  const removeOwnReference = (id: string) => {
    setExtraFiles('');
    removeReference(id);
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
    if (
      reason === 'protected-cleared' ||
      reason === 'channel-changed' ||
      reason === 'channel-revoked'
    )
      setExtraFiles('');
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
   * A drop or a paste. The `File` objects are looked at only for what can be said at once (a
   * folder, a file over the limit, data with no file behind it); then the native share
   * confirmation opens for the first file (D-DROP), with a note naming it. The main process
   * decides everything again: it refuses what cannot be shared, and only its Share gives the file
   * capability, so a drop never shares a file by itself. A desktop build without that
   * confirmation opens the secure picker instead, with a note that says what to pick there. While
   * either is already open, a drop or paste opens nothing and the note says to finish it.
   */
  const takeFiles = useCallback(async ({ files, hasFolder }: DroppedFiles) => {
    const current = latestUpload.current;
    const [first] = files;
    if (!first) return;
    // One window at a time. The one already open is the one to finish; a drop or paste now opens
    // nothing, and says so instead of doing nothing.
    if (current.choosing)
      return setDropHint(
        current.confirming ? filesCopy.finishConfirming : filesCopy.finishChoosing
      );
    if (hasFolder) return current.reportError(filesCopy.folderRefused);
    if (first.size > CREW_ATTACHMENT_LIMIT)
      return current.reportError(filesCopy.tooLarge(first.name));
    const confirm = canShareDroppedFiles();
    const path = localPath(first);
    if ((confirm || hasSecurePicker()) && path === '')
      return current.reportError(filesCopy.notSaved);
    const more = files.length > 1 ? filesCopy.oneAtATime : '';
    // The dialog names the file itself — a dropped shortcut by its target — so this names none.
    const hint = confirm
      ? filesCopy.confirmShare
      : filesCopy.chooseInWindow(first.name, folderName(path));
    setDropHint([hint, more].filter(Boolean).join(' '));
    setExtraFiles('');
    // The one file of several that Crew takes is named, once its upload has started, by the name
    // the upload has: the file the dialog confirmed, or the one picked in the window.
    if (files.length > 1) awaitingUpload.current = new Set(current.uploads.map((item) => item.id));
    try {
      if (confirm) await current.shareFile(first, shareNames.current);
      else await current.upload();
    } finally {
      setDropHint('');
    }
  }, []);
  // Several files dropped at once: when the upload of the one Crew took has started, say so in a
  // note the person closes (DW-18). Nothing started (Cancel, a refusal): nothing to say.
  const awaitingUpload = useRef<Set<string> | null>(null);
  useEffect(() => {
    const before = awaitingUpload.current;
    if (!before) return;
    const started = upload.uploads.find((item) => !before.has(item.id));
    if (started) {
      awaitingUpload.current = null;
      setExtraFiles(composerCopy.oneFileAtATime(visibleFileText(started.name)));
    } else if (!upload.choosing) awaitingUpload.current = null;
  }, [upload.uploads, upload.choosing]);
  // A note about the file window lasts only while one is open: however it was opened (the
  // Attach menu, a drop, a paste), closing it clears the note.
  useEffect(() => {
    if (!upload.choosing) setDropHint('');
  }, [upload.choosing]);
  const dropTarget = useMemo<CrewDropTarget | null>(
    () => (accepting ? { channelName: name, onFiles: (dropped) => void takeFiles(dropped) } : null),
    [accepting, name, takeFiles]
  );
  const enclosed = useCrewDropTarget(dropTarget);

  // A draft file that is already in the channel, by name or by contents once its upload's
  // checksum is known: a note under its chip, never a question (Q3-13). It says which it is once
  // both checksums are known — the same bytes, or a corrected file under the same name — rather
  // than one sentence for both (Q4-18). Read from the channel view's attachment index, which the
  // loaded messages' cards fill.
  const [attachmentIndex] = useAttachmentIndexVersion();
  const duplicates: Record<string, string> = {};
  if (attachmentIndex) {
    for (const file of draft.attachments) {
      const sha256 = upload.uploads.find((item) => item.blob_id === file.id)?.sha256 ?? '';
      // Names as the cards show them (RENDERER-1): the index holds shown names, so a name is
      // compared, and quoted, with its hidden characters made visible.
      const fileName = visibleFileText(file.name);
      const match = attachmentIndex.compare(fileName, sha256, file.id);
      if (!match) continue;
      const when = postedLabel(match.earlier.postedAt);
      duplicates[file.id] =
        match.kind === 'same'
          ? composerCopy.sameFileShared(fileName, match.earlier.name, name, when)
          : match.kind === 'different'
            ? composerCopy.differentFileShared(fileName, name, when)
            : composerCopy.alreadyShared(fileName, name, when);
    }
  }
  const agentOpen = ui.pane?.mode === 'agent';

  // Another channel's failure never shows here, nor comes along into it (QA M5).
  const composerError =
    ownsError &&
    error?.source === 'composer' &&
    (error.destination === undefined || error.destination === destination)
      ? error
      : null;
  useEffect(() => {
    shownComposerError.current = composerError;
  });
  const notes = composerNote({
    error: composerError,
    uploadError: upload.error,
    onDismissUpload: dismissUploadError,
    extraFiles,
    onDismissExtraFiles: () => {
      setExtraFiles('');
      ownInput.current?.focus();
    },
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
        setBody={editBody}
        attachments={draft.attachments}
        references={draft.references}
        upload={upload}
        duplicates={duplicates}
        server={serverLabel(controller.connection)}
        onRemoveAttachment={removeOwnAttachment}
        onRemoveReference={removeOwnReference}
        onSend={sendNow}
        posting={isPending('send')}
        agentOpen={agentOpen}
        agentPaneId={agentPaneId}
        onAskAgent={() => (agentOpen ? closePane() : openPane({ mode: 'agent' }))}
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

/** The dismiss control a note the person closes carries: one word, as every other (Q2-61). */
function DismissNote({ onDismiss }: { onDismiss(): void }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          shape="round"
          size="xs"
          aria-label={composerCopy.dismissUploadError}
          className="size-5"
          onClick={onDismiss}
        >
          <X aria-hidden />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{composerCopy.dismissUploadError}</TooltipContent>
    </Tooltip>
  );
}

/** The send's own answer above the card: a failure, or what became of a post in doubt. */
function sendNote(error: CrewActionError): ReactNode {
  if (error.message === crewActionCopy.sendTransferRecordKept)
    return (
      <Note tone="warning" role="status">
        {composerCopy.postedMetadata}
      </Note>
    );
  switch (error.code) {
    case POST_NOTE_CODES.checking:
      return (
        <Note tone="neutral" role="status">
          {error.message}
        </Note>
      );
    case POST_NOTE_CODES.confirmed:
      return (
        <Note tone="success" role="status">
          {error.message}
        </Note>
      );
    case POST_NOTE_CODES.unconfirmed:
      return (
        <Note tone="warning" role="alert">
          {error.message}
        </Note>
      );
    default:
      return (
        <Note tone="danger" role="alert">
          <span>{composerCopy.sendErrorLead}</span> <span>{error.message}</span>
        </Note>
      );
  }
}

/**
 * The one note directly above the card, in priority order: the send's answer, an upload failure,
 * the note about several dropped files, the picker a drop just opened, then the layout's standing
 * note. One at a time, so nothing stacks above the composer.
 *
 * The upload failure sits above the layout's note, beside the send failure, for the same
 * reason the spec puts the send failure first: it is the answer to what the person just did.
 * Below the note it would never show at all where it matters most, since the chat-connect note
 * stands in every grant state for as long as the route carries `?sessionId=`: a refused
 * folder, a pasted screenshot or the pinned privacy refusal would each look like a press that
 * did nothing. What keeps it from hiding the note is that it never lingers. It has its own
 * dismiss control, and it clears when the person edits the draft or sends, when the verified
 * privacy mode or scope changes, on a protected-state reset, and on another channel. The drop
 * hint lasts only while the picker it names is open. The note about several files names the
 * one Crew took, in red with its own dismiss control as the manual says (DW-18). It goes the
 * same ways as an upload failure except one: the file it names landing in the draft is not an
 * edit by the person, so that leaves it standing.
 */
function composerNote({
  error,
  uploadError,
  onDismissUpload,
  extraFiles,
  onDismissExtraFiles,
  dropHint,
  note,
}: {
  error: CrewActionError | null;
  uploadError: string;
  onDismissUpload(): void;
  extraFiles: string;
  onDismissExtraFiles(): void;
  dropHint: string;
  note: ReactNode;
}): ReactNode {
  let content: ReactNode = null;
  if (error) {
    content = sendNote(error);
  } else if (uploadError) {
    content = (
      <Note tone="danger" role="alert" action={<DismissNote onDismiss={onDismissUpload} />}>
        {uploadError}
      </Note>
    );
  } else if (extraFiles) {
    content = (
      <Note tone="danger" role="alert" action={<DismissNote onDismiss={onDismissExtraFiles} />}>
        {extraFiles}
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
  duplicates: Readonly<Record<string, string>>;
  server: string;
  onRemoveAttachment(id: string): void;
  onRemoveReference(id: string): void;
  onSend(): void;
  posting: boolean;
  agentOpen: boolean;
  agentPaneId?: string;
  onAskAgent(): void;
  onSharePath(): void;
  onPasteFiles(files: File[]): void;
  inputRef?: Ref<HTMLTextAreaElement>;
}

function assignRef<T>(ref: Ref<T> | undefined, value: T | null) {
  if (typeof ref === 'function') ref(value);
  else if (ref) (ref as { current: T | null }).current = value;
}

/**
 * The caret of a draft put back into the composer (QA M9): at its end, so typing adds to it.
 *
 * Choosing a channel in the sidebar by mouse leaves Chromium's frame selection inside the text
 * box's editor. When a kept draft is then written into the box, that selection collapses to the
 * start and Chromium copies it into the box's own selection a moment later, so a focus by Tab or by
 * script put the caret before the draft and the words typed went in front of it. The value the
 * person typed never moves the caret; any other value (a draft put back, a send taking out what was
 * sent) puts it at the end, clears the stale document selection that would undo it, and a focus
 * that is not the person's pointer puts it there again if it was reset to the start meanwhile.
 */
export function useRestoredDraftCaret(
  textarea: MutableRefObject<HTMLTextAreaElement | null>,
  body: string
) {
  /** The value the person last typed: a body equal to it came through `onChange`. */
  const typedValue = useRef<string | null>(null);
  /** A value was put in by the app and the person has not placed the caret since. */
  const placed = useRef(false);
  useLayoutEffect(() => {
    const node = textarea.current;
    if (!node || body === typedValue.current) return;
    typedValue.current = null;
    if (!body) {
      placed.current = false;
      return;
    }
    placed.current = true;
    const selection = typeof document.getSelection === 'function' ? document.getSelection() : null;
    const card = node.parentElement;
    if (
      selection &&
      document.activeElement !== node &&
      selection.anchorNode &&
      (node.contains(selection.anchorNode) || card?.contains(selection.anchorNode))
    )
      selection.removeAllRanges();
    node.setSelectionRange(body.length, body.length);
  }, [textarea, body]);
  return {
    typed: (value: string) => {
      typedValue.current = value;
      placed.current = false;
    },
    touched: () => {
      placed.current = false;
    },
    focused: (event: FocusEvent<HTMLTextAreaElement>) => {
      const node = event.currentTarget;
      if (!placed.current) return;
      placed.current = false;
      const end = node.value.length;
      if (end > 0 && node.selectionStart === 0 && node.selectionEnd === 0)
        node.setSelectionRange(end, end);
    },
  };
}

function ComposerCard({
  name,
  body,
  setBody,
  attachments,
  references,
  upload,
  duplicates,
  server,
  onRemoveAttachment,
  onRemoveReference,
  onSend,
  posting,
  agentOpen,
  agentPaneId,
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

  const caret = useRestoredDraftCaret(textarea, body);

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
      // A post on its way: the card says so, and Send shows its spinner. The text stays writable.
      aria-busy={posting || undefined}
      data-posting={posting ? 'true' : undefined}
    >
      <ComposerChips
        attachments={attachments}
        references={references}
        uploads={upload.chips}
        duplicates={duplicates}
        server={server}
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
        // Laid out in the direction of what is typed (QA M13): Hebrew or Arabic runs right to left.
        dir="auto"
        value={body}
        // Writable while a post is on its way (QA M8): what is typed now stays, because success
        // takes out only what was sent.
        onChange={(event) => {
          caret.typed(event.target.value);
          setBody(event.target.value);
        }}
        onKeyDown={(event) => {
          caret.touched();
          onKeyDown(event);
        }}
        onPointerDown={caret.touched}
        onFocus={caret.focused}
        onPaste={onPaste}
      />
      <div className="crew-compose-controls">
        <AttachMenu
          onUpload={() => void upload.upload()}
          onSharePath={onSharePath}
          uploading={upload.choosing}
        />
        {/* A toggle for the agent pane: it says whether the pane is open, and which element it
            opens, as the channel header's details toggle says it is pressed (Q3-23). */}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-expanded={agentOpen}
          aria-controls={agentPaneId}
          onClick={onAskAgent}
        >
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
