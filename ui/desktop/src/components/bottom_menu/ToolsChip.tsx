import './pickers.css';
import { useRef, useState } from 'react';
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
 * opens one popover: a segmented control (Extensions, Skills, Knowledge) over
 * one list at a time, each a search field, a scope line and switch rows.
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

export function ToolsChip({ sessionId }: ToolsChipProps) {
  const [open, setOpen] = useState(false);
  const [segment, setSegment] = useState<ToolsSegment>('extensions');
  const inputRef = useRef<HTMLInputElement>(null);
  // Escape gives focus back to the chip; a click elsewhere leaves it where the
  // click put it, so a mouse user is not handed a focus ring on the chip.
  const closedByKeyboard = useRef(false);

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
        <div className="br-picker-popover__tabs">
          <SegmentedControl
            fill
            aria-label={TOOLS_COPY.segmentsLabel}
            options={SEGMENTS}
            value={segment}
            onValueChange={setSegment}
          />
        </div>
        {segment === 'extensions' && <ExtensionsPanel picker={extensions} inputRef={inputRef} />}
        {segment === 'skills' && <SkillsPanel picker={skills} inputRef={inputRef} />}
        {segment === 'knowledge' && (
          <KnowledgePanel picker={knowledge} inChat={sessionId !== null} inputRef={inputRef} />
        )}
      </PopoverContent>
    </Popover>
  );
}
