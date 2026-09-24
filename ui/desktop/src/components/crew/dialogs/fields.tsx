import * as React from 'react';
import CustomRadio from '../../ui/CustomRadio';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { AlertTriangle } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import type { ErrorSource } from '../state/types';
import { refusalText } from './refusals';

/**
 * The form pieces every Crew dialog shares, so one recipe (settings vocabulary rule 6) holds
 * across them: a `text-label` label, a `text-supporting` helper that turns danger with the field's
 * error, and the one error slot a dialog owns.
 */

/** The id of a field's helper line, for `aria-describedby`. */
export const helpId = (fieldId: string) => `${fieldId}-help`;
/** The id of a field's label, for a control that names itself with `aria-labelledby`. */
export const labelId = (fieldId: string) => `${fieldId}-label`;

export interface FieldProps {
  /** The control's id; the label points at it and the helper is `helpId(id)`. */
  id: string;
  label: React.ReactNode;
  /** A muted line under the control. Replaced by `error` while there is one. */
  helper?: React.ReactNode;
  /** The field's own validation message (danger ink). */
  error?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}

export function Field({ id, label, helper, error, children, className }: FieldProps) {
  const note = error ?? helper;
  return (
    <div className={cn('flex min-w-0 flex-col gap-1.5', className)}>
      <label id={labelId(id)} htmlFor={id} className="text-label text-text-default">
        {label}
      </label>
      {children}
      {note ? (
        <p
          id={helpId(id)}
          className={cn('text-supporting', error ? 'text-text-danger' : 'text-text-muted')}
        >
          {note}
        </p>
      ) : null}
    </div>
  );
}

export type AdornedInputProps = React.ComponentProps<typeof Input> & {
  /** A fixed mark before the value (`@` for a username, `#` for a channel). Not part of the value. */
  adornment: string;
};

/** An `Input` with a leading mark that reads as part of the field but is never typed or sent. */
export const AdornedInput = React.forwardRef<HTMLInputElement, AdornedInputProps>(
  ({ adornment, className, ...props }, ref) => (
    <div className="relative min-w-0">
      <span
        aria-hidden
        className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-label text-text-muted"
      >
        {adornment}
      </span>
      <Input ref={ref} className={cn('pl-6', className)} {...props} />
    </div>
  )
);
AdornedInput.displayName = 'AdornedInput';

export interface RadioRowOption<T extends string> {
  value: T;
  label: string;
  detail?: string;
}

export interface RadioRowsProps<T extends string> {
  /** The group's visible label; also its accessible name. */
  label: string;
  name: string;
  value: T;
  options: readonly RadioRowOption<T>[];
  onChange(value: T): void;
  disabled?: boolean;
}

/** A labelled radio group of `CustomRadio` rows: the Private / Public and content choices. */
export function RadioRows<T extends string>({
  label,
  name,
  value,
  options,
  onChange,
  disabled,
}: RadioRowsProps<T>) {
  const groupId = React.useId();
  return (
    <div role="radiogroup" aria-labelledby={`${groupId}-label`} className="flex flex-col">
      <span id={`${groupId}-label`} className="text-label text-text-default">
        {label}
      </span>
      {options.map((option) => (
        <CustomRadio
          key={option.value}
          id={`${groupId}-${option.value}`}
          name={name}
          value={option.value}
          checked={value === option.value}
          disabled={disabled}
          onChange={() => onChange(option.value)}
          label={option.label}
          secondaryLabel={option.detail}
        />
      ))}
    </div>
  );
}

/**
 * The one place a dialog shows an action error, and only while the controller routes that error
 * here (`useCrewErrorSlot`): a failure from a dialog that has since closed falls back to the
 * connection bar, so every error still renders exactly once. `render` rewords a refusal the copy
 * deck has its own sentence for; the text is always its own node.
 */
export function DialogErrorNote({
  source,
  render = refusalText,
  className,
}: {
  source: ErrorSource;
  render?: (message: string) => string;
  className?: string;
}) {
  const crew = useCrew();
  const here = useCrewErrorSlot(source);
  if (!here || !crew.error) return null;
  return (
    <Note tone="danger" role="alert" icon={AlertTriangle} className={className}>
      <span>{render(crew.error.message)}</span>
    </Note>
  );
}

/** The controller's error when it is routed to `source`, for a dialog that renders it on a field. */
export function useDialogError(source: ErrorSource): string | null {
  const crew = useCrew();
  const here = useCrewErrorSlot(source);
  return here && crew.error ? crew.error.message : null;
}

/** A dialog error the caller already resolved to words (for example one not shown on a field). */
export function ErrorNote({ text, className }: { text: string; className?: string }) {
  return (
    <Note tone="danger" role="alert" icon={AlertTriangle} className={className}>
      <span>{text}</span>
    </Note>
  );
}

/**
 * A native validity message kept in step with `problem`, so the form's own validation refuses the
 * value with the reason — the same mechanism as `required` and `pattern`.
 */
export function useCustomValidity<T extends HTMLInputElement | HTMLTextAreaElement>(
  problem: string | null
): React.RefObject<T | null> {
  const ref = React.useRef<T | null>(null);
  React.useEffect(() => {
    ref.current?.setCustomValidity(problem ?? '');
  }, [problem]);
  return ref;
}

/**
 * Clear the controller's error only when it is one of `sources` — a dialog starting over clears
 * its own refusal, never an error some other surface (the connection bar) is showing.
 */
export function useDismissOwnError(...sources: ErrorSource[]): () => void {
  const { error, dismissError } = useCrew();
  return () => {
    if (error && sources.includes(error.source)) dismissError();
  };
}
