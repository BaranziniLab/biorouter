import React, { useState } from 'react';
import { Button } from '../../ui/button';
import { Textarea } from '../../ui/textarea';
import { ModalShell } from '../../ModalShell';
import { workflowFormCopy } from '../copy';
import '../workflows.css';

const copy = workflowFormCopy.schemaEditor;

interface JsonSchemaEditorProps {
  isOpen: boolean;
  onClose: () => void;
  value: string;
  onChange: (value: string) => void;
  error?: string;
}

export default function JsonSchemaEditor({
  isOpen,
  onClose,
  value,
  onChange,
  error,
}: JsonSchemaEditorProps) {
  const [localValue, setLocalValue] = useState(value);
  const [localError, setLocalError] = useState('');

  React.useEffect(() => {
    if (isOpen) {
      setLocalValue(value);
      setLocalError('');
    }
  }, [isOpen, value]);

  const handleSave = () => {
    if (localValue.trim()) {
      try {
        JSON.parse(localValue.trim());
        setLocalError('');
      } catch {
        setLocalError(copy.invalidJson);
        return;
      }
    }

    onChange(localValue);
    onClose();
  };

  const handleCancel = () => {
    setLocalValue(value);
    setLocalError('');
    onClose();
  };

  const insertExample = () => {
    const example = `{
 "type": "object",
 "properties": {
 "result": {
 "type": "string",
 "description": "The main result"
 },
 "status": {
 "type": "string",
 "enum": ["success", "error"],
 "description": "Operation status"
 },
 "data": {
 "type": "object",
 "properties": {
 "items": {
 "type": "array",
 "items": {
 "type": "string"
 }
 }
 }
 }
 },
 "required": ["result", "status"]
}`;
    setLocalValue(example);
  };

  if (!isOpen) return null;

  const shownError = localError || error;

  // The editor rung (`xl`, 800px), shared with the instructions editor. What the schema is
  // for is said once, in the form's InfoTip beside "Response schema".
  return (
    <ModalShell
      open={isOpen}
      onOpenChange={(open) => !open && handleCancel()}
      size="xl"
      purpose="form"
      scrollBody
      title={copy.title}
      footer={
        <>
          <Button type="button" onClick={handleCancel} variant="ghost">
            {copy.cancel}
          </Button>
          <Button type="button" onClick={handleSave}>
            {copy.save}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-2 py-4">
        <div className="flex justify-end">
          <Button type="button" onClick={insertExample} variant="ghost" size="sm">
            {copy.insertExample}
          </Button>
        </div>
        <Textarea
          aria-label={copy.title}
          value={localValue}
          onChange={(e) => {
            setLocalValue(e.target.value);
            setLocalError('');
          }}
          aria-invalid={shownError ? true : undefined}
          aria-describedby={shownError ? 'schema-editor-error' : undefined}
          className="br-workflow-editor-field font-mono text-code"
          placeholder={`{
 "type": "object",
 "properties": {
 "result": {
 "type": "string",
 "description": "The main result"
 }
 },
 "required": ["result"]
}`}
        />
        {shownError && (
          <p id="schema-editor-error" role="alert" className="text-supporting text-text-danger">
            {shownError}
          </p>
        )}
      </div>
    </ModalShell>
  );
}
