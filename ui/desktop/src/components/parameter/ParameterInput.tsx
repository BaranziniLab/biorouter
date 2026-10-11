import React from 'react';
import { AlertTriangle, Trash2, ChevronRight } from '../icons/app-icons';
import { Parameter } from '../../workflow';
import { Button } from '../ui/button';
import { Field } from '../ui/field';
import { InfoTip } from '../ui/info-tip';
import { Input } from '../ui/input';
import { NativeSelect } from './NativeSelect';
import { Textarea } from '../ui/textarea';
import { cn } from '../../utils';

interface ParameterInputProps {
  parameter: Parameter;
  onChange: (name: string, updatedParameter: Partial<Parameter>) => void;
  onDelete?: (parameterKey: string) => void;
  isUnused?: boolean;
  isExpanded?: boolean;
  onToggleExpanded?: (parameterKey: string) => void;
}

export const parameterCopy = {
  description: 'Description',
  descriptionInfo: 'Shown to the person running the workflow.',
  descriptionPlaceholder: (key: string) => `What to ask for ${key}`,
  inputType: 'Input type',
  requirement: 'Requirement',
  defaultValue: 'Default value',
  options: 'Options',
  optionsInfo: 'One per line. Each line becomes a choice.',
  unused: 'Unused',
  unusedInfo: 'Not used in the instructions, the prompt or the starter buttons.',
  showDetails: (key: string) => `Show details for ${key}`,
  hideDetails: (key: string) => `Hide details for ${key}`,
  delete: (key: string) => `Delete parameter ${key}`,
  types: { string: 'String', select: 'Select', number: 'Number', boolean: 'Boolean' },
  requirements: { required: 'Required', optional: 'Optional' },
} as const;

const copy = parameterCopy;

/**
 * One workflow parameter: a filled card whose header names it (`{{key}}`, the machine string in
 * mono), flags it when nothing uses it, and can delete it; the details below edit what the
 * person running the workflow is asked. The card is the subtle fill with a transparent edge and
 * no outline (R-27), and it is a `.br-fill-surface`, so the fields and selects inside it rest
 * one ground back on the page colour. Header and details are told apart by space.
 */
const ParameterInput: React.FC<ParameterInputProps> = ({
  parameter,
  onChange,
  onDelete,
  isUnused = false,
  isExpanded = true,
  onToggleExpanded,
}) => {
  const { key, description, requirement } = parameter;
  const defaultValue = parameter.default || '';
  const idBase = `parameter-${key.replace(/[^a-zA-Z0-9_-]/g, '-')}`;

  return (
    <div className="parameter-input br-fill-surface overflow-hidden rounded-element border border-transparent bg-background-fill">
      <div
        className={cn(
          'flex min-h-10 items-center justify-between gap-2 px-2',
          onToggleExpanded && 'cursor-pointer hover:bg-background-fill-strong'
        )}
        onClick={(e) => {
          if (onToggleExpanded && !(e.target as HTMLElement).closest('button')) {
            onToggleExpanded(key);
          }
        }}
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {onToggleExpanded && (
            <Button
              type="button"
              variant="ghost"
              shape="round"
              size="sm"
              aria-expanded={isExpanded}
              // The Disclosure's chevron recipe: it turns 90° under `[data-state='open']`.
              data-state={isExpanded ? 'open' : 'closed'}
              aria-label={isExpanded ? copy.hideDetails(key) : copy.showDetails(key)}
              onClick={(e) => {
                e.stopPropagation();
                onToggleExpanded(key);
              }}
            >
              <ChevronRight aria-hidden className="biorouter-disclosure-chevron" />
            </Button>
          )}

          <code className="min-w-0 truncate rounded-inner bg-background-default px-1.5 font-mono text-code text-text-default">
            {'{{'}
            <span>{key}</span>
            {'}}'}
          </code>

          {isUnused && (
            <InfoTip label={copy.unused} help={copy.unusedInfo} asChild>
              <span className="inline-flex shrink-0 items-center gap-1 rounded-inner">
                <AlertTriangle aria-hidden className="size-3.5 text-text-warning" />
                <span className="text-supporting text-text-warning">{copy.unused}</span>
              </span>
            </InfoTip>
          )}
        </div>

        {onDelete && (
          <Button
            type="button"
            variant="ghost"
            shape="round"
            size="sm"
            className="text-text-muted hover:text-text-danger"
            aria-label={copy.delete(key)}
            onClick={(e) => {
              e.stopPropagation();
              onDelete(key);
            }}
          >
            <Trash2 aria-hidden />
          </Button>
        )}
      </div>

      {isExpanded && (
        <div className="flex flex-col gap-3 p-3">
          <Field id={`${idBase}-description`} label={copy.description} info={copy.descriptionInfo}>
            <Input
              type="text"
              value={description || ''}
              onChange={(e) => onChange(key, { description: e.target.value })}
              placeholder={copy.descriptionPlaceholder(key)}
            />
          </Field>

          <div className="grid grid-cols-2 gap-3">
            <Field id={`${idBase}-type`} label={copy.inputType}>
              <NativeSelect
                value={parameter.input_type || 'string'}
                onChange={(e) =>
                  onChange(key, { input_type: e.target.value as Parameter['input_type'] })
                }
              >
                <option value="string">{copy.types.string}</option>
                <option value="select">{copy.types.select}</option>
                <option value="number">{copy.types.number}</option>
                <option value="boolean">{copy.types.boolean}</option>
              </NativeSelect>
            </Field>

            <Field id={`${idBase}-requirement`} label={copy.requirement}>
              <NativeSelect
                value={requirement}
                onChange={(e) =>
                  onChange(key, { requirement: e.target.value as Parameter['requirement'] })
                }
              >
                <option value="required">{copy.requirements.required}</option>
                <option value="optional">{copy.requirements.optional}</option>
              </NativeSelect>
            </Field>
          </div>

          {requirement === 'optional' && (
            <Field id={`${idBase}-default`} label={copy.defaultValue}>
              <Input
                type="text"
                value={defaultValue}
                onChange={(e) => onChange(key, { default: e.target.value })}
              />
            </Field>
          )}

          {parameter.input_type === 'select' && (
            <Field id={`${idBase}-options`} label={copy.options} info={copy.optionsInfo}>
              <Textarea
                value={(parameter.options || []).join('\n')}
                onChange={(e) => onChange(key, { options: e.target.value.split('\n') })}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') e.stopPropagation();
                }}
                placeholder={'Option 1\nOption 2\nOption 3'}
                rows={3}
              />
            </Field>
          )}
        </div>
      )}
    </div>
  );
};

export default ParameterInput;
