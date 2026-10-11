import React, { useState } from 'react';
import { Button } from '../../ui/button';
import { Textarea } from '../../ui/textarea';
import { ModalShell } from '../../ModalShell';
import { workflowFormCopy } from '../copy';
import '../workflows.css';

const copy = workflowFormCopy.instructionsEditor;

interface InstructionsEditorProps {
  isOpen: boolean;
  onClose: () => void;
  value: string;
  onChange: (value: string) => void;
  error?: string;
}

export default function InstructionsEditor({
  isOpen,
  onClose,
  value,
  onChange,
  error,
}: InstructionsEditorProps) {
  const [localValue, setLocalValue] = useState(value);

  React.useEffect(() => {
    if (isOpen) {
      setLocalValue(value);
    }
  }, [isOpen, value]);

  const handleSave = () => {
    onChange(localValue);
    onClose();
  };

  const handleCancel = () => {
    setLocalValue(value); // Reset to original value
    onClose();
  };

  const insertExample = () => {
    const example = `You are an AI assistant helping with {{task_type}}. 

Follow these steps:
1. Analyze the provided {{input_data}}
2. Apply the specified {{methodology}} 
3. Generate a comprehensive report

Requirements:
- Be thorough and accurate
- Use clear, professional language
- Include specific examples where relevant
- Provide actionable recommendations

Format your response with:
- Executive summary
- Detailed analysis
- Key findings
- Next steps

Use {{parameter_name}} syntax for any user-provided values.`;
    setLocalValue(example);
  };

  if (!isOpen) return null;

  // The editor rung of the dialog ladder (`xl`, 800px), shared with the response-schema
  // editor. What `{{parameter_name}}` does is said once, in the form's InfoTip beside the
  // Instructions label; this view is the long text and nothing else.
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
          <Button type="button" onClick={handleCancel} variant="secondary">
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
          onChange={(e) => setLocalValue(e.target.value)}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? 'instructions-editor-error' : undefined}
          className="br-workflow-editor-field"
          placeholder={workflowFormCopy.instructionsPlaceholder}
        />
        {error && (
          <p id="instructions-editor-error" className="text-supporting text-text-danger">
            {error}
          </p>
        )}
      </div>
    </ModalShell>
  );
}
