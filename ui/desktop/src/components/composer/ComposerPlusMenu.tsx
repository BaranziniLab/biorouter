import '../bottom_menu/pickers.css';
import './composer.css';
import { useEffect, useRef, useState } from 'react';
import type { SessionClassification } from '../../api/types.gen';
import { cn } from '../../utils';
import { Plus } from '../icons/app-icons';
import { Button } from '../ui/button';
import { MENU_ROW_CLASS_NAME, MENU_ROW_TRAILING_CLASS_NAME } from '../ui/dropdown-menu';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { SegmentedControl, type SegmentedOption } from '../ui/segmented-control';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { useSurfaceTriggerHint } from '../ui/surfaceTriggerHint';
import { TOOLS_COPY } from '../bottom_menu/copy';
import { ToolsListPanel, useToolsLists, type ToolsSegment } from '../bottom_menu/toolsLists';
import { COMPOSER_COPY } from './copy';

const COPY = COMPOSER_COPY.plus;

export type ComposerTrigger = '@' | '/';

/** The four lists of the `+` popover: what to add, then the three tool lists. */
export type PlusSegment = 'add' | ToolsSegment;

const SEGMENTS: ReadonlyArray<SegmentedOption<PlusSegment>> = [
  { value: 'add', label: COPY.addSegment, testId: 'plus-segment-add' },
  {
    value: 'extensions',
    label: TOOLS_COPY.segments.extensions,
    testId: 'tools-segment-extensions',
  },
  { value: 'skills', label: TOOLS_COPY.segments.skills, testId: 'tools-segment-skills' },
  {
    value: 'knowledge',
    label: TOOLS_COPY.segments.knowledge,
    testId: 'tools-segment-knowledge',
  },
];

/**
 * How close the popover may come to the window's edges. The top clears the 44px
 * band (`--chrome-height`) with 8px to spare; Radix's `size` middleware turns it
 * into the available height that caps the popover.
 */
export const TOOLS_POPOVER_COLLISION_PADDING = { top: 52, right: 8, bottom: 8, left: 8 } as const;

export interface ComposerPlusMenuProps {
  /** The focused chat, or `null` on Home and in a chat that has not started. */
  sessionId: string | null;
  /**
   * The chat's ratcheted classification (`ChatInput`'s `sessionPrivacyTier`).
   * Not what an extension pairing is judged on: that is the bound model's tier,
   * which the extension list reads itself (issue #56).
   */
  privacyTier?: SessionClassification;
  /**
   * The composer's lasting identity (`ChatInput`'s hand-off key), under which
   * the settled counts are handed to the composer that replaces this one: a new
   * chat's first send remounts the composer (`composer/composerHandOff.ts`).
   */
  handOffKey?: string;
  /** The files picked in the system dialog, handled exactly like a drop. */
  onAttachFiles: (files: FileList) => void;
  /** Put `@` or `/` at the caret and open the matching menu. */
  onInsertTrigger: (trigger: ComposerTrigger) => void;
  /** Open the chat's Monitors dialog (in draft mode before it exists). Absent, no row. */
  onOpenDecisionMonitors?: () => void;
}

/**
 * The composer's `+` (owner, 2026-11-13): ONE button at the start of the one-line
 * composer, and one popover that holds everything that used to be split between
 * the `+` menu and the Tools chip.
 *
 * The popover is the Tools popover's recipe, unchanged: one list at a time, and
 * the segmented control on its bottom edge, which slides with the spring. The
 * control has a fourth segment in front, **Add**, holding the old `+` menu's rows
 * (attach files, mention, commands, and the chat's decision monitors), and a
 * click on `+` always opens on it. The other three are the old Tools lists
 * (Extensions, Skills, Knowledge), with their search, scope line and switches.
 *
 * ⚠ **The segmented control never moves (R-25, owner message 13).** It is the
 * popover's LAST child, so it sits on the popover's bottom edge, and that edge
 * is pinned 6px above the `+`: Radix places a `side="top"` popover by its
 * bottom, so a longer or shorter list grows or shrinks the popover UPWARD.
 * Collision handling would flip or slide it, so `avoidCollisions={false}`, and
 * the room above caps the height instead (`.br-picker-popover`'s `max-height`).
 * The DOM order is the visual order (list, then control), so Tab walks the
 * popover top to bottom.
 *
 * A chosen mention or command runs after the popover has closed and declined to
 * hand focus back to the `+`, so the caret lands in the text, not on a button.
 *
 * The counts the Tools chip used to carry (extensions, skills and knowledge bases
 * on in this chat) are the `+`'s tooltip and accessible name, and they are shown
 * only once all three lists have answered for THIS chat (V2-R7-3).
 */
export function ComposerPlusMenu({
  sessionId,
  handOffKey,
  onAttachFiles,
  onInsertTrigger,
  onOpenDecisionMonitors,
}: ComposerPlusMenuProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [segment, setSegment] = useState<PlusSegment>('add');
  const inputRef = useRef<HTMLInputElement>(null);
  const addListRef = useRef<HTMLDivElement>(null);
  // What to run once the popover has closed: a mention or a command (which puts
  // the caret in the text), or the monitors dialog (which takes focus itself).
  const afterClose = useRef<(() => void) | null>(null);
  // Escape gives focus back to the `+`; a click elsewhere leaves it where the
  // click put it, so a mouse user is not handed a focus ring on the button.
  const closedByKeyboard = useRef(false);
  // A list picked with the POINTER hands focus to its first stop (the search
  // field, or the first row of Add), so "click Skills, type gg" filters. A list
  // picked with the arrow keys keeps focus on the control, or the next arrow
  // would type into the field.
  const pickedWithPointer = useRef(false);
  const focusListAfterSwitch = useRef(false);

  const focusFirstStop = (target: PlusSegment) => {
    if (target === 'add') {
      addListRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    } else {
      inputRef.current?.focus();
    }
  };

  const onSegmentChange = (next: PlusSegment) => {
    focusListAfterSwitch.current = pickedWithPointer.current;
    pickedWithPointer.current = false;
    setSegment(next);
  };
  useEffect(() => {
    if (!focusListAfterSwitch.current) return;
    focusListAfterSwitch.current = false;
    focusFirstStop(segment);
  }, [segment]);

  // The three lists live for as long as the composer does, so the `+` has the
  // right counts before the popover is ever opened (`bottom_menu/toolsLists.tsx`).
  const lists = useToolsLists(sessionId, {
    open,
    segment: segment === 'add' ? null : segment,
    handOffKey,
    // "Show this chat's extensions" from elsewhere in the app.
    onOpenRequest: () => {
      setSegment('extensions');
      setOpen(true);
    },
  });
  const shown = lists.shown;

  // The label hint names the button on a hover and never comes back over the
  // open popover (H2).
  const hint = useSurfaceTriggerHint(open);

  const onOpenChange = (next: boolean) => {
    // A click on `+` opens on Add, the most common reason to press it. The
    // "show this chat's extensions" request sets its own list and opens directly.
    if (next) setSegment('add');
    setOpen(next);
    if (!next) lists.clearQueries();
  };

  const closeThen = (action: () => void) => {
    afterClose.current = action;
    onOpenChange(false);
  };

  // The Add list is a menu: Up and Down move between its rows, Home and End jump
  // to the ends, the way every menu in the app moves.
  const onAddListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const rows = [
      ...(addListRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []),
    ];
    if (rows.length === 0) return;
    const at = rows.indexOf(document.activeElement as HTMLElement);
    let next: number | null = null;
    if (event.key === 'ArrowDown') next = at < 0 ? 0 : (at + 1) % rows.length;
    else if (event.key === 'ArrowUp')
      next = at < 0 ? rows.length - 1 : (at - 1 + rows.length) % rows.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = rows.length - 1;
    if (next === null) return;
    event.preventDefault();
    rows[next]?.focus();
  };

  const accessibleName =
    shown !== null ? `${COPY.label}. ${TOOLS_COPY.chipLabel(...shown)}` : COPY.label;

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
      <Popover open={open} onOpenChange={onOpenChange}>
        <Tooltip {...hint.tooltip}>
          <TooltipTrigger asChild {...hint.trigger}>
            <PopoverTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="default"
                shape="round"
                aria-label={accessibleName}
                aria-busy={shown === null || undefined}
                data-testid="composer-plus"
                // Muted at rest; the kept step while its popover is open
                // (`.br-surface-trigger`, main.css), and the glyph turns a
                // quarter-eighth on the spring (`.br-composer-plus`, composer.css).
                className="br-surface-trigger br-composer-plus text-text-muted hover:text-text-default"
              >
                <Plus aria-hidden className="br-composer-plus__glyph" />
              </Button>
            </PopoverTrigger>
          </TooltipTrigger>
          <TooltipContent side="top">
            <p>{COPY.label}</p>
            {shown !== null && <p>{TOOLS_COPY.chipTooltip(...shown)}</p>}
          </TooltipContent>
        </Tooltip>
        <PopoverContent
          side="top"
          align="start"
          // Never flipped below the `+` and never slid: the bottom edge, and the
          // segmented control on it, stay put (R-25). The room above caps the height.
          avoidCollisions={false}
          // Off the 44px band at the top of a chat, a window-drag region that would
          // eat clicks on the search field (CLAUDE.md, "Toasts"); 8px elsewhere.
          collisionPadding={TOOLS_POPOVER_COLLISION_PADDING}
          aria-label={COPY.label}
          data-testid="composer-plus-popover"
          data-segment={segment}
          className="br-picker-popover br-plus-popover"
          onOpenAutoFocus={(event) => {
            // Straight to the list's first stop: the first row of Add, or the
            // search field, so typing filters at once.
            event.preventDefault();
            focusFirstStop(segment);
          }}
          onEscapeKeyDown={() => {
            closedByKeyboard.current = true;
          }}
          onCloseAutoFocus={(event) => {
            const action = afterClose.current;
            afterClose.current = null;
            if (action) {
              event.preventDefault();
              closedByKeyboard.current = false;
              action();
              return;
            }
            if (!closedByKeyboard.current) event.preventDefault();
            closedByKeyboard.current = false;
          }}
        >
          {segment === 'add' && (
            <div className="br-picker-panel br-plus-add" data-testid="composer-plus-add-panel">
              <div
                ref={addListRef}
                role="menu"
                aria-label={COPY.addListLabel}
                className="br-plus-add__list"
                onKeyDown={onAddListKeyDown}
              >
                <button
                  type="button"
                  role="menuitem"
                  data-testid="composer-plus-attach"
                  className={cn(MENU_ROW_CLASS_NAME, 'br-plus-action')}
                  onClick={() => {
                    // The system dialog needs the click's activation, so it opens
                    // now, while the popover closes.
                    fileInputRef.current?.click();
                    onOpenChange(false);
                  }}
                >
                  {COPY.attachFile}
                </button>
                <button
                  type="button"
                  role="menuitem"
                  data-testid="composer-plus-mention"
                  className={cn(MENU_ROW_CLASS_NAME, 'br-plus-action')}
                  onClick={() => closeThen(() => onInsertTrigger('@'))}
                >
                  {COPY.mention}
                  <span className={MENU_ROW_TRAILING_CLASS_NAME}>{COPY.mentionHint}</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  data-testid="composer-plus-commands"
                  className={cn(MENU_ROW_CLASS_NAME, 'br-plus-action')}
                  onClick={() => closeThen(() => onInsertTrigger('/'))}
                >
                  {COPY.commands}
                  <span className={MENU_ROW_TRAILING_CLASS_NAME}>{COPY.commandsHint}</span>
                </button>
                {onOpenDecisionMonitors ? (
                  <>
                    <div role="separator" className="br-plus-add__separator" />
                    <button
                      type="button"
                      role="menuitem"
                      data-testid="composer-plus-decision-monitors"
                      className={cn(MENU_ROW_CLASS_NAME, 'br-plus-action')}
                      onClick={() => closeThen(onOpenDecisionMonitors)}
                    >
                      {COPY.decisionMonitors}
                    </button>
                  </>
                ) : null}
              </div>
            </div>
          )}
          {segment !== 'add' && (
            <ToolsListPanel
              lists={lists}
              segment={segment}
              inChat={sessionId !== null}
              inputRef={inputRef}
            />
          )}
          {/* Last, on the bottom edge: the one place in the popover that does not move. */}
          <div
            className="br-picker-popover__tabs"
            data-testid="composer-tools-tabs"
            onPointerDown={() => {
              pickedWithPointer.current = true;
            }}
            // Capture: Radix selects on the focus an arrow key moves, inside the
            // item's own keydown, before the event would bubble up to here.
            onKeyDownCapture={() => {
              pickedWithPointer.current = false;
            }}
          >
            <SegmentedControl
              fill
              aria-label={COPY.segmentsLabel}
              options={SEGMENTS}
              value={segment}
              onValueChange={onSegmentChange}
            />
          </div>
        </PopoverContent>
      </Popover>
    </>
  );
}
