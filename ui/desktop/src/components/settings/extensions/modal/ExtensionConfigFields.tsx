import { Input } from '../../../ui/input';
import { Field } from '../../../ui/field';
import { EXTENSION_MODAL_COPY as COPY } from '../copy';

interface ExtensionConfigFieldsProps {
  type: 'stdio' | 'sse' | 'streamable_http' | 'builtin';
  full_cmd: string;
  endpoint: string;
  onChange: (key: string, value: string) => void;
  submitAttempted?: boolean;
  isValid?: boolean;
}

export default function ExtensionConfigFields({
  type,
  full_cmd,
  endpoint,
  onChange,
  submitAttempted = false,
  isValid,
}: ExtensionConfigFieldsProps) {
  const invalid = submitAttempted && !isValid;

  if (type === 'stdio') {
    return (
      <Field
        id="extension-command"
        label={COPY.commandLabel}
        required
        error={invalid ? COPY.commandRequired : undefined}
      >
        {/* Monospace, because this is a COMMAND — the same value a configured extension runs. A
            body font here meant a command was proportional to edit and monospace everywhere it
            is shown. The endpoint field below is deliberately NOT changed: a URL is a different
            value class, and it is body font everywhere else too (e.g. LocalModelInventory's
            "Official URL"). The error edge is `aria-invalid`, which Field sets and Input draws. */}
        <Input
          machine
          value={full_cmd}
          onChange={(e) => onChange('cmd', e.target.value)}
          placeholder={COPY.commandPlaceholder}
          className="w-full font-mono text-code"
        />
      </Field>
    );
  }
  return (
    <Field
      id="extension-endpoint"
      label={COPY.endpointLabel}
      required
      error={invalid ? COPY.endpointRequired : undefined}
    >
      <Input
        machine
        value={endpoint}
        onChange={(e) => onChange('endpoint', e.target.value)}
        placeholder={COPY.endpointPlaceholder}
        className="w-full"
      />
    </Field>
  );
}
