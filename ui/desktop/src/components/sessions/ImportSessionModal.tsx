import React, { useState, useCallback, useRef } from 'react';
import { AlertCircle, Upload } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { ModalShell } from '../ModalShell';
import {
  CANCEL,
  IMPORTING,
  IMPORT_BROWSE,
  IMPORT_DROP,
  IMPORT_INVALID_JSON,
  IMPORT_NOT_JSON,
  IMPORT_TITLE,
} from './copy';

interface ImportSessionModalProps {
  isOpen: boolean;
  onClose: () => void;
  onImport: (json: string) => Promise<void>;
}

/**
 * Import a chat from an exported JSON file. One drop target that is also a
 * button, so the file picker is reachable from the keyboard as well as by
 * dragging a file onto it; the target's own two lines say what to do, so the
 * dialog carries no description paragraph repeating them.
 */
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
        setError(IMPORT_NOT_JSON);
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
        setError(e instanceof SyntaxError ? IMPORT_INVALID_JSON : String(e));
        setIsSubmitting(false);
      }
    },
    [onImport, onClose]
  );

  const handleDrop = useCallback(
    (e: React.DragEvent<HTMLButtonElement>) => {
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
      onOpenChange={(open) => !open && !isSubmitting && handleClose()}
      size="md"
      purpose={isSubmitting ? 'required' : 'info'}
      title={IMPORT_TITLE}
      footer={
        <Button variant="secondary" onClick={handleClose} disabled={isSubmitting}>
          {CANCEL}
        </Button>
      }
    >
      <div className="flex flex-col gap-3">
        <button
          type="button"
          data-testid="import-drop-target"
          disabled={isSubmitting}
          onDragOver={(e) => {
            e.preventDefault();
            setIsDragging(true);
          }}
          onDragLeave={() => setIsDragging(false)}
          onDrop={handleDrop}
          onClick={() => fileInputRef.current?.click()}
          className={[
            'biorouter-modal-panel biorouter-focus-surface flex w-full select-none flex-col items-center justify-center gap-2 rounded-container py-10 transition-colors duration-[var(--dur-fast)] ease-[var(--ease-out)]',
            // Status is a wash (`--wash-*`, generated per family), never a hand-mixed alpha.
            isDragging
              ? '!border-border-info bg-wash-info'
              : error
                ? '!border-border-danger bg-wash-danger'
                : 'hover:!border-border-strong tint-interactive',
          ].join(' ')}
        >
          {isSubmitting ? (
            <span className="text-supporting text-text-muted">{IMPORTING}</span>
          ) : (
            <>
              <Upload className="h-5 w-5 text-text-muted" aria-hidden="true" />
              <span className="text-label text-text-default">{IMPORT_DROP}</span>
              <span className="text-supporting text-text-muted">{IMPORT_BROWSE}</span>
            </>
          )}
        </button>
        <input
          ref={fileInputRef}
          type="file"
          accept=".json,application/json"
          onChange={handleFileInputChange}
          className="hidden"
          tabIndex={-1}
          aria-hidden="true"
        />

        {/* An inline error is a `Note`, like every other in-place prose block in the
            app. `role="alert"` because this one just failed. */}
        {error && (
          <Note tone="danger" role="alert" icon={AlertCircle}>
            {error}
          </Note>
        )}
      </div>
    </ModalShell>
  );
}
