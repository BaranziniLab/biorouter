import * as React from 'react';

import { cn } from '../../utils';
import { InfoTip, plainTextOf, useInfoTipId } from './info-tip';

/**
 * Field: the one form-field recipe (spec 2.6), copied from Crew's `dialogs/fields.tsx`.
 *
 * A `text-label` label, a 6px gap, the control, then ONE `text-supporting` muted helper linked
 * by `aria-describedby`; an error replaces the helper in danger ink. The control is given `id`,
 * `aria-describedby` and `aria-invalid` unless it sets them itself.
 *
 * The wrapper is `.br-form-field`, never `.br-field`: that class is the text field's resting edge
 * (`--border-control`, main.css) and belongs on the `Input`, `Textarea` and Select trigger only.
 *
 * ```tsx
 * <Field id="workflow-name" label="Name" helper="Shown in the sidebar." error={nameError}>
 *   <Input value={name} onChange={…} />
 * </Field>
 * ```
 */

/** The id of a field's helper (or error) line, for `aria-describedby`. */
export const fieldHelpId = (fieldId: string) => `${fieldId}-help`;
/** The id of a field's label, for a control that names itself with `aria-labelledby`. */
export const fieldLabelId = (fieldId: string) => `${fieldId}-label`;

export interface FieldProps {
  /** The control's id; the label points at it and the helper is `fieldHelpId(id)`. */
  id: string;
  label: React.ReactNode;
  /**
   * Help behind an InfoTip beside the label (its sibling, never inside the `<label>`), also
   * linked to the control by `aria-describedby`. Plain text, two sentences at most.
   */
  info?: string;
  /**
   * The InfoTip's name ("About {infoLabel}") when `label` is not plain text. Without it the name
   * is the text the label node shows; it is never the field's id (vision V3-R3-3).
   */
  infoLabel?: string;
  /** One muted line under the control. Replaced by `error` while there is one. */
  helper?: React.ReactNode;
  /** The field's validation message (danger ink). */
  error?: React.ReactNode;
  /** Marks the control `aria-required`. */
  required?: boolean;
  children: React.ReactNode;
  className?: string;
}

export function Field({
  id,
  label,
  info,
  infoLabel,
  helper,
  error,
  required,
  children,
  className,
}: FieldProps) {
  const note = error ?? helper;
  const helpId = note ? fieldHelpId(id) : undefined;
  const generatedInfoId = useInfoTipId();
  const infoId = info ? generatedInfoId : undefined;
  // An accessible name is words a person can say, never a DOM id: "About workflow-prompt" was
  // what a node label (the workflow editor's "Initial prompt (optional)") used to announce.
  const infoName = infoLabel ?? (plainTextOf(label) || 'this field');
  const control = React.isValidElement<Record<string, unknown>>(children)
    ? React.cloneElement(children, {
        id: (children.props.id as string | undefined) ?? id,
        'aria-describedby':
          [children.props['aria-describedby'] as string | undefined, infoId, helpId]
            .filter(Boolean)
            .join(' ') || undefined,
        'aria-invalid': children.props['aria-invalid'] ?? (error ? true : undefined),
        'aria-required': children.props['aria-required'] ?? (required ? true : undefined),
      })
    : children;
  return (
    <div className={cn('br-form-field flex min-w-0 flex-col gap-1.5', className)} data-slot="field">
      {info ? (
        <div className="flex min-w-0 items-center">
          <label id={fieldLabelId(id)} htmlFor={id} className="text-label text-text-default">
            {label}
          </label>
          <InfoTip label={infoName} help={info} id={infoId} />
        </div>
      ) : (
        <label id={fieldLabelId(id)} htmlFor={id} className="text-label text-text-default">
          {label}
        </label>
      )}
      {control}
      {note ? (
        <p
          id={helpId}
          className={cn('text-supporting', error ? 'text-text-danger' : 'text-text-muted')}
          data-slot={error ? 'field-error' : 'field-helper'}
        >
          {note}
        </p>
      ) : null}
    </div>
  );
}
