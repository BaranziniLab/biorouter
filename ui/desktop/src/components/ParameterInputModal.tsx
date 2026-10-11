import React, { useState, useEffect } from 'react';
import { Parameter } from '../workflow';
import { Button } from './ui/button';
import { getInitialWorkingDir } from '../utils/workingDir';
import { ModalShell } from './ModalShell';
import { Field } from './ui/field';
import { Input } from './ui/input';
import { NativeSelect } from './parameter/NativeSelect';

const copy = {
  title: 'Workflow parameters',
  start: 'Start workflow',
  cancel: 'Cancel',
  optional: '(optional)',
  chooseOption: 'Choose…',
  yes: 'True',
  no: 'False',
  required: (label: string) => `${label} is required`,
  cancelTitle: 'Cancel workflow setup?',
  cancelMessage: 'Your parameter values have not been submitted yet.',
  newChat: 'Start a new chat',
  backToForm: 'Back to the form',
} as const;

interface ParameterInputModalProps {
  parameters: Parameter[];
  onSubmit: (values: Record<string, string>) => void;
  onClose: () => void;
  initialValues?: Record<string, string>;
}

const ParameterInputModal: React.FC<ParameterInputModalProps> = ({
  parameters,
  onSubmit,
  onClose,
  initialValues,
}) => {
  const [inputValues, setInputValues] = useState<Record<string, string>>({});
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const [showCancelOptions, setShowCancelOptions] = useState(false);

  // Pre-fill the form with default values from the workflow and initialValues from deeplink
  useEffect(() => {
    const defaultValues: Record<string, string> = {};
    parameters.forEach((param) => {
      if (param.requirement === 'optional' && param.default) {
        defaultValues[param.key] =
          param.input_type === 'boolean' ? param.default.toLowerCase() : param.default;
      }
    });

    setInputValues({ ...defaultValues, ...initialValues });
  }, [parameters, initialValues]);

  const handleChange = (name: string, value: string): void => {
    setInputValues((prevValues: Record<string, string>) => ({ ...prevValues, [name]: value }));
  };

  const handleSubmit = (): void => {
    // Clear previous validation errors
    setValidationErrors({});

    // Check if all *required* parameters are filled
    const requiredParams: Parameter[] = parameters.filter((p) => p.requirement === 'required');
    const errors: Record<string, string> = {};

    requiredParams.forEach((param) => {
      const value = inputValues[param.key]?.trim();
      if (!value) {
        errors[param.key] = copy.required(param.description || param.key);
      }
    });

    if (Object.keys(errors).length > 0) {
      setValidationErrors(errors);
      return;
    }

    onSubmit(inputValues);
  };

  const handleCancel = (): void => {
    // Always show cancel options if workflow has any parameters (required or optional)
    const hasAnyParams = parameters.length > 0;

    if (hasAnyParams) {
      setShowCancelOptions(true);
    } else {
      onClose();
    }
  };

  const handleCancelOption = (option: 'new-chat' | 'back-to-form'): void => {
    if (option === 'new-chat') {
      try {
        const workingDir = getInitialWorkingDir();
        window.electron.createChatWindow(undefined, workingDir);
        window.electron.hideWindow();
      } catch (error) {
        console.error('Error creating new window:', error);
        onClose();
      }
    } else {
      setShowCancelOptions(false); // Go back to the parameter form
    }
  };

  // Two states, one shell. The cancel prompt is a confirmation (width S,
  // dismissible: backing out of it is free); the parameter form is a form
  // (width M) whose typed values a stray backdrop click must not discard.
  if (showCancelOptions) {
    return (
      <ModalShell
        open
        onOpenChange={(open) => !open && setShowCancelOptions(false)}
        size="sm"
        purpose="info"
        title={copy.cancelTitle}
        footer={
          <>
            <Button onClick={() => handleCancelOption('new-chat')} variant="secondary">
              {copy.newChat}
            </Button>
            <Button autoFocus onClick={() => handleCancelOption('back-to-form')}>
              {copy.backToForm}
            </Button>
          </>
        }
      >
        <p className="py-1 text-body text-text-muted">{copy.cancelMessage}</p>
      </ModalShell>
    );
  }

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && handleCancel()}
      size="md"
      purpose="form"
      scrollBody
      title={copy.title}
      footer={
        <>
          <Button type="button" onClick={handleCancel} variant="secondary">
            {copy.cancel}
          </Button>
          <Button type="button" onClick={handleSubmit}>
            {copy.start}
          </Button>
        </>
      }
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          handleSubmit();
        }}
        className="flex flex-col gap-4 py-4"
      >
        {parameters.map((param, index) => {
          const id = `workflow-parameter-${index}`;
          const name = param.description || param.key;
          const label =
            param.requirement === 'required' ? (
              name
            ) : (
              <>
                {name} <span className="font-normal text-text-muted">{copy.optional}</span>
              </>
            );
          const value = inputValues[param.key] || '';
          return (
            <Field
              key={param.key}
              id={id}
              label={label}
              error={validationErrors[param.key]}
              required={param.requirement === 'required'}
            >
              {param.input_type === 'select' && param.options ? (
                <NativeSelect
                  value={value}
                  onChange={(e) => handleChange(param.key, e.target.value)}
                >
                  <option value="">{copy.chooseOption}</option>
                  {param.options.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </NativeSelect>
              ) : param.input_type === 'boolean' ? (
                <NativeSelect
                  value={value}
                  onChange={(e) => handleChange(param.key, e.target.value)}
                >
                  <option value="">{copy.chooseOption}</option>
                  <option value="true">{copy.yes}</option>
                  <option value="false">{copy.no}</option>
                </NativeSelect>
              ) : (
                <Input
                  type={param.input_type === 'number' ? 'number' : 'text'}
                  value={value}
                  onChange={(e) => handleChange(param.key, e.target.value)}
                  placeholder={param.default || undefined}
                />
              )}
            </Field>
          );
        })}
      </form>
    </ModalShell>
  );
};

export default ParameterInputModal;
