import React, { useState, useCallback, useRef } from 'react';
import { AlertCircle, Upload } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { MODAL_SIZE } from '../ModalShell';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog';

interface ImportSessionModalProps {
  isOpen: boolean;
  onClose: () => void;
  onImport: (json: string) => Promise<void>;
}

export function ImportSessionModal({ isOpen, onClose, onImport }: ImportSessionModalProps) {
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

  const processFile = useCallback(
    async (file: File) => {
      if (!file.name.endsWith('.json') && file.type !== 'application/json') {
        setError('Choose a JSON file.');
        return;
      }
      setError('');
      setIsSubmitting(true);
      try {
        const json = await file.text();
        JSON.parse(json);
        await onImport(json);
        reset();
        onClose();
      } catch (e) {
        setError(e instanceof SyntaxError ? 'Invalid JSON file.' : String(e));
        setIsSubmitting(false);
      }
    },
    [onImport, onClose]
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
      {/* V8 — the ladder's form rung by name. The literal it replaces was
          already 480px, so this is the same width said in the language the
          other dialogs are read in. */}
      <DialogContent dismissible={!isSubmitting} className={MODAL_SIZE.md}>
        <DialogHeader>
          <DialogTitle>Import chat</DialogTitle>
          <DialogDescription>Drag and drop a chat JSON file, or click to browse.</DialogDescription>
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
            // V4 — status is a WASH, never a hand-mixed alpha. These were
            // `bg-block-teal/5` and `bg-background-danger/10`: two different
            // mixes of two different hues, neither derived per family or per
            // mode, so the drop target read as a faint teal smear in Parchment
            // and as almost nothing in Roche Limit dark. `--wash-*` is the same
            // 22% formula `Note` and `Badge` use and is generated per family.
            isDragging
              ? '!border-border-info bg-wash-info'
              : error
                ? '!border-border-danger bg-wash-danger'
                : 'hover:!border-border-strong tint-interactive',
          ].join(' ')}
        >
          <input
            ref={fileInputRef}
            type="file"
            accept=".json,application/json"
            onChange={handleFileInputChange}
            className="hidden"
          />
          {/* V6 — roles, not sizes. `text-sm font-medium` IS `text-label`
              exactly, and `text-xs` was a fourth size beside the three roles
              this dialog already uses. */}
          {isSubmitting ? (
            <p className="text-supporting text-text-muted animate-pulse">Importing…</p>
          ) : (
            <>
              <Upload className="w-8 h-8 text-text-muted" />
              <p className="text-label text-text-default">Drop a JSON file here</p>
              <p className="text-supporting text-text-muted">or click to browse</p>
            </>
          )}
        </div>

        {/* V4 — an inline error is a `Note`, like every other in-place prose
            block in the app. `role="alert"` because this one just failed. */}
        {error && (
          <Note tone="danger" role="alert" icon={AlertCircle}>
            {error}
          </Note>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={handleClose} disabled={isSubmitting}>
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
