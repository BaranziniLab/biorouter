import { Input } from '../../../ui/input';
import { Field, fieldLabelId } from '../../../ui/field';
import { SegmentedControl, type SegmentedOption } from '../../../ui/segmented-control';
import { EXTENSION_MODAL_COPY as COPY } from '../copy';

type ExtensionType = 'stdio' | 'sse' | 'streamable_http' | 'builtin';

interface ExtensionInfoFieldsProps {
  name: string;
  type: ExtensionType;
  description: string;
  onChange: (key: string, value: string) => void;
  submitAttempted: boolean;
}

const TYPE_FIELD_ID = 'extension-type';

/**
 * Name, type and description. Each label points at its control, so every field has a name a
 * screen reader can say (they were unassociated `<label>`s), and a validation message sits in
 * the field's own flow instead of floating over whatever follows.
 *
 * The type is a choice of two short options, so it is the `SegmentedControl` and not a select
 * (spec principle 5); the sentence that tells them apart is the label's InfoTip. An existing
 * extension of the retired SSE type still says so, in a third, disabled segment.
 */
export default function ExtensionInfoFields({
  name,
  type,
  description,
  onChange,
  submitAttempted,
}: ExtensionInfoFieldsProps) {
  const isNameValid = () => {
    return name.trim() !== '';
  };

  const options: SegmentedOption<string>[] = [
    { value: 'stdio', label: COPY.typeCommand },
    { value: 'streamable_http', label: COPY.typeHttp },
    ...(type === 'sse'
      ? [{ value: 'sse', label: COPY.typeSse, disabled: true } as SegmentedOption<string>]
      : []),
  ];

  return (
    <>
      <div className="br-ext-identity">
        <Field
          id="extension-name"
          label={COPY.nameLabel}
          required
          error={submitAttempted && !isNameValid() ? COPY.nameRequired : undefined}
        >
          <Input machine value={name} onChange={(e) => onChange('name', e.target.value)} />
        </Field>

        <Field id={TYPE_FIELD_ID} label={COPY.typeLabel} info={COPY.typeHelp}>
          <SegmentedControl
            options={options}
            value={type}
            onValueChange={(next) => onChange('type', next)}
            aria-labelledby={fieldLabelId(TYPE_FIELD_ID)}
          />
        </Field>
      </div>

      <Field id="extension-description" label={COPY.descriptionLabel}>
        <Input value={description} onChange={(e) => onChange('description', e.target.value)} />
      </Field>
    </>
  );
}
