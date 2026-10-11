import './pickers.css';
import * as React from 'react';
import { InfoTip, useInfoTipId } from '../ui/info-tip';
import { Slider, type SliderStep } from '../ui/slider';
import {
  REASONING_EFFORT_HELP,
  REASONING_EFFORT_LABELS,
  REASONING_EFFORTS,
  setReasoningEffort,
  type ReasoningEffort,
} from '../../store/reasoningEffort';
import { MODEL_COPY } from './copy';

const STEPS: ReadonlyArray<SliderStep<ReasoningEffort>> = REASONING_EFFORTS.map((effort) => ({
  value: effort,
  label: REASONING_EFFORT_LABELS[effort],
}));

export interface EffortSectionProps {
  /** Where the effort is kept (`store/reasoningEffort.ts`): a chat's scope, or a draft's. */
  scope: string;
  /** The effort the store holds for `scope`. */
  effort: ReasoningEffort;
  /** The slider's thumb, so the menu around it can move focus there (`moveFocusAcrossEffort`). */
  thumbRef?: React.Ref<HTMLSpanElement>;
}

/**
 * The model menu's Effort section (R-26, owner message 13): ONE slider with three stops,
 * Quick, Normal and Deep, where the menu used to list three radio rows.
 *
 * The head row says "Effort" with an InfoTip on the left and the current stop on the right; the
 * slider under it fills the menu's width. A change applies at once, through the same store and
 * the same scope the radio rows wrote (`setReasoningEffort(scope, …)`), and the menu stays open
 * so the person sees what they set.
 *
 * The InfoTip carries what the three levels mean, and the thumb points at the InfoTip's hidden
 * description, so a screen reader hears it on focus without hovering: the per-level descriptions
 * the radio rows had in tooltips are now one place.
 *
 * The section is a `group` named "Effort" (a `menu` may own groups; a bare slider among its rows
 * would not be one of its allowed children). How the keyboard reaches it inside the menu is
 * `moveFocusAcrossEffort`, below.
 */
export function EffortSection({ scope, effort, thumbRef }: EffortSectionProps) {
  const labelId = React.useId();
  const helpId = useInfoTipId();
  return (
    <div role="group" aria-labelledby={labelId} className="br-effort" data-testid="effort-section">
      <div className="br-effort__head">
        <span id={labelId} className="br-effort__label">
          {MODEL_COPY.effortGroup}
        </span>
        <InfoTip
          id={helpId}
          label={MODEL_COPY.aboutEffort}
          help={REASONING_EFFORT_HELP}
          data-testid="effort-info"
        />
        {/* The slider speaks its own value; this is the same word for the eye. */}
        <span className="br-effort__value" aria-hidden="true" data-testid="effort-value">
          {REASONING_EFFORT_LABELS[effort]}
        </span>
      </div>
      <Slider
        ref={thumbRef}
        steps={STEPS}
        value={effort}
        onValueChange={(next) => setReasoningEffort(scope, next)}
        aria-labelledby={labelId}
        aria-describedby={helpId}
        className="br-effort__slider"
        data-testid="effort-slider"
      />
    </div>
  );
}

const MENU_ROW_SELECTOR =
  '[role="menuitem"]:not([data-disabled]), [role="menuitemradio"]:not([data-disabled]), [role="menuitemcheckbox"]:not([data-disabled])';

/**
 * Lets the menu's Up and Down keys reach the Effort slider, as `onKeyDownCapture` on the menu.
 *
 * ⚠ **A Radix menu's arrow keys visit menu ROWS only** (its roving focus group holds
 * `MenuItem`s), and its Tab leaves the menu (`useTabLeavesMenu`), so a slider among the rows is
 * reached by neither: the keyboard could not set the effort at all. So the slider is made one
 * more stop in the menu's vertical order: Up from the row below it lands on the thumb, Down
 * from the thumb goes to that row, and the same the other way round should rows ever sit above
 * it. On the thumb, Up and Down walk the MENU (the menu runs vertically, and a slider that
 * swallowed them would be a dead end the person can only leave with Escape); Left, Right, Home,
 * End, Page Up and Page Down move the effort, as the slider pattern has them. Anything else
 * (row to row, typeahead, Tab, Escape) is left to Radix.
 *
 * Runs in the capture phase, so it decides before the thumb's own handler (which would take Up
 * and Down) and before the row's roving handler.
 */
export function moveFocusAcrossEffort(
  event: React.KeyboardEvent<HTMLElement>,
  thumb: HTMLElement | null
): void {
  if (!thumb || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return;
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const menu = event.currentTarget;
  const target = event.target as HTMLElement;
  const rows = Array.from(menu.querySelectorAll<HTMLElement>(MENU_ROW_SELECTOR));
  // Every stop in document order: the rows and the thumb.
  const stops = [...rows, thumb].sort((a, b) =>
    a === b ? 0 : a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1
  );
  const step = event.key === 'ArrowDown' ? 1 : -1;

  let next: HTMLElement | undefined;
  if (target === thumb) {
    next = stops[stops.indexOf(thumb) + step];
    // No row that way: stay on the thumb, and do not let the slider read it as a value key.
    if (!next) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
  } else if (target === menu) {
    // Focus on the menu itself (the pointer left a row): with no row to go to, the thumb.
    if (rows.length > 0) return;
    next = thumb;
  } else {
    const at = stops.indexOf(target);
    if (at === -1 || stops[at + step] !== thumb) return;
    next = thumb;
  }
  event.preventDefault();
  event.stopPropagation();
  next.focus();
}
