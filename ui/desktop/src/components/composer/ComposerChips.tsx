import type { DroppedFile } from '../../hooks/useFileDrop';
import type { QuoteReference } from '../../utils/quotedText';
import type { RefSpan } from '../../utils/resourceRefs';
import { File, Image, RotateCcw, X } from '../icons/app-icons';
import { ResourceRefChip } from '../ResourceRefChip';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { InfoTip } from '../ui/info-tip';
import { Spinner } from '../ui/spinner';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { COMPOSER_COPY } from './copy';
import './composer.css';

const COPY = COMPOSER_COPY.attachments;

/** An image staged in the composer (a paste, a preview region, or a draft handed back). */
export interface ComposerImage {
  id: string;
  dataUrl: string;
  isLoading: boolean;
  error?: string;
}

/** A 16px remove or retry target inside a chip, named by its tooltip. */
function ChipAction({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="br-composer-chip-action"
          aria-label={label}
          onClick={onClick}
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * A 48px thumbnail. While it saves it sits under a translucent scrim with the
 * one spinner; once saved its remove control appears in the corner on hover or
 * focus.
 */
function Thumbnail({
  src,
  alt,
  loading,
  onRemove,
}: {
  src?: string;
  alt: string;
  loading: boolean;
  onRemove: () => void;
}) {
  return (
    <div className="br-composer-thumb" data-testid="composer-image-thumb">
      {src && <img src={src} alt={alt} />}
      {loading ? (
        <div className="br-composer-thumb__scrim">
          <Spinner size={16} />
        </div>
      ) : (
        <span className="br-composer-thumb__remove">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="secondary"
                size="xs"
                shape="round"
                aria-label={COPY.removeImage}
                onClick={onRemove}
                className="size-5"
              >
                <X className="size-3" aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{COPY.removeImage}</TooltipContent>
          </Tooltip>
        </span>
      )}
    </div>
  );
}

/**
 * One chip for something that did not attach: the name in danger ink, the
 * reason behind an InfoTip (its text is always in the accessibility tree), and
 * the ways out.
 */
function FailedChip({
  name,
  error,
  icon,
  onRetry,
  onRemove,
  removeLabel,
}: {
  name: string;
  error: string;
  icon: 'file' | 'image';
  onRetry?: () => void;
  onRemove: () => void;
  removeLabel: string;
}) {
  const Icon = icon === 'image' ? Image : File;
  return (
    <Badge
      variant="chip"
      tone="danger"
      className="max-w-[280px] min-w-0"
      data-testid="composer-failed-chip"
    >
      <Icon className="size-3.5 shrink-0" aria-hidden />
      <span className="min-w-0 truncate">{name}</span>
      <InfoTip label={name} help={error} />
      {onRetry && (
        <ChipAction label={COPY.retryLabel} onClick={onRetry}>
          <RotateCcw className="size-3.5" aria-hidden />
        </ChipAction>
      )}
      <ChipAction label={removeLabel} onClick={onRemove}>
        <X className="size-3.5" aria-hidden />
      </ChipAction>
    </Badge>
  );
}

/**
 * What goes with the message, in one row above the text (Crew's
 * `ComposerChips`): the references, then the images as 48px thumbnails, then
 * the files as chips. Renders nothing when there is nothing to show, so an
 * empty composer has no empty row.
 */
export function ComposerChips({
  refs,
  images,
  files,
  onRemoveRef,
  onRemoveImage,
  onRetryImage,
  onRemoveFile,
}: {
  refs: readonly (RefSpan | QuoteReference)[];
  images: readonly ComposerImage[];
  files: readonly DroppedFile[];
  onRemoveRef: (index: number) => void;
  onRemoveImage: (id: string) => void;
  onRetryImage: (id: string) => void;
  onRemoveFile: (id: string) => void;
}) {
  if (refs.length === 0 && images.length === 0 && files.length === 0) return null;

  return (
    <ul className="br-composer-chips" data-testid="composer-chips">
      {refs.length > 0 && (
        // The reference chips keep their own test id: they are still the rail
        // a reference lands on.
        <li className="br-composer-chips__refs" data-testid="composer-reference-rail">
          {refs.map((ref, index) => (
            <ResourceRefChip
              key={`${index}:${ref.kind}:${ref.value}`}
              refSpan={ref}
              onRemove={() => onRemoveRef(index)}
            />
          ))}
        </li>
      )}

      {images.map((image) => (
        <li key={image.id}>
          {image.error && !image.isLoading ? (
            <FailedChip
              name={COPY.pastedImageAlt}
              error={image.error}
              icon="image"
              onRetry={image.dataUrl ? () => onRetryImage(image.id) : undefined}
              onRemove={() => onRemoveImage(image.id)}
              removeLabel={COPY.removeImage}
            />
          ) : (
            <Thumbnail
              src={image.dataUrl || undefined}
              alt={COPY.pastedImageAlt}
              loading={image.isLoading}
              onRemove={() => onRemoveImage(image.id)}
            />
          )}
        </li>
      ))}

      {files.map((file) => (
        <li key={file.id}>
          {file.error && !file.isLoading ? (
            <FailedChip
              name={file.name}
              error={file.error}
              icon={file.isImage ? 'image' : 'file'}
              onRemove={() => onRemoveFile(file.id)}
              removeLabel={COPY.removeFile}
            />
          ) : file.canUploadAsImage ? (
            <Thumbnail
              src={file.dataUrl}
              alt={COPY.imageAlt(file.name)}
              loading={Boolean(file.isLoading)}
              onRemove={() => onRemoveFile(file.id)}
            />
          ) : (
            <Badge
              variant="chip"
              className="max-w-[240px] min-w-0"
              data-testid="composer-file-chip"
            >
              <File className="size-3.5 shrink-0" aria-hidden />
              <span className="min-w-0 truncate text-text-default">{file.name}</span>
              {file.isLoading ? (
                <Spinner size={14} />
              ) : (
                <ChipAction label={COPY.removeFile} onClick={() => onRemoveFile(file.id)}>
                  <X className="size-3.5" aria-hidden />
                </ChipAction>
              )}
            </Badge>
          )}
        </li>
      ))}
    </ul>
  );
}
