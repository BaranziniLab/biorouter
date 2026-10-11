import './pickers.css';
import { useEffect, useRef, useState } from 'react';
import type { SessionClassification } from '../../api/types.gen';
import { ChevronDown } from '../icons/app-icons';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { SegmentedControl, type SegmentedOption } from '../ui/segmented-control';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { ExtensionsPanel, useExtensionPicker } from './BottomMenuExtensionSelection';
import { KnowledgePanel, useKnowledgePicker } from './BottomMenuKnowledgeSelection';
import { SkillsPanel, useSkillPicker } from './BottomMenuSkillSelection';
import { TOOLS_COPY } from './copy';

/**
 * The composer's Tools chip (spec 3.7): one control, labelled "Tools" and the
 * sum of the extensions, skills and knowledge bases this chat can reach. It
 * opens one popover over the chip: one list at a time (a search field, a scope
 * line and switch rows) with the segmented control (Extensions, Skills,
 * Knowledge) under it, directly above the chip.
 *
 * ⚠ **The segmented control never moves (R-25, owner message 13).** It is the
 * popover's LAST child, so it sits on the popover's bottom edge, and that edge
 * is pinned 6px above the chip: Radix places a `side="top"` popover by its
 * bottom, so a longer or shorter list grows or shrinks the popover UPWARD.
 * Two things would still move it, and both are switched off here:
 * - collision handling: Radix flips a popover to the side with more room and
 *   slides it along the chip. `avoidCollisions={false}` rules both out, and
 *   the height is capped by the room above instead (`max-height` reads
 *   Radix's available-height variable in `pickers.css`), so a long list
 *   scrolls and the popover never needs to go anywhere else;
 * - a control at the top: the old layout, where every tab switch moved the
 *   control by the change in the list's height, under the pointer that had
 *   just clicked it.
 * `toolsPopover.browser.test.tsx` measures it in Chromium across the three
 * lists and two window heights. The DOM order is the visual order (search,
 * scope line, list, control), so Tab walks the popover top to bottom.
 *
 * ⚠ The props are the wave-0 contract (WS-PICKERS). `ChatInput` places this
 * inside the composer card and owns nothing below it.
 */
export interface ToolsChipProps {
  /** The focused chat, or `null` on Home and in a chat that has not started. */
  sessionId: string | null;
  /**
   * The chat's ratcheted classification (`ChatInput`'s `sessionPrivacyTier`).
   * Not what an extension pairing is judged on: that is the bound model's tier,
   * which the extension list reads itself (issue #56).
   */
  privacyTier?: SessionClassification;
}

export type ToolsSegment = 'extensions' | 'skills' | 'knowledge';

const SEGMENTS: ReadonlyArray<SegmentedOption<ToolsSegment>> = [
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

export function ToolsChip({ sessionId }: ToolsChipProps) {
  const [open, setOpen] = useState(false);
  const [segment, setSegment] = useState<ToolsSegment>('extensions');
  const inputRef = useRef<HTMLInputElement>(null);
  // Escape gives focus back to the chip; a click elsewhere leaves it where the
  // click put it, so a mouse user is not handed a focus ring on the chip.
  const closedByKeyboard = useRef(false);
  // A list picked with the POINTER hands focus to its search field, the place
  // focus goes when the popover opens, so "click Skills, type gg" filters. Left
  // on the segment, the typing went nowhere. A list picked with the arrow keys
  // keeps focus on the control, or the next arrow would type into the field.
  const pickedWithPointer = useRef(false);
  const focusSearchAfterSwitch = useRef(false);
  const onSegmentChange = (next: ToolsSegment) => {
    focusSearchAfterSwitch.current = pickedWithPointer.current;
    pickedWithPointer.current = false;
    setSegment(next);
  };
  useEffect(() => {
    if (!focusSearchAfterSwitch.current) return;
    focusSearchAfterSwitch.current = false;
    // The new list's field: `inputRef` is handed to whichever panel is mounted.
    inputRef.current?.focus();
  }, [segment]);

  // The three lists live for as long as the composer does, so the chip's number
  // is right before the popover is ever opened. Each re-reads its source when
  // its own list comes on screen.
  const extensions = useExtensionPicker(sessionId, {
    open: open && segment === 'extensions',
    // "Show this chat's extensions" from elsewhere in the app.
    onOpenRequest: () => {
      setSegment('extensions');
      setOpen(true);
    },
  });
  const skills = useSkillPicker(sessionId, { open: open && segment === 'skills' });
  const knowledge = useKnowledgePicker({ open: open && segment === 'knowledge' });

  const total = extensions.activeCount + skills.activeCount + knowledge.visibleCount;
  const counts = [extensions.activeCount, skills.activeCount, knowledge.visibleCount] as const;

  const onOpenChange = (next: boolean) => {
    setOpen(next);
    if (!next) {
      extensions.setQuery('');
      skills.setQuery('');
      knowledge.setQuery('');
    }
  };

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <button
              type="button"
              data-testid="composer-tools-chip"
              className="br-picker-chip"
              aria-label={TOOLS_COPY.chipLabel(...counts)}
            >
              <span>{TOOLS_COPY.chip}</span>
              <span className="br-picker-chip__count">{total}</span>
              <ChevronDown className="br-picker-chip__chevron" aria-hidden="true" />
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="top">{TOOLS_COPY.chipTooltip(...counts)}</TooltipContent>
      </Tooltip>
      <PopoverContent
        side="top"
        align="start"
        // Never flipped below the chip and never slid: the bottom edge, and the
        // segmented control on it, stay put (R-25). The room above caps the height.
        avoidCollisions={false}
        // Off the 44px band at the top of a chat, a window-drag region that would
        // eat clicks on the search field (CLAUDE.md, "Toasts"); 8px elsewhere.
        collisionPadding={TOOLS_POPOVER_COLLISION_PADDING}
        aria-label={TOOLS_COPY.chip}
        data-testid="composer-tools-popover"
        className="br-picker-popover"
        onOpenAutoFocus={(event) => {
          // Straight into the search field, so typing filters at once.
          event.preventDefault();
          inputRef.current?.focus();
        }}
        onEscapeKeyDown={() => {
          closedByKeyboard.current = true;
        }}
        onCloseAutoFocus={(event) => {
          if (!closedByKeyboard.current) event.preventDefault();
          closedByKeyboard.current = false;
        }}
      >
        {segment === 'extensions' && <ExtensionsPanel picker={extensions} inputRef={inputRef} />}
        {segment === 'skills' && <SkillsPanel picker={skills} inputRef={inputRef} />}
        {segment === 'knowledge' && (
          <KnowledgePanel picker={knowledge} inChat={sessionId !== null} inputRef={inputRef} />
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
            aria-label={TOOLS_COPY.segmentsLabel}
            options={SEGMENTS}
            value={segment}
            onValueChange={onSegmentChange}
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}
