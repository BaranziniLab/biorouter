import { Input } from '../../../ui/input';
import { Field } from '../../../ui/field';
import { EXTENSION_MODAL_COPY as COPY } from '../copy';

interface ExtensionTimeoutFieldProps {
  timeout: number;
  onChange: (key: string, value: string | number) => void;
  submitAttempted: boolean;
}

/**
 * How long a call to the extension may run, in seconds.
 *
 * ⚠ This field's error used to read "Timeout " (a sentence cut off at its first word), was
 * `absolute` so it floated over whatever came next, and the label gave no unit. The unit is in
 * the label now and the sentence is whole, in the field's own flow.
 */
export default function ExtensionTimeoutField({
  timeout,
  onChange,
  submitAttempted,
}: ExtensionTimeoutFieldProps) {
  const isTimeoutValid = () => {
    // Check if timeout is not undefined, null, or empty string
    if (timeout === undefined || timeout === null) {
      return false;
    }

    // Convert to number if it's a string
    const timeoutValue = typeof timeout === 'string' ? Number(timeout) : timeout;

    // Check if it's a valid number (not NaN) and is a positive number
    return !isNaN(timeoutValue) && timeoutValue > 0;
  };

  return (
    <Field
      id="extension-timeout"
      label={COPY.timeoutLabel}
      error={submitAttempted && !isTimeoutValid() ? COPY.timeoutRequired : undefined}
    >
      <Input
        value={timeout}
        inputMode="numeric"
        // A number of seconds is a short field; its message is not, and stays whole beside it.
        className="br-ext-timeout-input"
        onChange={(e) => onChange('timeout', e.target.value)}
      />
    </Field>
  );
}
