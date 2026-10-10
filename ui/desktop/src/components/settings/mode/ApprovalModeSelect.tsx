import { SettingSelect } from '../SettingSelect';
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

const OPTIONS = approvalsCopy.modes.map((mode) => ({
  value: mode.key as string,
  label: mode.label,
  description: mode.description,
  testId: `approval-mode-${mode.key}`,
}));

/**
 * Settings > Chat > Approvals > Approval mode: the one settings select (spec §3.13, principle
 * 5: four options that each need a line of explanation). Each item carries its one-line
 * description, so the page needs none. An unknown stored key shows as the first mode.
 */
export function ApprovalModeSelect({
  value,
  onValueChange,
  'aria-labelledby': ariaLabelledBy,
  'aria-describedby': ariaDescribedBy,
}: ApprovalModeSelectProps) {
  return (
    <SettingSelect
      options={OPTIONS}
      value={value}
      onValueChange={onValueChange}
      aria-labelledby={ariaLabelledBy}
      aria-describedby={ariaDescribedBy}
      aria-label={approvalsCopy.mode}
      contentClassName="w-72"
      data-testid="approval-mode-trigger"
    />
  );
}
