import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Paperclip } from '../../icons/app-icons';
import { composerCopy } from './copy';

/**
 * The composer's paperclip: a real menu (`aria-haspopup="menu"`, from Radix) with the two ways
 * a file reaches a channel. **Upload a file…** opens the secure main-process picker;
 * **Share a server path…** opens the dialog that shares a path by name without uploading
 * anything. The items are words only, like every other Crew menu (Q2-61).
 *
 * The menu opens beside the paperclip, its bottom level with the button's, so it stays inside
 * the card and never covers the note above the card (Q2-61): the note is usually the answer to
 * what the person just did, such as a refused paste, and the menu is where they go next.
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
      <DropdownMenuContent align="end" side="right" sideOffset={4} className="crew-menu">
        <DropdownMenuItem disabled={uploading} onSelect={onUpload}>
          {composerCopy.upload}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onSharePath}>{composerCopy.sharePath}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
