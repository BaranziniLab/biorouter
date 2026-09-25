import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Button } from '../../ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem } from '../../ui/dropdown-menu';
import { Progress } from '../../ui/progress';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Copy, Download, Eye, File, Fingerprint, Pause } from '../../icons/app-icons';
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
import { CopyForSupport, useMenuCopy } from '../timeline/TimelineCopy';
import { postedLabel, useAttachmentWhich, useRegisterAttachment } from './attachmentIndex';
import { cachedBlob, forgetBlob, rememberBlob } from './blobMetadataCache';
import { filesCopy } from './copy';
import { MoreActionsTrigger } from './GlyphButton';
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
 * a glyph-only **Save {name}** (the secure native save dialog), **Preview {name}** for the image
 * types the daemon previews, and `⋯`: Save {name}…, this computer's download record (Pause while
 * it moves, Resume… and Remove from list otherwise), and last "Copy for support" ▸ Copy file ID
 * and Copy SHA-256 (Q3-26) — never a menu of IDs only. A download in progress draws a thin bar
 * along the row's bottom edge.
 *
 * Every control is named for the file, and when another loaded card has the same name, for its
 * post time too ("Save counts.csv, 6:54 PM"), which the meta shows as well ("100 bytes · 6:54
 * PM"): two same-named files were two identical cards (Q3-13). What the card learns goes into the
 * channel view's attachment index, which is how it knows.
 *
 * `tabIndex`: in the timeline the controls are Tab stops only while their message is the active
 * row, like the row's own actions (Q3-05); in the Files tab they always are.
 *
 * The metadata is asked for once per mount, and a finished file's answer is kept for the next
 * mount (`blobMetadataCache`), which starts from it: a card that mounted again drew a nameless
 * "Attachment" with a dimmed Save until its answer came back (Q4-17). Download progress comes
 * from the one shared transfers poller, never from a timer of the card's own (L13), so a channel
 * with fifty attachments still polls once, and only while something moves.
 *
 * The name takes the row and the size gives way first (`files.css`, Q4-03); the whole name is in
 * its tooltip. `postedTimeInMeta={false}` leaves a namesake's post time out of the meta where the
 * line above the card already says when it was shared (the Files tab); the controls' names keep it.
 *
 * `sending`: the stand-in for a file in a post the broker accepted but the observer has not
 * delivered yet (Q4-19). The same row at the same height — name and size — with no controls, and
 * nothing registered in the channel's attachment index; `fallbackName` is the draft's name for it
 * until its answer comes. Its answer is kept like any other, so the posted card that replaces it
 * starts named.
 */
export function AttachmentCard({
  connectionId,
  blobId,
  tabIndex,
  postedAt = null,
  postedTimeInMeta = true,
  sending = false,
  fallbackName = '',
}: {
  connectionId: string;
  blobId: string;
  /** The controls' Tab stop; absent, they are ordinary Tab stops. */
  tabIndex?: number;
  /** When the message carrying the file was posted (Unix milliseconds), when known. */
  postedAt?: number | null;
  /** Whether a namesake's post time is in the meta (the timeline) or not (the Files tab). */
  postedTimeInMeta?: boolean;
  /** A file in a post on its way: shown, not acted on. */
  sending?: boolean;
  /** The name to show until the file's metadata loads (a post on its way). */
  fallbackName?: string;
}) {
  const [metadata, setMetadata] = useState<CrewBlob | null>(() => cachedBlob(connectionId, blobId));
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [working, setWorking] = useState(false);
  const [preview, setPreview] = useState('');
  const previewUrl = useRef('');
  const generation = useRef(0);
  const { transfers, refresh } = useCrewTransfers(connectionId);
  const { copy, region } = useCopyAnnouncer();
  const [menuOpen, setMenuOpen] = useState(false);
  const menuCopy = useMenuCopy<'id' | 'sha'>(copy, setMenuOpen);

  useEffect(() => {
    let active = true;
    // A finished file's kept answer, so a card that mounts again is never nameless (Q4-17).
    setMetadata(cachedBlob(connectionId, blobId));
    setError('');
    setLoadError('');
    setPreview('');
    void crewRequest<CrewBlob>(connectionId, 'blob.status', { blob_id: blobId })
      .then((blob) => {
        if (!active) return;
        rememberBlob(connectionId, blobId, blob);
        setMetadata(blob);
      })
      .catch((failure: unknown) => {
        if (!active) return;
        // The kept answer is no longer known to hold: draw what a first failed load draws.
        forgetBlob(connectionId, blobId);
        setMetadata(null);
        setLoadError(failureText(failure, filesCopy.detailsFailed));
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

  const name = metadata?.name || fallbackName || filesCopy.attachment;
  useRegisterAttachment(
    blobId,
    metadata && !sending
      ? {
          name: metadata.name,
          sha256: metadata.sha256,
          complete: metadata.complete,
          postedAt,
        }
      : null
  );
  const posted = useAttachmentWhich(blobId, name, metadata ? postedAt : null);
  const which = filesCopy.which(posted);
  const state = download ? transferStatePresentation(download) : null;
  const downloading = Boolean(state?.active);
  const previewable = Boolean(metadata && PREVIEWABLE_MEDIA_TYPES.includes(metadata.media_type));
  const saveDisabled = !metadata || working || downloading;
  const meta = [
    metadata ? formatBytes(metadata.size) : '',
    // A namesake's own time, without the ", 1 of 2" its controls may carry — except where the
    // line above the card says when it was shared (the Files tab, Q4-03).
    posted && postedTimeInMeta ? postedLabel(postedAt) : '',
    state && state.key !== 'saved' && !sending ? state.word : '',
  ]
    .filter(Boolean)
    .join(' · ');

  if (sending) {
    return (
      <div className="crew-attachment-card" data-sending="true">
        <div className="crew-attachment-row">
          <File className="crew-attachment-icon" aria-hidden />
          <span className="crew-attachment-name">{name}</span>
          {meta ? <span className="crew-attachment-meta">{meta}</span> : null}
        </div>
      </div>
    );
  }

  return (
    <div className="crew-attachment-card" data-downloading={downloading ? 'true' : undefined}>
      <div className="crew-attachment-row">
        <File className="crew-attachment-icon" aria-hidden />
        {/* The whole name on hover, however much of it the row can show (Q4-03). */}
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="crew-attachment-name" data-crew-file-name="">
              {name}
            </span>
          </TooltipTrigger>
          <TooltipContent>{name}</TooltipContent>
        </Tooltip>
        {meta ? <span className="crew-attachment-meta">{meta}</span> : null}
        <span className="crew-attachment-actions">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={filesCopy.saveNamed(name, which)}
                tabIndex={tabIndex}
                disabled={saveDisabled}
                onClick={save}
              >
                <Download aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{filesCopy.saveNamed(name, which)}</TooltipContent>
          </Tooltip>
          {previewable ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  shape="round"
                  aria-label={filesCopy.previewNamed(name, which)}
                  aria-pressed={Boolean(preview)}
                  tabIndex={tabIndex}
                  disabled={working && !preview}
                  onClick={() => void togglePreview()}
                >
                  <Eye aria-hidden />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {preview
                  ? filesCopy.hidePreviewNamed(name, which)
                  : filesCopy.previewNamed(name, which)}
              </TooltipContent>
            </Tooltip>
          ) : null}
          <DropdownMenu open={menuOpen} onOpenChange={menuCopy.onOpenChange}>
            <MoreActionsTrigger name={name} which={which} tabIndex={tabIndex} />
            <DropdownMenuContent align="end" className="crew-menu">
              <DropdownMenuItem disabled={saveDisabled} onSelect={save}>
                <Download aria-hidden />
                {filesCopy.saveItem(name)}
              </DropdownMenuItem>
              {download ? (
                <>
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
              <CopyForSupport label={filesCopy.copyForSupport}>
                <DropdownMenuItem
                  data-crew-copy-state={menuCopy.state('id')}
                  onSelect={menuCopy.select('id', blobId)}
                >
                  <Copy aria-hidden />
                  {menuCopy.label('id', filesCopy.copyFileId)}
                </DropdownMenuItem>
                {metadata?.sha256 ? (
                  <DropdownMenuItem
                    data-crew-copy-state={menuCopy.state('sha')}
                    onSelect={menuCopy.select('sha', metadata.sha256)}
                  >
                    <Fingerprint aria-hidden />
                    {menuCopy.label('sha', filesCopy.copySha)}
                  </DropdownMenuItem>
                ) : null}
              </CopyForSupport>
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
