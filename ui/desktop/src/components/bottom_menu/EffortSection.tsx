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

/**
 * Radix menus only visit menu rows. Include the selected effort segment in that vertical order,
 * before the radio group's own key handler can interpret Up or Down as a value change.
 */
export function moveFocusAcrossEffort(
  event: React.KeyboardEvent<HTMLElement>,
  control: HTMLElement | null
): void {
  if (!control || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return;
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const selected = control.querySelector<HTMLElement>('[role="radio"][aria-checked="true"]');
  if (!selected) return;
  const menu = event.currentTarget;
  const target = event.target as HTMLElement;
  const rows = Array.from(menu.querySelectorAll<HTMLElement>(MENU_ROW_SELECTOR));
  const stops = [...rows, selected].sort((a, b) =>
    a === b ? 0 : a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1
  );
  const step = event.key === 'ArrowDown' ? 1 : -1;

  let next: HTMLElement | undefined;
  if (control.contains(target)) {
    next = stops[stops.indexOf(selected) + step];
    // At the menu edge, keep the value and focus where they are.
    if (!next) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
  } else if (target === menu) {
    // Focus on the menu itself (the pointer left a row): with no row to go to, the selection.
    if (rows.length > 0) return;
    next = selected;
  } else {
    const at = stops.indexOf(target);
    if (at === -1 || stops[at + step] !== selected) return;
    next = selected;
  }
  event.preventDefault();
  event.stopPropagation();
  next.focus();
}
