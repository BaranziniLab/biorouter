import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Progress } from '../../ui/progress';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import {
  Copy,
  Download,
  Eye,
  File,
  Fingerprint,
  MoreHorizontal,
  Pause,
} from '../../icons/app-icons';
import { crewRequest } from '../crewApi';
import {
  beginTransfer,
  forgetTransfer,
  pauseTransfer,
  previewAttachment,
  resumeTransfer,
  type CrewTransfer,
} from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';
import { filesCopy } from './copy';
import { formatBytes } from './formatBytes';
import { TransferMenuItems } from './TransferRow';
import { useCopyAnnouncer } from './useCopyAnnouncer';
import { useCrewTransfers } from './useCrewTransfers';
import './files.css';

/** `blob.status`: what the workspace knows about one shared file. */
export interface CrewBlob {
  id: string;
  channel_id: string;
  name: string;
  size: number;
  sha256: string;
  complete: boolean;
  media_type: string;
}

/** The media types the daemon's preview route renders; nothing else offers Preview. */
export const PREVIEWABLE_MEDIA_TYPES: readonly string[] = [
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
];

const failureText = (failure: unknown, fallback: string) =>
  failure instanceof Error && failure.message ? failure.message : fallback;

/**
 * A shared file in a message: a 40px row with the file glyph, its name, its size in 1024 units,
 * a glyph-only **Save attachment** (the secure native save dialog), **Preview image** for the
 * image types the daemon previews, and `⋯` for Copy file ID, Copy SHA-256 and this computer's
 * download record (Pause while it moves, Resume… and Remove from list otherwise). A download
 * in progress draws a thin bar along the row's bottom edge.
 *
 * The metadata is asked for once per file. Download progress comes from the one shared
 * transfers poller, never from a timer of the card's own (L13), so a channel with fifty
 * attachments still polls once, and only while something moves.
 */
export function AttachmentCard({ connectionId, blobId }: { connectionId: string; blobId: string }) {
  const [metadata, setMetadata] = useState<CrewBlob | null>(null);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [working, setWorking] = useState(false);
  const [preview, setPreview] = useState('');
  const previewUrl = useRef('');
  const generation = useRef(0);
  const { transfers, refresh } = useCrewTransfers(connectionId);
  const { copy, region } = useCopyAnnouncer();

  useEffect(() => {
    let active = true;
    setMetadata(null);
    setError('');
    setLoadError('');
    setPreview('');
    void crewRequest<CrewBlob>(connectionId, 'blob.status', { blob_id: blobId })
      .then((blob) => {
        if (active) setMetadata(blob);
      })
      .catch((failure: unknown) => {
        if (active) setLoadError(failureText(failure, filesCopy.detailsFailed));
      });
    return () => {
      active = false;
      generation.current += 1;
      URL.revokeObjectURL(previewUrl.current);
      previewUrl.current = '';
    };
  }, [connectionId, blobId]);

  const download = useMemo(() => {
    const records = transfers.filter(
      (item) =>
        item.direction === 'download' &&
        item.blob_id === blobId &&
        item.connection_id === connectionId
    );
    return records[records.length - 1] ?? null;
  }, [transfers, blobId, connectionId]);

  const act = useCallback(
    async (operation: () => Promise<unknown>, fallback: string) => {
      const current = generation.current;
      setWorking(true);
      setError('');
      try {
        await operation();
        await refresh();
      } catch (failure) {
        if (current === generation.current) setError(failureText(failure, fallback));
      } finally {
        if (current === generation.current) setWorking(false);
      }
    },
    [refresh]
  );

  const save = () => {
    if (!metadata) return;
    void act(
      () =>
        beginTransfer({
          connection_id: connectionId,
          channel_id: metadata.channel_id,
          direction: 'download',
          blob_id: blobId,
          suggestedName: metadata.name,
        }),
      filesCopy.downloadFailed
    );
  };

  const togglePreview = async () => {
    if (!metadata) return;
    if (preview) {
      setPreview('');
      return;
    }
    const current = generation.current;
    setWorking(true);
    setError('');
    try {
      const image = await previewAttachment(connectionId, metadata.channel_id, blobId);
      if (current !== generation.current) return;
      URL.revokeObjectURL(previewUrl.current);
      previewUrl.current = URL.createObjectURL(image);
      setPreview(previewUrl.current);
    } catch (failure) {
      if (current === generation.current) setError(failureText(failure, filesCopy.previewFailed));
    } finally {
      if (current === generation.current) setWorking(false);
    }
  };

  const name = metadata?.name || filesCopy.attachment;
  const state = download ? transferStatePresentation(download) : null;
  const downloading = Boolean(state?.active);
  const previewable = Boolean(metadata && PREVIEWABLE_MEDIA_TYPES.includes(metadata.media_type));
  const meta = [
    metadata ? formatBytes(metadata.size) : '',
    state && state.key !== 'saved' ? state.word : '',
  ]
    .filter(Boolean)
    .join(' · ');

  return (
    <div className="crew-attachment" data-downloading={downloading ? 'true' : undefined}>
      <div className="crew-attachment-row">
        <File className="crew-attachment-icon" aria-hidden />
        <span className="crew-attachment-name">{name}</span>
        {meta ? <span className="crew-attachment-meta">{meta}</span> : null}
        <span className="crew-attachment-actions">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={filesCopy.saveAttachment}
                disabled={!metadata || working || downloading}
                onClick={save}
              >
                <Download aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{filesCopy.saveTooltip(name)}</TooltipContent>
          </Tooltip>
          {previewable ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  shape="round"
                  aria-label={filesCopy.previewImage}
                  aria-pressed={Boolean(preview)}
                  disabled={working && !preview}
                  onClick={() => void togglePreview()}
                >
                  <Eye aria-hidden />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {preview ? filesCopy.hidePreview : filesCopy.previewImage}
              </TooltipContent>
            </Tooltip>
          ) : null}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={filesCopy.fileActions(name)}
              >
                <MoreHorizontal aria-hidden />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="crew-menu">
              <DropdownMenuItem onSelect={() => void copy(blobId)}>
                <Copy aria-hidden />
                {filesCopy.copyFileId}
              </DropdownMenuItem>
              {metadata?.sha256 ? (
                <DropdownMenuItem onSelect={() => void copy(metadata.sha256)}>
                  <Fingerprint aria-hidden />
                  {filesCopy.copySha}
                </DropdownMenuItem>
              ) : null}
              {download ? (
                <>
                  <DropdownMenuSeparator />
                  {downloading && state?.key !== 'pausing' ? (
                    <DropdownMenuItem
                      onSelect={() =>
                        void act(() => pauseTransfer(download.id), filesCopy.transferFailed)
                      }
                    >
                      <Pause aria-hidden />
                      {filesCopy.pause}
                    </DropdownMenuItem>
                  ) : null}
                  <TransferMenuItems
                    transfer={download}
                    onResume={(transfer: CrewTransfer) =>
                      void act(() => resumeTransfer(transfer), filesCopy.transferFailed)
                    }
                    onRemove={(transfer: CrewTransfer) =>
                      void act(() => forgetTransfer(transfer.id), filesCopy.transferFailed)
                    }
                  />
                </>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        </span>
      </div>
      {downloading && state?.percent !== undefined ? (
        <Progress
          value={state.percent}
          label={`${name}: ${state.word}`}
          className="crew-attachment-progress"
        />
      ) : null}
      {preview ? (
        <img className="crew-attachment-preview" src={preview} alt={filesCopy.previewAlt(name)} />
      ) : null}
      {download?.error ? <p className="crew-file-row-error">{download.error}</p> : null}
      {loadError ? <p className="crew-file-row-error">{loadError}</p> : null}
      {error ? (
        <p role="alert" className="crew-file-row-error">
          {error}
        </p>
      ) : null}
      {region}
    </div>
  );
}
