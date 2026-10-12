import React from 'react';
import { Button } from '../../../ui/button';
import { InfoTip } from '../../../ui/info-tip';
import { IconAction } from '../../../ui/row-actions';
import { Plus, X, Pencil } from '../../../icons/app-icons';
import { Input } from '../../../ui/input';
import { cn } from '../../../../utils';
import { EXTENSION_MODAL_COPY as COPY } from '../copy';

const SECRET_PLACEHOLDER = '••••••••';

interface EnvVarsSectionProps {
  envVars: { key: string; value: string; isEdited?: boolean }[];
  onAdd: (key: string, value: string) => void;
  onRemove: (index: number) => void;
  onChange: (index: number, field: 'key' | 'value', value: string) => void;
  submitAttempted: boolean;
  onPendingInputChange?: (hasPendingInput: boolean) => void;
}

export default function EnvVarsSection({
  envVars,
  onAdd,
  onRemove,
  onChange,
  submitAttempted,
  onPendingInputChange,
}: EnvVarsSectionProps) {
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
    onPendingInputChange?.(hasPendingInput);
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
      setValidationError(COPY.variableNeedsBoth);
      return;
    }

    if (keyHasSpaces) {
      setInvalidFields({
        key: true,
        value: false,
      });
      setValidationError(COPY.variableNoSpaces);
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
    const value = envVars[index][field].trim();
    return value === '';
  };

  const handleEdit = (index: number) => {
    // Mark this env var as edited
    onChange(index, 'value', envVars[index].value === '••••••••' ? '' : envVars[index].value);

    // Mark as edited in the parent component
    const updatedEnvVar = {
      ...envVars[index],
      isEdited: true,
    };

    // Update the envVars array with the edited flag
    const newEnvVars = [...envVars];
    newEnvVars[index] = updatedEnvVar;
  };

  const titleId = React.useId();
  const errorId = React.useId();

  // An invalid cell's red edge is the Input's own: `aria-invalid` draws it, focused or not.
  return (
    <div className="br-ext-section" role="group" aria-labelledby={titleId}>
      <div className="br-ext-section-title">
        <h3 id={titleId} className="text-label text-text-default">
          {COPY.envTitle}
        </h3>
        <InfoTip label={COPY.envTitle} help={COPY.envHelp} />
      </div>
      <div className="br-ext-kv">
        {/* Existing environment variables */}
        {envVars.map((envVar, index) => {
          const saved = envVar.value === SECRET_PLACEHOLDER && !envVar.isEdited;
          return (
            <React.Fragment key={index}>
              <Input
                machine
                value={envVar.key}
                onChange={(e) => onChange(index, 'key', e.target.value)}
                placeholder={COPY.variableName}
                aria-label={COPY.variableName}
                aria-invalid={isFieldInvalid(index, 'key') || undefined}
                className="w-full"
              />
              <Input
                machine
                value={envVar.value}
                readOnly={saved}
                onChange={(e) => {
                  // If this is the first edit of a placeholder value, clear it
                  const newValue = saved ? '' : e.target.value;
                  onChange(index, 'value', newValue);
                }}
                placeholder={COPY.value}
                aria-label={COPY.value}
                aria-invalid={isFieldInvalid(index, 'value') || undefined}
                className={cn('w-full', saved && 'text-text-muted opacity-60 cursor-not-allowed')}
              />
              {saved ? (
                <IconAction
                  icon={Pencil}
                  label={COPY.edit(envVar.key)}
                  onSelect={() => handleEdit(index)}
                />
              ) : (
                <span aria-hidden="true" /> /* keeps the grid's edit column */
              )}
              <IconAction
                icon={X}
                label={COPY.removeEntry(envVar.key)}
                onSelect={() => onRemove(index)}
              />
            </React.Fragment>
          );
        })}

        {/* Empty row with Add button */}
        <Input
          machine
          value={newKey}
          onChange={(e) => {
            setNewKey(e.target.value);
            clearValidation();
          }}
          placeholder={COPY.variableName}
          aria-label={COPY.variableName}
          aria-invalid={invalidFields.key || undefined}
          aria-describedby={validationError ? errorId : undefined}
          className="w-full"
        />
        <Input
          machine
          value={newValue}
          onChange={(e) => {
            setNewValue(e.target.value);
            clearValidation();
          }}
          placeholder={COPY.value}
          aria-label={COPY.value}
          aria-invalid={invalidFields.value || undefined}
          aria-describedby={validationError ? errorId : undefined}
          className="w-full"
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
