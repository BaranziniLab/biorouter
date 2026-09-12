import React, { useState } from 'react';
import { Button } from '../../ui/button';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../../ui/dialog';

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

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && handleCancel()}>
      <DialogContent className="flex max-h-[90vh] w-[900px] max-w-[90vw] flex-col overflow-hidden sm:max-w-[90vw] lg:max-w-[900px]">
        <div className="mb-4 pr-8">
          <DialogTitle>Instructions editor</DialogTitle>
        </div>

        <div className="flex-1 flex flex-col min-h-0">
          <div className="mb-4">
            <div className="flex items-center justify-between mb-2">
              <label className="block text-label text-text-default">Instructions</label>
              <Button
                type="button"
                onClick={insertExample}
                variant="ghost"
                size="sm"
                className="text-supporting"
              >
                Insert example
              </Button>
            </div>
            <DialogDescription className="text-supporting text-text-muted mb-3">
              Use{' '}
              <code className="bg-background-muted px-1 rounded-inner">{`{{parameter_name}}`}</code>{' '}
              syntax to define parameters that users can fill in
            </DialogDescription>
          </div>

          <div className="flex-1 min-h-0">
            <textarea
              value={localValue}
              onChange={(e) => setLocalValue(e.target.value)}
              className={`biorouter-modal-panel w-full h-full min-h-[500px] px-3 py-2 text-code rounded-element text-text-default placeholder:text-text-muted transition-colors resize-none font-mono ${error ? '!border-border-danger' : ''}`}
              placeholder="Detailed instructions for the AI, hidden from the user"
            />
            {error && <p className="text-text-danger text-body mt-2">{error}</p>}
          </div>
        </div>

        <div className="flex justify-end space-x-3 mt-6 pt-4 border-t border-border-subtle">
          <Button type="button" onClick={handleCancel} variant="ghost">
            Cancel
          </Button>
          <Button type="button" onClick={handleSave} variant="default">
            Save instructions
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
