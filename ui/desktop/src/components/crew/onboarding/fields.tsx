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
  type RefObject,
} from 'react';
import CustomRadio from '../../ui/CustomRadio';
import { Disclosure } from '../../ui/disclosure';
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
 *
 * `mode={null}` is the unmade choice (an invitation that states no privacy): both rows unchecked,
 * nothing assumed. It is the same fieldset either way, so the first pick keeps keyboard focus on
 * the radio just chosen instead of swapping the rows for new ones and dropping it to the page.
 */
export function PrivacyFields({
  mode,
  institution,
  disabled,
  institutionHelper = joinCopy.institutionHelper,
  testId,
  onMode,
  onInstitution,
}: {
  mode: 'private' | 'public' | null;
  institution: string;
  disabled?: boolean;
  /** What to say under the institution: whom to ask, when the caller knows. */
  institutionHelper?: ReactNode;
  testId?: string;
  onMode: (mode: 'private' | 'public') => void;
  onInstitution: (institution: string) => void;
}) {
  const name = useId();
  return (
    <>
      <fieldset className="crew-onboard-radios" data-testid={testId}>
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

/** Whether a remote work folder holds a value its field would refuse (not an absolute path). */
export function remoteFolderInvalid(remoteRoot: string): boolean {
  const folder = remoteRoot.trim();
  return Boolean(folder) && !folder.startsWith('/');
}

/**
 * "Agent on {server}" (Q2-37): what the person's agent may do on the server, in its own labelled
 * row rather than among the SSH settings. Folded to one line that states it ("No work folder ·
 * agent commands off"), off unless the person turns it on. The folder lets the agent read and write
 * files there; the switch lets it run commands in that folder, and needs the folder first.
 */
export function AgentAccessFields({
  server,
  open,
  onOpenChange,
  remoteRoot,
  remoteExecution,
  disabled,
  onRemoteRoot,
  onRemoteExecution,
}: {
  server: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  remoteRoot: string;
  remoteExecution: boolean;
  disabled?: boolean;
  onRemoteRoot: (value: string) => void;
  onRemoteExecution: (value: boolean) => void;
}) {
  const folder = remoteRoot.trim();
  return (
    <Disclosure
      label={joinCopy.agentHeading(server || 'the server')}
      open={open}
      onOpenChange={onOpenChange}
      summary={joinCopy.agentSummary(folder, Boolean(folder) && remoteExecution)}
    >
      <div className="crew-onboard-form" data-testid="crew-onboard-agent">
        <Field
          label={joinCopy.remoteFolder}
          helper={joinCopy.remoteFolderHelper}
          invalidMessage={joinCopy.remoteFolderInvalid}
        >
          {(props) => (
            <Input
              {...props}
              disabled={disabled}
              pattern="/.*"
              value={remoteRoot}
              spellCheck={false}
              onChange={(event) => {
                onRemoteRoot(event.target.value);
                if (!event.target.value.trim()) onRemoteExecution(false);
              }}
            />
          )}
        </Field>
        <SwitchRow
          label={joinCopy.remoteExecution}
          checked={Boolean(folder) && remoteExecution}
          disabled={disabled || !folder}
          hint={folder ? undefined : joinCopy.remoteExecutionNeedsFolder}
          onCheckedChange={onRemoteExecution}
        />
      </div>
    </Disclosure>
  );
}

/** How long after a dialog opens its first field holds focus against the surface that opened it. */
export const INITIAL_FOCUS_HOLD_MS = 1000;

/**
 * Put focus on `target` when a dialog opens (Q2-27), and keep it there while the surface that
 * opened the dialog finishes closing. A menu item opens Join or Host, and the menu, closing a moment
 * later, hands focus back to its own trigger (or drops it to `<body>`), after the field's own
 * `autoFocus` already ran: the dialog opened with focus nowhere in it. For a short while after
 * opening, focus that lands outside the dialog, on `<body>`, or on the dialog's own frame is put
 * back on `target`. The person's first key or click ends it, so nothing they do is ever overridden.
 */
export function useInitialFocus(target: RefObject<HTMLElement | null>, active: boolean): void {
  useEffect(() => {
    if (!active || typeof document === 'undefined') return;
    const dialogOf = () =>
      target.current?.closest<HTMLElement>('[role="dialog"], [role="alertdialog"]') ?? null;
    const lost = () => {
      const current = document.activeElement;
      if (!current || current === document.body || current === document.documentElement)
        return true;
      if (!current.isConnected) return true;
      const dialog = dialogOf();
      return !dialog || current === dialog || !dialog.contains(current);
    };
    const reclaim = () => {
      const element = target.current;
      if (element?.isConnected && lost()) element.focus();
    };
    let frame: number | null = null;
    const schedule = () => {
      if (frame !== null) return;
      const run = () => {
        frame = null;
        reclaim();
      };
      frame =
        typeof window.requestAnimationFrame === 'function'
          ? window.requestAnimationFrame(run)
          : window.setTimeout(run, 0);
    };
    let stopped = false;
    const stop = () => {
      if (stopped) return;
      stopped = true;
      document.removeEventListener('focusin', schedule, true);
      document.removeEventListener('focusout', schedule, true);
      document.removeEventListener('keydown', stop, true);
      document.removeEventListener('pointerdown', stop, true);
      window.clearTimeout(timer);
      if (frame !== null) {
        if (typeof window.cancelAnimationFrame === 'function') window.cancelAnimationFrame(frame);
        window.clearTimeout(frame);
      }
    };
    reclaim();
    document.addEventListener('focusin', schedule, true);
    document.addEventListener('focusout', schedule, true);
    document.addEventListener('keydown', stop, true);
    document.addEventListener('pointerdown', stop, true);
    const timer = window.setTimeout(stop, INITIAL_FOCUS_HOLD_MS);
    return stop;
  }, [active, target]);
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
