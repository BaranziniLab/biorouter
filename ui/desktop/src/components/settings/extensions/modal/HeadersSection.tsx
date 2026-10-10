import React from 'react';
import { Button } from '../../../ui/button';
import { InfoTip } from '../../../ui/info-tip';
import { IconAction } from '../../../ui/row-actions';
import { Plus, X } from '../../../icons/app-icons';
import { Input } from '../../../ui/input';
import { cn } from '../../../../utils';
import { EXTENSION_MODAL_COPY as COPY } from '../copy';

interface HeadersSectionProps {
  headers: { key: string; value: string; isEdited?: boolean }[];
  onAdd: (key: string, value: string) => void;
  onRemove: (index: number) => void;
  onChange: (index: number, field: 'key' | 'value', value: string) => void;
  submitAttempted: boolean;
  onPendingInputChange: (
    hasPendingInput: boolean,
    pendingHeader: { key: string; value: string } | null
  ) => void;
}

export default function HeadersSection({
  headers,
  onAdd,
  onRemove,
  onChange,
  submitAttempted,
  onPendingInputChange,
}: HeadersSectionProps) {
  const [newKey, setNewKey] = React.useState('');
  const [newValue, setNewValue] = React.useState('');
  const [validationError, setValidationError] = React.useState<string | null>(null);
  const [invalidFields, setInvalidFields] = React.useState<{ key: boolean; value: boolean }>({
    key: false,
    value: false,
  });

  // Notify parent when pending input changes
  React.useEffect(() => {
    const hasPendingInput = newKey.trim() !== '' || newValue.trim() !== '';
    const pendingHeader =
      newKey.trim() && newValue.trim() ? { key: newKey, value: newValue } : null;
    onPendingInputChange(hasPendingInput, pendingHeader);
  }, [newKey, newValue, onPendingInputChange]);

  const handleAdd = () => {
    const keyEmpty = !newKey.trim();
    const valueEmpty = !newValue.trim();
    const keyHasSpaces = newKey.includes(' ');

    if (keyEmpty || valueEmpty) {
      setInvalidFields({
        key: keyEmpty,
        value: valueEmpty,
      });
      setValidationError(COPY.headerNeedsBoth);
      return;
    }

    if (keyHasSpaces) {
      setInvalidFields({
        key: true,
        value: false,
      });
      setValidationError(COPY.headerNoSpaces);
      return;
    }

    setValidationError(null);
    setInvalidFields({ key: false, value: false });
    onAdd(newKey, newValue);
    setNewKey('');
    setNewValue('');
  };

  const clearValidation = () => {
    setValidationError(null);
    setInvalidFields({ key: false, value: false });
  };

  const isFieldInvalid = (index: number, field: 'key' | 'value') => {
    if (!submitAttempted) return false;
    const value = headers[index][field].trim();
    return value === '';
  };

  const titleId = React.useId();
  const errorId = React.useId();
  const bad = 'border-border-danger focus:border-border-danger';

  return (
    <div className="br-ext-section" role="group" aria-labelledby={titleId}>
      <div className="br-ext-section-title">
        <h3 id={titleId} className="text-label text-text-default">
          {COPY.headersTitle}
        </h3>
        <InfoTip label={COPY.headersTitle} help={COPY.headersHelp} />
      </div>
      <div className="br-ext-kv br-ext-kv--headers">
        {/* Existing headers */}
        {headers.map((header, index) => (
          <React.Fragment key={index}>
            <Input
              value={header.key}
              onChange={(e) => onChange(index, 'key', e.target.value)}
              placeholder={COPY.headerName}
              aria-label={COPY.headerName}
              aria-invalid={isFieldInvalid(index, 'key') || undefined}
              className={cn('w-full', isFieldInvalid(index, 'key') && bad)}
            />
            <Input
              value={header.value}
              onChange={(e) => onChange(index, 'value', e.target.value)}
              placeholder={COPY.value}
              aria-label={COPY.value}
              aria-invalid={isFieldInvalid(index, 'value') || undefined}
              className={cn('w-full', isFieldInvalid(index, 'value') && bad)}
            />
            <IconAction
              icon={X}
              label={COPY.removeEntry(header.key)}
              onSelect={() => onRemove(index)}
            />
          </React.Fragment>
        ))}

        {/* Empty row with Add button */}
        <Input
          value={newKey}
          onChange={(e) => {
            setNewKey(e.target.value);
            clearValidation();
          }}
          placeholder={COPY.headerName}
          aria-label={COPY.headerName}
          aria-invalid={invalidFields.key || undefined}
          aria-describedby={validationError ? errorId : undefined}
          className={cn('w-full', invalidFields.key && bad)}
        />
        <Input
          value={newValue}
          onChange={(e) => {
            setNewValue(e.target.value);
            clearValidation();
          }}
          placeholder={COPY.value}
          aria-label={COPY.value}
          aria-invalid={invalidFields.value || undefined}
          aria-describedby={validationError ? errorId : undefined}
          className={cn('w-full', invalidFields.value && bad)}
        />
        <Button variant="secondary" onClick={handleAdd} className="br-ext-kv-add">
          <Plus aria-hidden />
          {COPY.add}
        </Button>
      </div>
      {validationError && (
        <p id={errorId} role="alert" className="text-supporting text-text-danger">
          {validationError}
        </p>
      )}
    </div>
  );
}
