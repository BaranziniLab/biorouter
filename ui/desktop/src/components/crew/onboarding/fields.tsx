import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from 'react';
import CustomRadio from '../../ui/CustomRadio';
import { Input } from '../../ui/input';
import { Switch } from '../../ui/switch';
import { cn } from '../../../utils';
import { INSTITUTION_ID_PATTERN } from '../identity';
import { joinCopy } from './copy';

/** The props a `Field` hands its control, so the label, helper and invalid state are wired once. */
export interface FieldControlProps {
  id: string;
  'aria-describedby'?: string;
  'aria-invalid'?: boolean;
  'aria-required'?: boolean;
  /** What `useFormValidation` says under the field when the value breaks its rule. */
  'data-invalid-message'?: string;
  /** What `useFormValidation` says under the field when it is required and empty. */
  'data-required-message'?: string;
}

/** The messages a form's own validation recorded, by control id (`useFormValidation`). */
const FieldErrorsContext = createContext<Readonly<Record<string, string>>>({});

/** Wrap a form's fields so each `Field` shows the message its control failed with. */
export const FieldErrorsProvider = FieldErrorsContext.Provider;

/**
 * A label, its control, an optional helper that turns danger with the field, and the message the
 * form's own validation recorded for it. `required` marks the field visibly ("Required", beside
 * the label and outside it, so the control's name stays the label's words) and sets
 * `aria-required` on the control.
 */
export function Field({
  label,
  helper,
  invalid = false,
  required = false,
  requiredMessage,
  invalidMessage,
  live = false,
  children,
}: {
  label: ReactNode;
  helper?: ReactNode;
  invalid?: boolean;
  required?: boolean;
  /** Said under the field when it is required and empty. Defaults to "Fill this in to continue." */
  requiredMessage?: string;
  /** Said under the field when its value breaks the control's rule (a pattern, a range). */
  invalidMessage?: string;
  /** The helper reports a result that arrives later: keep it a polite live region. */
  live?: boolean;
  children: (props: FieldControlProps) => ReactNode;
}) {
  const id = useId();
  const helperId = `${id}-helper`;
  const errorId = `${id}-error`;
  const error = useContext(FieldErrorsContext)[id];
  const hasHelper = helper !== undefined && helper !== null && helper !== false && helper !== '';
  const describedBy = [hasHelper ? helperId : null, error ? errorId : null]
    .filter(Boolean)
    .join(' ');
  return (
    <div className="crew-onboard-field">
      <div className="crew-onboard-label-row">
        <label htmlFor={id} className="text-label text-text-default">
          {label}
        </label>
        {required ? (
          <span className="text-supporting text-text-muted" aria-hidden="true">
            {joinCopy.required}
          </span>
        ) : null}
      </div>
      {children({
        id,
        ...(describedBy ? { 'aria-describedby': describedBy } : {}),
        ...(invalid || error ? { 'aria-invalid': true } : {}),
        ...(required ? { 'aria-required': true } : {}),
        ...(invalidMessage ? { 'data-invalid-message': invalidMessage } : {}),
        ...(requiredMessage ? { 'data-required-message': requiredMessage } : {}),
      })}
      {hasHelper || live ? (
        <p
          id={helperId}
          aria-live={live ? 'polite' : undefined}
          className={cn('text-supporting', invalid ? 'text-text-danger' : 'text-text-muted')}
        >
          {helper}
        </p>
      ) : null}
      {error ? (
        <p id={errorId} className="text-supporting text-text-danger">
          {error}
        </p>
      ) : null}
    </div>
  );
}

type Control = HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;

function isControl(target: unknown): target is Control {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  );
}

/** The sentence under a failed control: its own custom message, else the one its field named. */
function messageFor(control: Control): string {
  const { validity } = control;
  if (validity.customError) return control.validationMessage || joinCopy.fieldInvalid;
  if (validity.valueMissing) return control.dataset.requiredMessage || joinCopy.fieldRequired;
  return control.dataset.invalidMessage || joinCopy.fieldInvalid;
}

/**
 * The onboarding forms' own validation, in place of Chromium's native bubble (T-11, T-43): the
 * form is `noValidate`, `validate()` checks every mounted control, puts each failure's sentence
 * under its field (12px danger, linked by `aria-describedby`) and focuses the first. A message
 * clears as soon as its control holds a valid value again. The controls keep their `required`,
 * `pattern`, `min`/`max` and custom validity: those still define what is valid.
 */
export function useFormValidation() {
  const formRef = useRef<HTMLFormElement>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});

  const validate = useCallback((): boolean => {
    const form = formRef.current;
    if (!form) return true;
    const invalid = Array.from(form.elements).filter(
      (element): element is Control =>
        isControl(element) && !element.disabled && element.willValidate && !element.validity.valid
    );
    const next: Record<string, string> = {};
    for (const control of invalid) if (control.id) next[control.id] = messageFor(control);
    setErrors(next);
    invalid[0]?.focus();
    return invalid.length === 0;
  }, []);

  // Something else asked the browser to validate (`reportValidity`): no bubble, the same message.
  const onInvalid = useCallback((event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const target = event.target;
    if (!isControl(target) || !target.id) return;
    const message = messageFor(target);
    setErrors((current) =>
      current[target.id] === message ? current : { ...current, [target.id]: message }
    );
  }, []);

  const onInput = useCallback((event: FormEvent<HTMLFormElement>) => {
    const target = event.target;
    if (!isControl(target) || !target.id) return;
    setErrors((current) => {
      if (!(target.id in current)) return current;
      if (target.validity.valid) {
        const next = { ...current };
        delete next[target.id];
        return next;
      }
      const message = messageFor(target);
      return current[target.id] === message ? current : { ...current, [target.id]: message };
    });
  }, []);

  return {
    errors,
    validate,
    formProps: { ref: formRef, noValidate: true, onInvalid, onInput },
  };
}

/** A switch with its label beside it. Options that change behavior are switches, never checkboxes. */
export function SwitchRow({
  label,
  checked,
  disabled,
  hint,
  onCheckedChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  /** Why the switch is unavailable, shown under it and linked as its description. */
  hint?: string;
  onCheckedChange: (checked: boolean) => void;
}) {
  const id = useId();
  const hintId = `${id}-hint`;
  return (
    <div className="crew-onboard-field">
      <div className="crew-onboard-row">
        <Switch
          id={id}
          checked={checked}
          disabled={disabled}
          aria-describedby={hint ? hintId : undefined}
          onCheckedChange={onCheckedChange}
        />
        <label htmlFor={id} className="text-label text-text-default">
          {label}
        </label>
      </div>
      {hint ? (
        <p id={hintId} className="text-supporting text-text-muted">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

/**
 * Private / Public radio rows and the institution a Private connection needs. The institution is
 * required only while Private is chosen, marked so, and its value survives a trip to Public and
 * back. Its helper stays put while the person types, so the dialog never jumps by a line.
 */
export function PrivacyFields({
  mode,
  institution,
  disabled,
  institutionHelper = joinCopy.institutionHelper,
  onMode,
  onInstitution,
}: {
  mode: 'private' | 'public';
  institution: string;
  disabled?: boolean;
  /** What to say under the institution: whom to ask, when the caller knows. */
  institutionHelper?: ReactNode;
  onMode: (mode: 'private' | 'public') => void;
  onInstitution: (institution: string) => void;
}) {
  const name = useId();
  return (
    <>
      <fieldset className="crew-onboard-radios">
        <legend className="text-label text-text-default">{joinCopy.privacy}</legend>
        <CustomRadio
          id={`${name}-private`}
          name={name}
          value="private"
          checked={mode === 'private'}
          disabled={disabled}
          onChange={() => onMode('private')}
          label={joinCopy.private}
          secondaryLabel={joinCopy.privateHint}
        />
        <CustomRadio
          id={`${name}-public`}
          name={name}
          value="public"
          checked={mode === 'public'}
          disabled={disabled}
          onChange={() => onMode('public')}
          label={joinCopy.public}
          secondaryLabel={joinCopy.publicHint}
        />
      </fieldset>
      {mode === 'private' ? (
        <Field
          label={joinCopy.institution}
          helper={institutionHelper}
          required
          requiredMessage={joinCopy.institutionRequired}
          invalidMessage={joinCopy.institutionInvalid}
        >
          {(props) => (
            <Input
              {...props}
              required
              disabled={disabled}
              pattern={INSTITUTION_ID_PATTERN}
              placeholder={joinCopy.institutionPlaceholder}
              value={institution}
              spellCheck={false}
              autoComplete="off"
              onChange={(event) => onInstitution(event.target.value)}
            />
          )}
        </Field>
      ) : null}
    </>
  );
}

/**
 * Remount a dialog's content each time it opens, so no state from an earlier visit survives
 * (L14), while the closing dialog keeps its content through the exit animation. Returns a key.
 */
export function useOpenGeneration(open: boolean): number {
  const [generation, setGeneration] = useState(0);
  const wasOpen = useRef(open);
  useEffect(() => {
    if (wasOpen.current && !open) setGeneration((value) => value + 1);
    wasOpen.current = open;
  }, [open]);
  return generation;
}

/** Tracks whether the component is still mounted, for work that finishes after an await. */
export function useMounted() {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  return mounted;
}
