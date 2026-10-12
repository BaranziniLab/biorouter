import { useRef } from 'react';
import { Plus } from '../icons/app-icons';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { COMPOSER_COPY } from './copy';

const COPY = COMPOSER_COPY.plus;

export type ComposerTrigger = '@' | '/';

/**
 * The composer's `+` (spec 3.7): a ghost round 28px button whose menu attaches a
 * file, or starts a mention or a command at the caret, so neither needs to be
 * known in advance. Text-only rows with a muted hint, the shared menu recipe.
 *
 * A chosen mention or command runs after the menu has closed and declined to
 * hand focus back to the `+`, so the caret lands in the text, not on a button.
 */
export function ComposerPlusMenu({
  onAttachFiles,
  onInsertTrigger,
}: {
  /** The files picked in the system dialog, handled exactly like a drop. */
  onAttachFiles: (files: FileList) => void;
  /** Put `@` or `/` at the caret and open the matching menu. */
  onInsertTrigger: (trigger: ComposerTrigger) => void;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const chosenTrigger = useRef<ComposerTrigger | null>(null);

  return (
    <>
      <input
        ref={fileInputRef}
        type="file"
        multiple
        hidden
        tabIndex={-1}
        aria-hidden="true"
        data-testid="composer-file-input"
        onChange={(event) => {
          const files = event.target.files;
          if (files && files.length > 0) onAttachFiles(files);
          // Picking the same file twice in a row still fires `change`.
          event.target.value = '';
        }}
      />
      <DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={COPY.label}
                data-testid="composer-plus"
                className="text-text-muted hover:text-text-default"
              >
                <Plus aria-hidden />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent>{COPY.label}</TooltipContent>
        </Tooltip>
        <DropdownMenuContent
          side="top"
          align="start"
          onCloseAutoFocus={(event) => {
            const trigger = chosenTrigger.current;
            if (!trigger) return;
            chosenTrigger.current = null;
            event.preventDefault();
            onInsertTrigger(trigger);
          }}
        >
          <DropdownMenuItem onSelect={() => fileInputRef.current?.click()}>
            {COPY.attachFile}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              chosenTrigger.current = '@';
            }}
          >
            {COPY.mention}
            <DropdownMenuShortcut>{COPY.mentionHint}</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              chosenTrigger.current = '/';
            }}
          >
            {COPY.commands}
            <DropdownMenuShortcut>{COPY.commandsHint}</DropdownMenuShortcut>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}
