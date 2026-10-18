import {
  useCallback,
  useEffect,
  useId,
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
import type { ProviderDetails } from '../../../api';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { ArrowUp, Bot, Loader2, X } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { channelName, channelNamesAcrossTeams, channelSlug } from '../identity';
import { agentCopy, unknownOutcomeCopy } from '../pane/copy';
import { ModelTierMarks } from '../pane/CrewModelPicker';
import {
  isAffiliationRefusal,
  isPublicModelRefusal,
  knownInstitutions,
  modelDisplay,
  modelRefusalText,
  remoteAccessText,
  taskStartCheck,
  usePanePresentation,
  wideRunChannels,
  workspaceInstitutionLabel,
  type TaskStartCheck,
} from '../pane/presentation';
import { useConfiguredModels } from '../pane/useConfiguredModels';
import { crewActionCopy } from '../state/copy';
import { POST_NOTE_CODES, postDestination } from '../state/crewSend';
import { draftTooLongToKeep } from '../state/draftStash';
import type { AgentPaneSeed, CrewActionError, DraftFile, DraftReference } from '../state/types';
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
import {
  agentNameTaken,
  agentStartState,
  parseAgentInvocation,
  type AgentStartState,
  type SuggestionQuery,
} from './agentInvocation';
import { AttachMenu } from './AttachMenu';
import { ComposerChips } from './ComposerChips';
import {
  buildSuggestions,
  ComposerSuggestions,
  suggestionId,
  useComposerSuggestions,
  type ComposerSuggestion,
} from './ComposerSuggestions';
import { composerCopy } from './copy';
import './composer.css';

/** A draft whose first word is `@` or `/` something: the agent's models may be about to matter. */
const LEADING_SIGIL = /^\s*[@/]/;
const NO_CHANNELS: string[] = [];

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
 * - **`@agent`, `/agent`, `/task`** as the draft's first word start the person's own agent on a
 *   task in this channel (design OW-10, `agentInvocation.ts`), on their model for Crew tasks
 *   (OW-11). Before Enter a consent line inside the card says what Enter will do — the model and its
 *   tier, where the agent posts, what it reads and may do on the server — and Send reads "Start
 *   agent task". Enter is the consent. It starts the task through `askAgent` (Ask my agent's request
 *   exactly), or opens Ask my agent with the words when the pane must decide: no task yet, a model
 *   that would be refused here or none, a start whose outcome is unknown. A draft with files, or
 *   longer than a task may be, starts nothing and the line says why. "Post as a message" posts the
 *   draft as written. Typing `@` or `/` offers the agent and the channel's people in a suggestion
 *   list (`ComposerSuggestions`), which takes Enter only to insert a row.
 *
 * React authorizes nothing here: the daemon and broker decide every post, every upload and every
 * task.
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
    request: controller.request,
  });
  const [dropHint, setDropHint] = useState('');
  // Several files dropped at once (DW-18): the note naming the one Crew took, until closed. Unlike
  // an upload failure it does not go with any change to the draft: the file it names lands in the
  // draft when its upload completes, which for a small file is within one transfer poll, and the
  // note went with it. So it goes on the person's own edits (typing, removing a chip), a send, its
  // dismiss control, a new drop or paste, and the surface resets listed below.
  const [extraFiles, setExtraFiles] = useState('');
  // "Post as a message" (OW-10): this draft posts as written, `@agent` and all, until it no longer
  // reads as a task.
  const [literal, setLiteral] = useState(false);
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
    ) {
      setExtraFiles('');
      // "Post as a message" was said of one channel's draft.
      setLiteral(false);
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

  // `@agent`, `/agent`, `/task` (OW-10): the person's own draft, and nothing else, can start their
  // agent. A message from anyone else is never parsed.
  const { workspace, dir } = usePanePresentation();
  const nameTaken = useMemo(() => agentNameTaken(snapshot), [snapshot]);
  const parsed = useMemo(
    () => parseAgentInvocation(draft.body, { agentNameTaken: nameTaken }),
    [draft.body, nameTaken]
  );
  useEffect(() => {
    if (!parsed) setLiteral(false);
  }, [parsed]);
  const invocation = literal ? null : parsed;
  // The models are read the first time a draft could need them, not on every mount.
  const models = useConfiguredModels({
    optional: true,
    enabled: parsed !== null || LEADING_SIGIL.test(draft.body),
  });
  const taskModel = models.taskDefault;
  const taskProvider = models.providers?.find((item) => item.name === taskModel?.provider);
  const shownModel = taskModel ? modelDisplay(taskModel, taskProvider) : null;
  const shownModelName = shownModel
    ? agentCopy.modelChoice(shownModel.model, shownModel.provider)
    : '';
  // An open workspace whose broker reads wide: `@agent` reads every channel the person is in.
  const wide = useMemo(
    () => wideRunChannels({ snapshot, channel, capabilities: controller.capabilities }),
    [snapshot, channel, controller.capabilities]
  );
  const contextChannels = wide ?? NO_CHANNELS;
  const channelLabels = useMemo(
    () => channelNamesAcrossTeams(snapshot?.channels ?? [], snapshot?.teams ?? []),
    [snapshot]
  );
  // The pane's own checks, on the model the consent line names (OW-10): the two never disagree.
  const check = taskStartCheck({
    provider: taskProvider,
    providers: models.providers,
    connection: controller.connection,
    snapshot,
    channel,
    contextChannels,
    workspace,
    channelLabel: (item) => channelLabels.get(item.id) ?? channelName(item),
    model: shownModel?.model ?? null,
  });
  const agentState: AgentStartState | null = invocation
    ? agentStartState({
        invocation,
        hasFiles: draft.attachments.length > 0 || draft.references.length > 0,
        unknownOutcome: controller.unknownRunDestination !== null,
        modelsKnown: models.providers !== null || models.failure !== null,
        hasModel: taskModel !== null,
        blockText: check.blockText,
      })
    : null;
  const startingTask = isPending('run.start');
  const starting = useRef(false);
  const openAgentPane = (seed: AgentPaneSeed) => openPane({ mode: 'agent', seed });
  /**
   * Enter, or the send button: a message as before, else what the consent line said it would do.
   * A start is single flight: the pending `run.start` (shared with the pane) and, until that
   * renders, a ref.
   */
  const submitNow = () => {
    if (!invocation || !agentState) return sendNow();
    latestUpload.current.dismissError();
    setExtraFiles('');
    const seed = { task: invocation.prompt, draft: draft.body };
    switch (agentState.kind) {
      case 'files':
      case 'too-long':
        // Nothing starts; the line under the text says why.
        return;
      case 'empty':
      case 'unknown-outcome':
      case 'loading':
        return openAgentPane(seed);
      case 'no-model':
      case 'blocked':
        return openAgentPane({ ...seed, pickModel: true });
      case 'ready':
        break;
    }
    if (!taskModel || startingTask || starting.current) return;
    const ask = controller.askAgent;
    if (!ask) return openAgentPane(seed);
    starting.current = true;
    // Exactly the model the line showed, never one read again at Enter.
    void ask({
      prompt: invocation.prompt,
      draftBody: draft.body,
      model: taskModel,
      contextChannels,
    }).finally(() => {
      starting.current = false;
    });
  };
  const known = useMemo(() => knownInstitutions(models.providers), [models.providers]);
  const taskError = (failure: CrewActionError) =>
    taskStartNote({
      error: failure,
      check,
      model: shownModel?.model ?? agentCopy.model,
      institution:
        check.institution?.label ??
        workspaceInstitutionLabel(controller.connection, snapshot, known),
      onChooseModel: () =>
        openAgentPane({
          task: parsed?.prompt ?? '',
          draft: draft.body,
          pickModel: true,
        }),
      onOpenPane: () => openAgentPane({ task: parsed?.prompt ?? '', draft: draft.body }),
    });

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
    taskError,
    tooLongToKeep: draftTooLongToKeep(draft.body),
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
        onSubmit={submitNow}
        agentMode={invocation !== null}
        agentRefused={agentState?.kind === 'files' || agentState?.kind === 'too-long'}
        startingTask={startingTask}
        renderAgentLine={(hintId) =>
          agentState ? (
            <AgentConsentLine
              id={hintId}
              state={agentState}
              slug={name}
              model={shownModelName}
              provider={taskProvider}
              privateOnly={check.privateOnly}
              reads={contextChannels.length + 1}
              access={remoteAccessText(controller.connection)}
              onChange={() =>
                openAgentPane({
                  task: invocation?.prompt ?? '',
                  draft: draft.body,
                  pickModel: true,
                })
              }
              onPostAsMessage={() => setLiteral(true)}
            />
          ) : literal && parsed ? (
            <div className="crew-compose-agent" data-tone="neutral">
              <p id={hintId} className="crew-compose-agent-text">
                {composerCopy.postingAsMessage}
              </p>
              <span className="crew-compose-agent-actions">
                <Button
                  type="button"
                  variant="link"
                  className="h-auto p-0 text-supporting"
                  onClick={() => setLiteral(false)}
                >
                  {composerCopy.startTaskInstead}
                </Button>
              </span>
            </div>
          ) : null
        }
        buildRows={(query) =>
          buildSuggestions({
            query,
            snapshot,
            channel,
            slug: name,
            dir,
            agentNameTaken: nameTaken,
            agentDetail: shownModelName ? (
              <>
                {composerCopy.agentRowDetail(shownModelName, name)}{' '}
                <ModelTierMarks provider={taskProvider} privateOnly={check.privateOnly} />
              </>
            ) : (
              composerCopy.agentRowDetail(null, name)
            ),
          })
        }
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
 * A start from the composer that failed (`@agent`, OW-10), above the card: "Couldn't start your
 * agent." and the reason in its own node, a model refusal in Ask my agent's words with **Choose
 * another model**, and a start whose outcome is unknown as the pane's gate title with **Open Ask my
 * agent**, where the gate is (C9 to C12). The draft keeps the task's words either way.
 */
function taskStartNote({
  error,
  check,
  model,
  institution,
  onChooseModel,
  onOpenPane,
}: {
  error: CrewActionError;
  check: TaskStartCheck;
  model: string;
  institution: string | null;
  onChooseModel(): void;
  onOpenPane(): void;
}): ReactNode {
  if (error.code === 'crew_start_outcome_unknown')
    return (
      <Note
        tone="warning"
        role="alert"
        action={
          <Button type="button" variant="secondary" size="sm" onClick={onOpenPane}>
            {composerCopy.openAskMyAgent}
          </Button>
        }
      >
        <span>{unknownOutcomeCopy.title}</span>
      </Note>
    );
  const modelRefused = isAffiliationRefusal(error) || isPublicModelRefusal(error);
  const reason =
    modelRefusalText({
      error,
      mismatch: check.mismatchText,
      publicText: check.publicText,
      model,
      institution,
    }) ?? error.message;
  return (
    <Note
      tone="danger"
      role="alert"
      action={
        modelRefused ? (
          <Button type="button" variant="secondary" size="sm" onClick={onChooseModel}>
            {composerCopy.chooseAnotherModel}
          </Button>
        ) : undefined
      }
    >
      <span>{composerCopy.startErrorLead}</span> <span>{reason}</span>
    </Note>
  );
}

/**
 * The consent line under the text while the draft starts a task (OW-10): what Enter will do, said
 * before Enter, because Enter is the consent. When Enter starts the task it names the model and
 * its tier mark (the same chip Ask my agent shows), where the agent posts, what it reads and what
 * it may do on the server; otherwise it says what Enter does instead, or why nothing starts. The
 * words are a live region, so they are heard as they change; the links beside them are not.
 *
 * **Change** opens Ask my agent on its model picker with the task's words; **Post as a message**
 * sends this draft as written.
 */
function AgentConsentLine({
  id,
  state,
  slug,
  model,
  provider,
  privateOnly,
  reads,
  access,
  onChange,
  onPostAsMessage,
}: {
  id: string;
  state: AgentStartState;
  slug: string;
  model: string;
  provider: ProviderDetails | undefined;
  privateOnly: boolean;
  /** How many channels the task reads, the one it posts in included. */
  reads: number;
  access: string | null;
  onChange(): void;
  onPostAsMessage(): void;
}) {
  let tone: 'neutral' | 'warning' = 'neutral';
  let text: ReactNode;
  switch (state.kind) {
    case 'ready':
      text = (
        <>
          <span className="crew-compose-agent-model">
            {composerCopy.agentStartsWith(model)}{' '}
            <ModelTierMarks provider={provider} privateOnly={privateOnly} />.
          </span>{' '}
          <span>
            {reads > 1 ? composerCopy.agentReadsWide(reads, slug) : composerCopy.agentReads(slug)}
          </span>
          {access && <span className="crew-compose-agent-access">{access}</span>}
        </>
      );
      break;
    case 'blocked':
      tone = 'warning';
      text = `${state.text} ${composerCopy.agentOpensPicker}`;
      break;
    case 'no-model':
      text = composerCopy.agentNoModel;
      break;
    case 'loading':
      text = composerCopy.agentLoading;
      break;
    case 'empty':
      text = composerCopy.agentEmpty;
      break;
    case 'unknown-outcome':
      tone = 'warning';
      text = composerCopy.agentUnknownOutcome;
      break;
    case 'files':
      tone = 'warning';
      text = composerCopy.agentFiles;
      break;
    case 'too-long':
      tone = 'warning';
      text = composerCopy.agentTooLong;
      break;
  }
  return (
    <div className="crew-compose-agent" data-tone={tone}>
      <Bot className="crew-compose-agent-icon" aria-hidden />
      <p id={id} className="crew-compose-agent-text" role="status">
        {text}
      </p>
      <span className="crew-compose-agent-actions">
        {state.kind === 'ready' && (
          <Button
            type="button"
            variant="link"
            className="h-auto p-0 text-supporting"
            aria-label={composerCopy.agentChangeName}
            onClick={onChange}
          >
            {composerCopy.agentChange}
          </Button>
        )}
        <Button
          type="button"
          variant="link"
          className="h-auto p-0 text-supporting"
          onClick={onPostAsMessage}
        >
          {composerCopy.postAsMessage}
        </Button>
      </span>
    </div>
  );
}

/**
 * The one note directly above the card, in priority order: the send's answer, an upload failure,
 * the note about several dropped files, a draft too long to keep across a channel switch
 * (MSG2-N4), the picker a drop just opened, then the layout's standing note. One at a time, so
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
 * hint lasts only while the picker it names is open. The note about several files names the
 * one Crew took, in red with its own dismiss control as the manual says (DW-18). It goes the
 * same ways as an upload failure except one: the file it names landing in the draft is not an
 * edit by the person, so that leaves it standing.
 */
function composerNote({
  error,
  taskError,
  tooLongToKeep,
  uploadError,
  onDismissUpload,
  extraFiles,
  onDismissExtraFiles,
  dropHint,
  note,
}: {
  error: CrewActionError | null;
  /** How a failed `@agent` start reads (`taskStartNote`); a send's answer is `sendNote`'s. */
  taskError(error: CrewActionError): ReactNode;
  tooLongToKeep: boolean;
  uploadError: string;
  onDismissUpload(): void;
  extraFiles: string;
  onDismissExtraFiles(): void;
  dropHint: string;
  note: ReactNode;
}): ReactNode {
  let content: ReactNode = null;
  if (error) {
    content = error.action === 'task-start' ? taskError(error) : sendNote(error);
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
  } else if (tooLongToKeep) {
    content = (
      <Note tone="warning" role="status">
        {composerCopy.draftTooLongToKeep}
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
  /** Enter and Send: `onSend` for a message, the agent's start for `@agent` (OW-10). */
  onSubmit(): void;
  /** The draft starts a task: Send reads "Start agent task" and the consent line shows. */
  agentMode: boolean;
  /** Nothing starts for this draft (files, too long); the consent line says why. */
  agentRefused: boolean;
  /** A task is being started, from here or from Ask my agent. */
  startingTask: boolean;
  /** The consent line (or the "posts as a message" line), whose words carry `hintId`. */
  renderAgentLine(hintId: string): ReactNode;
  /** The suggestion rows for the word before the caret. */
  buildRows(query: SuggestionQuery): ComposerSuggestion[];
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
  onSubmit,
  agentMode,
  agentRefused,
  startingTask,
  renderAgentLine,
  buildRows,
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
  const hintId = useId();
  const listId = useId();
  const agentLine = renderAgentLine(hintId);

  // The suggestion list follows the caret (`suggestionQuery`); `null` is "at the end".
  const [caretAt, setCaretAt] = useState<number | null>(null);
  const [focused, setFocused] = useState(false);
  const suggestions = useComposerSuggestions({
    body,
    caret: caretAt ?? body.length,
    focused,
    build: buildRows,
  });
  // Where the caret goes once an accepted row's text is in the box.
  const placeCaret = useRef<number | null>(null);

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
  // After an accepted row: the caret goes after what it inserted, not to the end.
  useLayoutEffect(() => {
    const at = placeCaret.current;
    const node = textarea.current;
    if (at === null || !node) return;
    placeCaret.current = null;
    node.setSelectionRange(at, at);
    setCaretAt(at);
  }, [body]);
  /** Insert a suggestion row's text. It starts nothing: Enter does, once the list is closed. */
  const acceptRow = (index: number) => {
    const next = suggestions.accept(index);
    if (!next) return;
    // The person's own edit, so the restored-draft rule leaves the caret to `placeCaret`.
    caret.typed(next.body);
    placeCaret.current = next.caret;
    setBody(next.body);
    textarea.current?.focus();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // An open suggestion list takes its keys first (Enter and Tab accept a row); it ignores every
    // key while an input method is composing, as the rest of this handler does.
    const listed = suggestions.onKeyDown(e);
    if (listed === 'accept') {
      acceptRow(suggestions.active);
      return;
    }
    if (listed) return;
    if (
      e.key !== 'Enter' ||
      e.shiftKey ||
      e.nativeEvent.isComposing ||
      e.nativeEvent.keyCode === 229
    )
      return;
    e.preventDefault();
    if (!e.repeat) onSubmit();
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
      {suggestions.open && (
        <ComposerSuggestions
          id={listId}
          rows={suggestions.rows}
          active={suggestions.active}
          onHover={suggestions.setActive}
          onPick={acceptRow}
        />
      )}
      <textarea
        ref={setTextarea}
        className="crew-compose-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
        aria-label={composerCopy.label(name)}
        placeholder={composerCopy.placeholder(name)}
        rows={1}
        // Laid out in the direction of what is typed (QA M13): Hebrew or Arabic runs right to left.
        dir="auto"
        value={body}
        // The suggestion list it offers (`@agent`, `/task`, people): named, and its row pointed at,
        // while focus stays here. The accessible name stays the pinned "Message #name" (C6).
        aria-autocomplete="list"
        aria-controls={suggestions.open ? listId : undefined}
        aria-activedescendant={
          suggestions.open ? suggestionId(listId, suggestions.active) : undefined
        }
        aria-describedby={agentLine ? hintId : undefined}
        // Writable while a post is on its way (QA M8): what is typed now stays, because success
        // takes out only what was sent.
        onChange={(event) => {
          caret.typed(event.target.value);
          setBody(event.target.value);
          setCaretAt(event.target.selectionStart);
          setFocused(true);
        }}
        onSelect={(event) => setCaretAt(event.currentTarget.selectionStart)}
        onKeyDown={(event) => {
          caret.touched();
          onKeyDown(event);
        }}
        onPointerDown={caret.touched}
        onFocus={(event) => {
          setFocused(true);
          caret.focused(event);
        }}
        onBlur={() => setFocused(false)}
        onPaste={onPaste}
      />
      {agentLine}
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
        {/* One node in both modes. For a draft that starts a task it is "Start agent task", described
            by the consent line, and disabled while a task is starting (single flight) or when
            nothing would start; "Send message" stays its name for every other draft (C6). */}
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="crew-compose-send">
              <Button
                type="button"
                shape="round"
                variant={hasContent || agentMode ? 'default' : 'secondary'}
                data-variant={hasContent || agentMode ? 'default' : 'secondary'}
                aria-label={agentMode ? composerCopy.startTask : composerCopy.send}
                aria-describedby={agentMode && agentLine ? hintId : undefined}
                aria-busy={(agentMode ? startingTask : posting) || undefined}
                disabled={agentMode ? startingTask || agentRefused : !hasContent && !posting}
                className={cn(
                  !agentMode && !hasContent && 'cursor-not-allowed disabled:opacity-100'
                )}
                onClick={() => {
                  if (agentMode) onSubmit();
                  else if (!posting) onSend();
                  // After a send the person keeps typing: focus goes back to the text, not to a
                  // button that is about to be disabled by the emptied draft.
                  textarea.current?.focus();
                }}
              >
                {(agentMode ? startingTask : posting) ? (
                  <Loader2 className="size-4 animate-spin" aria-hidden />
                ) : agentMode ? (
                  <Bot className="size-4" aria-hidden />
                ) : (
                  <ArrowUp className="size-4" aria-hidden />
                )}
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>
            {agentMode
              ? composerCopy.startTaskTooltip
              : posting
                ? composerCopy.sending
                : composerCopy.send}
          </TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}
