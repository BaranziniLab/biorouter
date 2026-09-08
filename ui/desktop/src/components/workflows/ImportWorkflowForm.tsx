import { useState, useCallback, useRef } from 'react';
import { Upload } from '../icons/app-icons';
import { Button } from '../ui/button';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog';
import { MODAL_SIZE } from '../ModalShell';
import { toastSuccess } from '../../toasts';
import { saveWorkflow } from '../../workflow/workflow_management';
import { parseWorkflow } from '../../api';
import type { Workflow } from '../../workflow';

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

  const processContent = useCallback(
    async (content: string, filename: string) => {
      const isYaml = /\.(ya?ml)$/i.test(filename);
      const isJson = /\.json$/i.test(filename);
      if (!isYaml && !isJson) {
        setError('Choose a YAML or JSON workflow file.');
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
        toastSuccess({
          title: workflow.title?.trim() || 'Workflow',
          msg: 'Workflow imported',
        });
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
    <Dialog open={isOpen} onOpenChange={(open) => !open && !isSubmitting && handleClose()}>
      <DialogContent dismissible={!isSubmitting} className={MODAL_SIZE.md}>
        <DialogHeader>
          <DialogTitle>Import Workflow</DialogTitle>
          <DialogDescription>
            Drag and drop a workflow YAML or JSON file, or click to browse.
          </DialogDescription>
        </DialogHeader>

        <div
          onDragOver={(e) => {
            e.preventDefault();
            setIsDragging(true);
          }}
          onDragLeave={() => setIsDragging(false)}
          onDrop={handleDrop}
          onClick={() => !isSubmitting && fileInputRef.current?.click()}
          className={[
            'biorouter-modal-panel flex flex-col items-center justify-center gap-2 rounded-container py-10 cursor-pointer transition-colors select-none',
            isDragging
              ? '!border-block-teal bg-block-teal/5'
              : error
                ? '!border-border-danger bg-background-danger/10'
                : 'hover:!border-border-strong hover:bg-overlay-hover',
          ].join(' ')}
        >
          <input
            ref={fileInputRef}
            type="file"
            accept=".yaml,.yml,.json"
            onChange={handleFileInputChange}
            className="hidden"
          />
          {isSubmitting ? (
            <p className="text-body text-text-muted animate-pulse">Importing…</p>
          ) : (
            <>
              <Upload className="w-8 h-8 text-text-muted" />
              <p className="text-label text-text-default">Drop a YAML or JSON file here</p>
              <p className="text-supporting text-text-muted">or click to browse</p>
            </>
          )}
        </div>

        {error && <p className="text-body text-text-danger">{error}</p>}

        <DialogFooter>
          <Button variant="outline" onClick={handleClose} disabled={isSubmitting}>
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function ImportWorkflowButton({ onClick }: { onClick: () => void }) {
  return (
    // Variant only. `buttonVariants`' base already emits `inline-flex
    // items-center justify-center gap-2` and sizes a bare svg to 16px, and a
    // `flex` on the call site FLIPS that `inline-flex` through tailwind-merge —
    // which is how a row action elsewhere in the app became a full-width bar
    // (vocabulary V7).
    <Button onClick={onClick} variant="outline">
      <Upload />
      Import Workflow
    </Button>
  );
}
