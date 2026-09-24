import { useCallback, useId, useMemo, useRef, useState } from 'react';
import { Button } from '../../ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../../ui/dropdown-menu';
import { File, MoreHorizontal, Paperclip } from '../../icons/app-icons';
import { forgetTransfer, pauseTransfer, resumeTransfer, type CrewTransfer } from '../crewTransfers';
import { useCrew } from '../state/CrewControllerContext';
import { AttachmentCard, type CrewBlob } from './AttachmentCard';
import { filesCopy } from './copy';
import { formatBytes } from './formatBytes';
import { ServerPathRow } from './ServerPathRow';
import { TransferMenuItems, TransferRow } from './TransferRow';
import { useCrewTransfers } from './useCrewTransfers';
import './files.css';

const failureText = (failure: unknown, fallback: string) =>
  failure instanceof Error && failure.message ? failure.message : fallback;

type SharedItem = { kind: 'attachment'; id: string } | { kind: 'reference'; id: string };

/**
 * The details pane's Files tab, in three sections, each shown only when it has something:
 *
 * - **In progress**: this channel's transfers on this computer that have not finished, with
 *   Pause while they move and Resume… / Remove from list otherwise.
 * - **Uploaded, not sent**: finished uploads to this channel that are not in the composer,
 *   with **Attach** (formerly "Restore to composer"). Attach re-reads the file's record first
 *   and refuses one that no longer matches the upload.
 * - **In this channel**: the files and server paths in the loaded messages, newest first.
 *
 * Transfers come from the one shared poller; an action error renders once, here.
 */
export function FilesTab() {
  const { connectionId, channelId, messages, draft, addAttachment, request } = useCrew();
  const { transfers, error: listError, refresh } = useCrewTransfers(connectionId);
  const [error, setError] = useState('');
  const [attaching, setAttaching] = useState<string | null>(null);
  const headingId = useId();
  const scope = useRef(`${connectionId}\n${channelId}`);
  scope.current = `${connectionId}\n${channelId}`;

  const channelTransfers = useMemo(
    () =>
      transfers.filter(
        (item) => item.connection_id === connectionId && item.channel_id === channelId
      ),
    [transfers, connectionId, channelId]
  );
  const inProgress = channelTransfers.filter((item) => item.state !== 'completed');
  const attachedIds = new Set(draft.attachments.map((item) => item.id));
  const notSent = channelTransfers.filter(
    (item) =>
      item.direction === 'upload' &&
      item.state === 'completed' &&
      item.blob_id &&
      !attachedIds.has(item.blob_id)
  );
  const shared = useMemo(() => {
    const seen = new Set<string>();
    const items: SharedItem[] = [];
    for (const message of [...messages].reverse()) {
      for (const id of message.attachments ?? []) {
        if (seen.has(`a:${id}`)) continue;
        seen.add(`a:${id}`);
        items.push({ kind: 'attachment', id });
      }
      for (const id of message.references ?? []) {
        if (seen.has(`r:${id}`)) continue;
        seen.add(`r:${id}`);
        items.push({ kind: 'reference', id });
      }
    }
    return items;
  }, [messages]);

  const act = useCallback(
    async (operation: () => Promise<unknown>) => {
      const started = scope.current;
      setError('');
      try {
        await operation();
        await refresh();
      } catch (failure) {
        if (started === scope.current) setError(failureText(failure, filesCopy.transferFailed));
      }
    },
    [refresh]
  );
  const actions = {
    onPause: (transfer: CrewTransfer) => void act(() => pauseTransfer(transfer.id)),
    onResume: (transfer: CrewTransfer) => void act(() => resumeTransfer(transfer)),
    onRemove: (transfer: CrewTransfer) => void act(() => forgetTransfer(transfer.id)),
  };

  /** Re-read the shared file and attach it only when it is still exactly this upload. */
  const attach = async (transfer: CrewTransfer) => {
    const started = scope.current;
    setAttaching(transfer.id);
    setError('');
    try {
      const blob = await request<CrewBlob>('blob.status', { blob_id: transfer.blob_id });
      if (
        !blob.complete ||
        blob.channel_id !== channelId ||
        blob.sha256 !== transfer.sha256 ||
        blob.size !== transfer.size
      )
        throw new Error(filesCopy.attachMismatch);
      if (started === scope.current) addAttachment({ id: blob.id, name: blob.name });
    } catch (failure) {
      if (started === scope.current) setError(failureText(failure, filesCopy.transferFailed));
    } finally {
      if (started === scope.current) setAttaching(null);
    }
  };

  const empty = inProgress.length === 0 && notSent.length === 0 && shared.length === 0;

  return (
    <div className="crew-files-tab">
      {error ? (
        <p role="alert" className="crew-file-row-error">
          {error}
        </p>
      ) : null}
      {listError ? <p className="crew-file-row-error">{filesCopy.transfersUnavailable}</p> : null}
      {inProgress.length > 0 ? (
        <section className="crew-files-section" aria-labelledby={`${headingId}-in-progress`}>
          <h3 id={`${headingId}-in-progress`} className="crew-files-heading">
            {filesCopy.inProgress}
          </h3>
          <ul className="crew-file-list">
            {inProgress.map((transfer) => (
              <TransferRow key={transfer.id} transfer={transfer} {...actions} />
            ))}
          </ul>
        </section>
      ) : null}
      {notSent.length > 0 ? (
        <section className="crew-files-section" aria-labelledby={`${headingId}-not-sent`}>
          <h3 id={`${headingId}-not-sent`} className="crew-files-heading">
            {filesCopy.uploadedNotSent}
          </h3>
          <ul className="crew-file-list">
            {notSent.map((transfer) => (
              <li key={transfer.id} className="crew-file-row">
                <div className="crew-file-row-main">
                  <File className="crew-file-row-icon" aria-hidden />
                  <span className="crew-file-row-name">{transfer.name}</span>
                  <span className="crew-file-row-meta">{formatBytes(transfer.size)}</span>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    aria-label={filesCopy.attachNamed(transfer.name)}
                    disabled={attaching === transfer.id}
                    onClick={() => void attach(transfer)}
                  >
                    <Paperclip aria-hidden />
                    {filesCopy.attach}
                  </Button>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        shape="round"
                        aria-label={filesCopy.fileActions(transfer.name)}
                      >
                        <MoreHorizontal aria-hidden />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="crew-menu">
                      <TransferMenuItems
                        transfer={transfer}
                        onResume={actions.onResume}
                        onRemove={actions.onRemove}
                      />
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {shared.length > 0 ? (
        <section className="crew-files-section" aria-labelledby={`${headingId}-in-channel`}>
          <h3 id={`${headingId}-in-channel`} className="crew-files-heading">
            {filesCopy.inThisChannel}
          </h3>
          <ul className="crew-file-list crew-file-list-cards">
            {shared.map((item) => (
              <li key={`${item.kind}:${item.id}`}>
                {item.kind === 'attachment' ? (
                  <AttachmentCard connectionId={connectionId} blobId={item.id} />
                ) : (
                  <ServerPathRow connectionId={connectionId} referenceId={item.id} />
                )}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {empty ? <p className="crew-files-empty">{filesCopy.noFiles}</p> : null}
    </div>
  );
}
