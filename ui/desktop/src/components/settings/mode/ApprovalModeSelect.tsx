import { useId } from 'react';
import { ChevronDown } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { approvalsCopy } from '../chat/copy';

export interface ApprovalModeSelectProps {
  /** The stored `BIOROUTER_MODE` key. */
  value: string;
  onValueChange: (mode: string) => void;
  /** From `SettingRow`: the row label's id, so the trigger is named by it. */
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
  id?: string;
}

/**
 * Settings > Chat > Approvals > Approval mode: a select menu (spec §3.13, principle 5: four
 * options that each need a line of explanation). Each item carries its one-line description as
 * a muted second line, so the page needs none.
 *
 * The trigger's accessible name is the row label AND the current value ("Approval mode
 * Autonomous"), so the visible word is in the name.
 *
 * ⚠ The row's `id` is deliberately NOT forwarded to the trigger: Radix names the menu after
 * the trigger's own id, and replacing it would leave the menu labelled by nothing.
 */
export function ApprovalModeSelect({
  value,
  onValueChange,
  'aria-labelledby': ariaLabelledBy,
  'aria-describedby': ariaDescribedBy,
}: ApprovalModeSelectProps) {
  const valueId = `${useId().replace(/[^a-zA-Z0-9_-]/g, '')}-approval-mode-value`;
  const current = approvalsCopy.modes.find((mode) => mode.key === value) ?? approvalsCopy.modes[0];

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="secondary"
          size="sm"
          aria-labelledby={ariaLabelledBy ? `${ariaLabelledBy} ${valueId}` : undefined}
          aria-label={ariaLabelledBy ? undefined : `${approvalsCopy.mode} ${current.label}`}
          aria-describedby={ariaDescribedBy}
          data-testid="approval-mode-trigger"
        >
          <span id={valueId}>{current.label}</span>
          <ChevronDown aria-hidden="true" className="text-text-muted" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72">
        <DropdownMenuRadioGroup value={current.key} onValueChange={onValueChange}>
          {approvalsCopy.modes.map((mode) => (
            <DropdownMenuRadioItem
              key={mode.key}
              value={mode.key}
              data-testid={`approval-mode-${mode.key}`}
            >
              <span className="flex min-w-0 flex-col">
                <span className="text-text-default">{mode.label}</span>
                <span className="text-supporting text-text-muted">{mode.description}</span>
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
