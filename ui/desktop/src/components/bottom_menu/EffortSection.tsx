import './pickers.css';
import * as React from 'react';
import { InfoTip, useInfoTipId } from '../ui/info-tip';
import { SegmentedControl, type SegmentedOption } from '../ui/segmented-control';
import {
  REASONING_EFFORT_HELP,
  REASONING_EFFORT_LABELS,
  REASONING_EFFORTS,
  setReasoningEffort,
  type ReasoningEffort,
} from '../../store/reasoningEffort';
import { MODEL_COPY } from './copy';

const OPTIONS: ReadonlyArray<SegmentedOption<ReasoningEffort>> = REASONING_EFFORTS.map(
  (effort) => ({
    value: effort,
    label: REASONING_EFFORT_LABELS[effort],
  })
);

export interface EffortSectionProps {
  /** Where the effort is kept (`store/reasoningEffort.ts`): a chat's scope, or a draft's. */
  scope: string;
  /** The effort the store holds for `scope`. */
  effort: ReasoningEffort;
  /** Lets the surrounding menu reach the selected segment with Up and Down. */
  controlRef?: React.Ref<HTMLDivElement>;
}

/**
 * A choice applies immediately and keeps the menu open. The surrounding menu's vertical keys
 * visit this control as one stop; horizontal keys stay within its radio group.
 */
export function EffortSection({ scope, effort, controlRef }: EffortSectionProps) {
  const labelId = React.useId();
  const helpId = useInfoTipId();
  return (
    <div
      role="group"
      aria-labelledby={labelId}
      className="br-effort"
      data-testid="effort-section"
      onKeyDown={(event) => {
        if (
          ['ArrowLeft', 'ArrowRight', 'Home', 'End', ' '].includes(event.key) &&
          (event.target as HTMLElement).closest('[role="radiogroup"]')
        )
          event.stopPropagation();
      }}
    >
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
      </div>
      <SegmentedControl
        ref={controlRef}
        options={OPTIONS}
        fill
        value={effort}
        onValueChange={(next) => setReasoningEffort(scope, next)}
        aria-labelledby={labelId}
        aria-describedby={helpId}
        className="br-effort__control"
        data-testid="effort-control"
      />
    </div>
  );
}

const MENU_ROW_SELECTOR =
  '[role="menuitem"]:not([data-disabled]), [role="menuitemradio"]:not([data-disabled]), [role="menuitemcheckbox"]:not([data-disabled])';

/** An InfoTip's trigger: its glyph, or a badge it made focusable (`ui/info-tip.tsx`). */
const INFO_TIP_TRIGGER_SELECTOR = '[data-slot="info-tip-trigger"]';

/**
 * Radix menus visit only menu rows with Up and Down, and Tab leaves an open menu (Q2-50), so
 * anything else in a menu that takes focus is out of the keyboard's reach unless it joins that
 * order. This puts the selected effort segment and every InfoTip in the menu (the model menu's
 * "About this model" and "About effort" glyphs and its affiliation badge, whose help was
 * reachable by the pointer alone) among the rows in document order, and moves focus to them
 * before the radio group's own key handler can read Up or Down as a value change. Row to row
 * stays Radix's own move. A focus moved here is a script's, so it previews nothing (H1); Enter
 * or Space keeps the tip, and a screen reader hears its words through `aria-describedby`.
 */
export function moveFocusAcrossMenuStops(
  event: React.KeyboardEvent<HTMLElement>,
  control: HTMLElement | null
): void {
  if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const menu = event.currentTarget;
  const target = event.target as HTMLElement;
  const selected =
    control?.querySelector<HTMLElement>('[role="radio"][aria-checked="true"]') ?? null;
  const extras = Array.from(menu.querySelectorAll<HTMLElement>(INFO_TIP_TRIGGER_SELECTOR));
  if (selected) extras.push(selected);
  if (extras.length === 0) return;
  const rows = Array.from(menu.querySelectorAll<HTMLElement>(MENU_ROW_SELECTOR));
  const stops = [...rows, ...extras].sort((a, b) =>
    a === b ? 0 : a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1
  );
  const step = event.key === 'ArrowDown' ? 1 : -1;

  let next: HTMLElement | undefined;
  if (target === menu) {
    // Focus on the menu itself (the pointer left a row): Radix goes to a row; with no row, the
    // first or the last of the other stops.
    if (rows.length > 0) return;
    next = step > 0 ? stops[0] : stops[stops.length - 1];
  } else {
    const current = selected && control?.contains(target) ? selected : target;
    const at = stops.indexOf(current);
    if (at === -1) return;
    const neighbour = stops[at + step];
    if (rows.includes(current)) {
      // From a row to a row is Radix's move; to anything else, this one's.
      if (!neighbour || rows.includes(neighbour)) return;
      next = neighbour;
    } else if (!neighbour) {
      // At the menu edge, keep the value and focus where they are.
      event.preventDefault();
      event.stopPropagation();
      return;
    } else {
      next = neighbour;
    }
  }
  event.preventDefault();
  event.stopPropagation();
  next.focus();
}
