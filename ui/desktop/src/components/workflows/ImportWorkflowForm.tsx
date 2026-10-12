import { useState, useCallback, useRef } from 'react';
import { Upload } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Spinner } from '../ui/spinner';
import { ModalShell } from '../ModalShell';
import { toastSuccess } from '../../toasts';
import { saveWorkflow } from '../../workflow/workflow_management';
import { parseWorkflow } from '../../api';
import type { Workflow } from '../../workflow';
import { workflowsCopy } from './copy';
import './workflows.css';

const copy = workflowsCopy.importDialog;

interface ImportWorkflowFormProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: () => void;
}

async function parseWorkflowFromFile(fileContent: string): Promise<Workflow> {
  try {
    const response = await parseWorkflow({
      body: { content: fileContent },
      throwOnError: true,
    });
    return response.data.workflow;
  } catch (error) {
    const msg =
      typeof error === 'object' && error !== null && 'message' in error
        ? (error as { message: string }).message
        : 'Unknown error';
    throw new Error(msg);
  }
}

export default function ImportWorkflowForm({
  isOpen,
  onClose,
  onSuccess,
}: ImportWorkflowFormProps) {
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const reset = () => {
    setError('');
    setIsDragging(false);
    setIsSubmitting(false);
  };

  const handleClose = () => {
    reset();
    onClose();
  };

  const browse = () => {
    if (!isSubmitting) fileInputRef.current?.click();
  };

  const processContent = useCallback(
    async (content: string, filename: string) => {
      const isYaml = /\.(ya?ml)$/i.test(filename);
      const isJson = /\.json$/i.test(filename);
      if (!isYaml && !isJson) {
        setError(copy.wrongType);
        return;
      }
      setError('');
      setIsSubmitting(true);
      try {
        const workflow = await parseWorkflowFromFile(content);
        await saveWorkflow(workflow, null);
        reset();
        onClose();
        onSuccess();
        toastSuccess({ title: copy.imported(workflow.title?.trim() || 'workflow') });
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setIsSubmitting(false);
      }
    },
    [onClose, onSuccess]
  );

  const processFile = useCallback(
    async (file: File) => {
      const content = await file.text();
      await processContent(content, file.name);
    },
    [processContent]
  );

  const handleDrop = useCallback(
    (e: React.DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setIsDragging(false);
      const file = e.dataTransfer.files[0];
      if (file) processFile(file);
    },
    [processFile]
  );

  const handleFileInputChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (file) processFile(file);
      e.target.value = '';
    },
    [processFile]
  );

  return (
    <ModalShell
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isSubmitting) handleClose();
      }}
      size="md"
      purpose="form"
      title={copy.title}
      footer={
        <Button variant="secondary" onClick={handleClose} disabled={isSubmitting}>
          {copy.cancel}
        </Button>
      }
    >
      <div className="flex flex-col gap-2 pb-1">
        {/* The drop zone says what to do in one line, so the dialog carries no
            description repeating it (Knowledge's and Add skill's recipe, V3-08). It is a
            button to the keyboard too: Enter or Space opens the file picker. */}
        <div
          role="button"
          tabIndex={isSubmitting ? -1 : 0}
          aria-disabled={isSubmitting || undefined}
          aria-describedby={error ? 'import-workflow-error' : undefined}
          data-state={isDragging ? 'dragging' : error ? 'error' : undefined}
          onDragOver={(e) => {
            e.preventDefault();
            setIsDragging(true);
          }}
          onDragLeave={() => setIsDragging(false)}
          onDrop={handleDrop}
          onClick={browse}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              browse();
            }
          }}
          className="br-workflow-dropzone"
        >
          <input
            ref={fileInputRef}
            type="file"
            accept=".yaml,.yml,.json"
            onChange={handleFileInputChange}
            className="hidden"
          />
          {isSubmitting ? (
            <span role="status" className="flex items-center gap-2 text-body text-text-muted">
              <Spinner size={16} />
              {copy.importing}
            </span>
          ) : (
            <>
              <span aria-hidden className="br-workflow-drop-plate">
                <Upload className="size-4" />
              </span>
              <span className="text-label text-text-default">{copy.drop}</span>
            </>
          )}
        </div>

        {error && (
          <p id="import-workflow-error" role="alert" className="text-supporting text-text-danger">
            {error}
          </p>
        )}
      </div>
    </ModalShell>
  );
}
