import { InfoTip } from '../../ui/info-tip';

/**
 * The label of a GROUP of fields or rows inside a workflow dialog ("Activities", "Parameters",
 * "Response schema", "Model"), with its InfoTip as the label's sibling.
 *
 * It is not a field label. A field's label is 14px `text-label` and names ONE control; a group
 * label names a set, so it draws smaller and quieter (12px, strong, muted: `.br-form-group-label`
 * in `workflows.css`). When the two looked alike, "Activities" read as one more field above
 * "Intro message" and "Model" as one more field above "Provider" (vision round 1, V3-18), and
 * the grouping was ambiguous.
 */
export function GroupLabel({ id, label, info }: { id: string; label: string; info?: string }) {
  return (
    <div className="flex min-w-0 items-center">
      <span id={id} className="br-form-group-label">
        {label}
      </span>
      {info && <InfoTip label={label} help={info} />}
    </div>
  );
}
