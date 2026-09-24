import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Link, Paperclip, Upload } from '../../icons/app-icons';
import { composerCopy } from './copy';

/**
 * The composer's paperclip: a real menu (`aria-haspopup="menu"`, from Radix) with the two ways
 * a file reaches a channel. **Upload a file…** opens the secure main-process picker, the only
 * source of a file capability; **Share a server path…** opens the dialog that shares a path by
 * name without uploading anything.
 */
export function AttachMenu({
  onUpload,
  onSharePath,
  disabled = false,
  uploading = false,
}: {
  onUpload(): void;
  onSharePath(): void;
  disabled?: boolean;
  /** The picker is already open: a second one would race the first. */
  uploading?: boolean;
}) {
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              shape="round"
              aria-label={composerCopy.attach}
              disabled={disabled}
              className="text-text-muted"
            >
              <Paperclip aria-hidden />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>{composerCopy.attach}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="start" side="top" className="crew-menu">
        <DropdownMenuItem disabled={uploading} onSelect={onUpload}>
          <Upload aria-hidden />
          {composerCopy.upload}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onSharePath}>
          <Link aria-hidden />
          {composerCopy.sharePath}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
