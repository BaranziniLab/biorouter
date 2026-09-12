import React, { useState } from 'react';
import { Button } from '../../ui/button';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../../ui/dialog';

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
        setLocalError('Invalid JSON format');
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

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && handleCancel()}>
      <DialogContent className="flex max-h-[90vh] w-[800px] max-w-[90vw] flex-col overflow-hidden sm:max-w-[90vw] lg:max-w-[800px]">
        <div className="mb-4 pr-8">
          <DialogTitle>JSON Schema Editor</DialogTitle>
        </div>

        <div className="flex-1 flex flex-col min-h-0">
          <div className="mb-4">
            <div className="flex items-center justify-between mb-2">
              <label className="block text-label text-text-default">Response JSON Schema</label>
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
              Define the expected structure of the AI's response using JSON Schema format
            </DialogDescription>
          </div>

          <div className="flex-1 min-h-0">
            <textarea
              value={localValue}
              onChange={(e) => {
                setLocalValue(e.target.value);
                setLocalError('');
              }}
              className={`biorouter-modal-panel w-full h-full min-h-[400px] px-3 py-2 text-code rounded-element text-text-default placeholder:text-text-muted transition-colors resize-none font-mono ${localError || error ? '!border-border-danger' : ''}`}
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
            {(localError || error) && (
              <p className="text-text-danger text-body mt-2">{localError || error}</p>
            )}
          </div>
        </div>

        <div className="flex justify-end space-x-3 mt-6 pt-4 border-t border-border-subtle">
          <Button type="button" onClick={handleCancel} variant="ghost">
            Cancel
          </Button>
          <Button type="button" onClick={handleSave} variant="default">
            Save schema
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
