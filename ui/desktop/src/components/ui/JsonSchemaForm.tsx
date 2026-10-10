import React, { useState, useCallback } from 'react';
import { Input } from './input';
import { Button } from './button';

interface JsonSchemaProperty {
  type?: string;
  description?: string;
  default?: unknown;
  enum?: string[];
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
}

export interface JsonSchema {
  type?: string;
  properties?: Record<string, JsonSchemaProperty>;
  required?: string[];
  title?: string;
  description?: string;
}

interface JsonSchemaFormProps {
  schema: JsonSchema;
  onSubmit: (data: Record<string, unknown>) => void;
  onCancel?: () => void;
  submitLabel?: string;
  cancelLabel?: string;
  disabled?: boolean;
}

export default function JsonSchemaForm({
  schema,
  onSubmit,
  onCancel,
  submitLabel = 'Submit',
  cancelLabel = 'Cancel',
  disabled = false,
}: JsonSchemaFormProps) {
  const [formData, setFormData] = useState<Record<string, unknown>>(() => {
    const initial: Record<string, unknown> = {};
    if (schema.properties) {
      for (const [key, prop] of Object.entries(schema.properties)) {
        if (prop.default !== undefined) {
          initial[key] = prop.default;
        } else if (prop.type === 'boolean') {
          initial[key] = false;
        } else if (prop.type === 'number' || prop.type === 'integer') {
          initial[key] = prop.minimum ?? 0;
        } else {
          initial[key] = '';
        }
      }
    }
    return initial;
  });

  const [errors, setErrors] = useState<Record<string, string>>({});

  const validateField = useCallback(
    (key: string, value: unknown): string | null => {
      const prop = schema.properties?.[key];
      if (!prop) return null;

      const isRequired = schema.required?.includes(key);

      if (isRequired && (value === '' || value === null || value === undefined)) {
        return 'This field is required';
      }

      if (prop.type === 'string' && typeof value === 'string') {
        if (!isRequired && value === '') return null;

        if (prop.minLength !== undefined && value.length < prop.minLength) {
          return `Minimum length is ${prop.minLength}`;
        }
        if (prop.maxLength !== undefined && value.length > prop.maxLength) {
          return `Maximum length is ${prop.maxLength}`;
        }
      }

      if ((prop.type === 'number' || prop.type === 'integer') && typeof value === 'number') {
        if (prop.minimum !== undefined && value < prop.minimum) {
          return `Minimum value is ${prop.minimum}`;
        }
        if (prop.maximum !== undefined && value > prop.maximum) {
          return `Maximum value is ${prop.maximum}`;
        }
      }

      return null;
    },
    [schema]
  );

  const handleChange = useCallback(
    (key: string, value: unknown) => {
      setFormData((prev) => ({ ...prev, [key]: value }));

      const error = validateField(key, value);
      setErrors((prev) => {
        if (error) {
          return { ...prev, [key]: error };
        }
        const newErrors = { ...prev };
        delete newErrors[key];
        return newErrors;
      });
    },
    [validateField]
  );

  const handleSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();

      const newErrors: Record<string, string> = {};
      if (schema.properties) {
        for (const key of Object.keys(schema.properties)) {
          const error = validateField(key, formData[key]);
          if (error) {
            newErrors[key] = error;
          }
        }
      }

      if (Object.keys(newErrors).length > 0) {
        setErrors(newErrors);
        return;
      }

      onSubmit(formData);
    },
    [formData, onSubmit, schema.properties, validateField]
  );

  const renderField = (key: string, prop: JsonSchemaProperty) => {
    const value = formData[key];
    const error = errors[key];
    const isRequired = schema.required?.includes(key);

    if (prop.enum) {
      return (
        <select
          id={key}
          value={String(value ?? '')}
          onChange={(e) => handleChange(key, e.target.value)}
          disabled={disabled}
          // The native <select> is a text field wearing a different tag, so it
          // takes the <Input> chrome exactly (§3.2): the 32px md rung, an 8px
          // inset, the derived `--border-emphasized` edge and the inset-ring
          // hover whisper. `main.css` already owns focus for `select` alongside
          // input and textarea, so the old `focus:border-border-strong` was a
          // second, weaker answer to a question already settled globally.
          className="flex h-8 w-full rounded-element border border-border-emphasized bg-background-default px-2 text-label transition-[color,background-color,border-color,box-shadow] hover:inset-ring-2 hover:inset-ring-border-emphasized/30 focus:inset-ring-0 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {!isRequired && <option value="">Select...</option>}
          {prop.enum.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
      );
    }

    if (prop.type === 'boolean') {
      return (
        <label className="flex items-center gap-2 cursor-pointer">
          {/* §3.3, and the same construction CustomRadio uses so the two controls
              are interchangeable in a form: a 22px visual box centred in a 24px
              target box. `accent-background-accent` is what actually binds a
              NATIVE checkbox to the theme — `border-*` and `rounded-*` on an
              `appearance: auto` control are inert, which is why the old classes
              looked like theming and were not. */}
          <span className="inline-flex h-6 w-6 shrink-0 items-center justify-center">
            <input
              type="checkbox"
              id={key}
              checked={Boolean(value)}
              onChange={(e) => handleChange(key, e.target.checked)}
              disabled={disabled}
              className="h-[22px] w-[22px] cursor-pointer accent-background-accent disabled:cursor-not-allowed"
            />
          </span>
          <span className="text-label text-text-default">{prop.description || key}</span>
        </label>
      );
    }

    if (prop.type === 'number' || prop.type === 'integer') {
      return (
        <Input
          type="number"
          id={key}
          value={String(value ?? '')}
          onChange={(e) => {
            const numValue =
              prop.type === 'integer' ? parseInt(e.target.value, 10) : parseFloat(e.target.value);
            handleChange(key, isNaN(numValue) ? '' : numValue);
          }}
          min={prop.minimum}
          max={prop.maximum}
          step={prop.type === 'integer' ? 1 : 'any'}
          disabled={disabled}
          className={error ? 'border-border-danger' : ''}
        />
      );
    }

    return (
      <Input
        type="text"
        id={key}
        value={String(value ?? '')}
        onChange={(e) => handleChange(key, e.target.value)}
        minLength={prop.minLength}
        maxLength={prop.maxLength}
        disabled={disabled}
        className={error ? 'border-border-danger' : ''}
      />
    );
  };

  if (!schema.properties || Object.keys(schema.properties).length === 0) {
    return <div className="text-text-muted text-body">No fields to display</div>;
  }

  return (
    <form onSubmit={handleSubmit} className="flex flex-col gap-4">
      {Object.entries(schema.properties).map(([key, prop]) => {
        const isRequired = schema.required?.includes(key);
        const error = errors[key];

        if (prop.type === 'boolean') {
          return (
            <div key={key} className="flex flex-col gap-1">
              {renderField(key, prop)}
              {error && <span className="text-text-danger text-supporting">{error}</span>}
            </div>
          );
        }

        return (
          <div key={key} className="flex flex-col gap-1">
            <label htmlFor={key} className="text-label text-text-default">
              {key}
              {isRequired && <span className="text-text-danger ml-1">*</span>}
            </label>
            {prop.description && prop.type !== 'boolean' && (
              <span className="text-supporting text-text-muted">{prop.description}</span>
            )}
            {renderField(key, prop)}
            {error && <span className="text-text-danger text-supporting">{error}</span>}
          </div>
        );
      })}

      <div className="flex justify-end gap-2">
        {onCancel && (
          <Button type="button" variant="ghost" size="sm" onClick={onCancel} disabled={disabled}>
            {cancelLabel}
          </Button>
        )}
        <Button type="submit" size="sm" disabled={disabled}>
          {submitLabel}
        </Button>
      </div>
    </form>
  );
}
