import { useRef, useState } from 'react';
import { Plus } from '../icons/app-icons';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { useSurfaceTriggerHint } from '../ui/surfaceTriggerHint';
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
 *
 * In a chat that exists, a last row opens the chat's decision monitors: the
 * summary's Decisions section, the other way in, is drawn only once a chat has a
 * decision call or a monitor, so without this row nothing could add the first.
 */
export function ComposerPlusMenu({
  onAttachFiles,
  onInsertTrigger,
  onOpenDecisionMonitors,
}: {
  /** The files picked in the system dialog, handled exactly like a drop. */
  onAttachFiles: (files: FileList) => void;
  /** Put `@` or `/` at the caret and open the matching menu. */
  onInsertTrigger: (trigger: ComposerTrigger) => void;
  /** Open the chat's Monitors dialog. Absent (and the row with it) before the chat exists. */
  onOpenDecisionMonitors?: () => void;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const chosenTrigger = useRef<ComposerTrigger | null>(null);
  // Controlled for its whole life, so the label hint can know the menu is open and stay shut
  // over it (H2).
  const [menuOpen, setMenuOpen] = useState(false);
  const hint = useSurfaceTriggerHint(menuOpen);

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
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <Tooltip {...hint.tooltip}>
          <TooltipTrigger asChild {...hint.trigger}>
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
          {onOpenDecisionMonitors ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                data-testid="composer-plus-decision-monitors"
                onSelect={onOpenDecisionMonitors}
              >
                {COPY.decisionMonitors}
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}
