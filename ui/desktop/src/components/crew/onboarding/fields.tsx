import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
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
}

/** A label, its control and an optional helper that turns danger with the field. */
export function Field({
  label,
  helper,
  invalid = false,
  children,
}: {
  label: ReactNode;
  helper?: ReactNode;
  invalid?: boolean;
  children: (props: FieldControlProps) => ReactNode;
}) {
  const id = useId();
  const helperId = `${id}-helper`;
  return (
    <div className="crew-onboard-field">
      <label htmlFor={id} className="text-label text-text-default">
        {label}
      </label>
      {children({
        id,
        ...(helper ? { 'aria-describedby': helperId } : {}),
        ...(invalid ? { 'aria-invalid': true } : {}),
      })}
      {helper ? (
        <p
          id={helperId}
          className={cn('text-supporting', invalid ? 'text-text-danger' : 'text-text-muted')}
        >
          {helper}
        </p>
      ) : null}
    </div>
  );
}

/** A switch with its label beside it. Options that change behavior are switches, never checkboxes. */
export function SwitchRow({
  label,
  checked,
  disabled,
  onCheckedChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="crew-onboard-row">
      <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={onCheckedChange} />
      <label htmlFor={id} className="text-label text-text-default">
        {label}
      </label>
    </div>
  );
}

/**
 * Private / Public radio rows and the institution a Private connection needs. The institution is
 * required only while Private is chosen, and its value survives a trip to Public and back.
 */
export function PrivacyFields({
  mode,
  institution,
  disabled,
  onMode,
  onInstitution,
}: {
  mode: 'private' | 'public';
  institution: string;
  disabled?: boolean;
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
          helper={institution.trim() ? undefined : joinCopy.institutionHelper}
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
