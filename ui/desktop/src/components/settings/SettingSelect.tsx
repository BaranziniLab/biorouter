import { useId } from 'react';
import { ChevronDown } from '../icons/app-icons';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { cn } from '../../utils';

export interface SettingSelectOption<T extends string = string> {
  value: T;
  label: string;
  /** One muted line under the label inside the menu item; no trailing period. */
  description?: string;
  testId?: string;
}

export interface SettingSelectProps<T extends string = string> {
  options: ReadonlyArray<SettingSelectOption<T>>;
  value: T;
  onValueChange: (value: T) => void;
  /** From `SettingRow`: the row label's id. The trigger is named by the label AND the value. */
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
  /** Used only when no row names the select. */
  'aria-label'?: string;
  /** Given by `SettingRow`; deliberately not forwarded (see below). */
  id?: string;
  disabled?: boolean;
  /** A fixed trigger width, so a column of selects lines up (`w-36`). */
  triggerClassName?: string;
  /** The menu's width; items with a description need room (`w-72`). */
  contentClassName?: string;
  'data-testid'?: string;
}

/**
 * The one settings select (spec §3.13, principle 5): four or more options, or options that each
 * need a line of explanation. A `secondary sm` trigger showing the value and a chevron, opening a
 * menu of radio items whose descriptions are a muted second line, so the page needs none.
 *
 * The trigger's accessible name is the row label and the current value ("Approval mode
 * Autonomous"), so the visible word is in the name (label in name).
 *
 * ⚠ The row's `id` is NOT forwarded to the trigger: Radix names the menu after the trigger's own
 * id, and replacing it would leave the menu labelled by nothing.
 */
export function SettingSelect<T extends string>({
  options,
  value,
  onValueChange,
  'aria-labelledby': ariaLabelledBy,
  'aria-describedby': ariaDescribedBy,
  'aria-label': ariaLabel,
  disabled,
  triggerClassName,
  contentClassName,
  'data-testid': testId,
}: SettingSelectProps<T>) {
  const valueId = `${useId().replace(/[^a-zA-Z0-9_-]/g, '')}-setting-select-value`;
  const current = options.find((option) => option.value === value) ?? options[0];

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild disabled={disabled}>
        <Button
          variant="secondary"
          size="sm"
          className={cn('justify-between', triggerClassName)}
          aria-labelledby={ariaLabelledBy ? `${ariaLabelledBy} ${valueId}` : undefined}
          aria-label={ariaLabelledBy ? undefined : ariaLabel}
          aria-describedby={ariaDescribedBy}
          data-testid={testId}
        >
          <span id={valueId} className="truncate">
            {current?.label}
          </span>
          <ChevronDown aria-hidden="true" className="text-text-muted" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className={contentClassName}>
        <DropdownMenuRadioGroup
          value={current?.value}
          onValueChange={(next) => onValueChange(next as T)}
        >
          {options.map((option) => (
            <DropdownMenuRadioItem
              key={option.value}
              value={option.value}
              data-testid={option.testId}
            >
              {option.description ? (
                <span className="flex min-w-0 flex-col">
                  <span className="text-text-default">{option.label}</span>
                  <span className="text-supporting text-text-muted">{option.description}</span>
                </span>
              ) : (
                option.label
              )}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
