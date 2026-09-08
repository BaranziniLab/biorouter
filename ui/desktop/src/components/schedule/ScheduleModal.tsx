import React, { useState, useEffect, FormEvent } from 'react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Note } from '../ui/note';
import { ScheduledJob } from '../../schedule';
import { CronPicker } from './CronPicker';
import { getStorageDirectory } from '../../workflow/workflow_management';
import { Folder } from '../icons/app-icons';
import { ModalShell } from '../ModalShell';
import { scheduleDisplayName } from '../../utils/builtins';

export interface NewSchedulePayload {
  id: string;
  workflow_source: string;
  cron: string;
  execution_mode?: string;
}

interface ScheduleModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSubmit: (payload: NewSchedulePayload | string) => Promise<void>;
  schedule: ScheduledJob | null;
  isLoadingExternally: boolean;
  apiErrorExternally: string | null;
  initialDeepLink?: string | null;
}

const FIELD_LABEL = 'mb-1.5 block text-label text-text-default';

/**
 * Create or edit a schedule.
 *
 * It renders through `ModalShell` rather than assembling its own dialog: the
 * shell owns the size ladder, the header/body/footer geometry and — the reason
 * that matters here — Astryx's `purpose` axis. This is a FORM, so a stray
 * backdrop click must not throw away a half-filled one, which is exactly the
 * bug `purpose="form"` exists to prevent.
 *
 * ⚠ **`md`, not `sm`.** The `sm` rung is documented as "confirmations and
 * single-decision notices", and the cron picker beside it is deliberately one
 * inline sentence of controls; 400px folds that sentence onto three lines and
 * turns the thing back into a stack of boxes. `md` is the ladder's own form
 * rung and is what the dialog's old ad-hoc `max-w-md` was reaching for.
 *
 * `purpose` flips to `required` while a save is in flight, so a dismissal
 * cannot orphan a create that the daemon is already running.
 */
export const ScheduleModal: React.FC<ScheduleModalProps> = ({
  isOpen,
  onClose,
  onSubmit,
  schedule,
  isLoadingExternally,
  apiErrorExternally,
}) => {
  const isEditMode = !!schedule;

  const [scheduleId, setScheduleId] = useState<string>('');
  const [workflowSourcePath, setWorkflowSourcePath] = useState<string>('');
  const [cronExpression, setCronExpression] = useState<string>('0 0 14 * * *');
  const [internalValidationError, setInternalValidationError] = useState<string | null>(null);
  const [isValid, setIsValid] = useState(true);

  useEffect(() => {
    if (isOpen) {
      if (schedule) {
        setScheduleId(schedule.id);
        setCronExpression(schedule.cron);
      } else {
        setScheduleId('');
        setWorkflowSourcePath('');
        setCronExpression('0 0 14 * * *');
        setInternalValidationError(null);
      }
    }
  }, [isOpen, schedule]);

  const handleBrowseFile = async () => {
    const defaultPath = getStorageDirectory(true);
    const filePath = await window.electron.selectFileOrDirectory(defaultPath);
    if (filePath) {
      if (filePath.endsWith('.yaml') || filePath.endsWith('.yml')) {
        setWorkflowSourcePath(filePath);
        setInternalValidationError(null);
      } else {
        setInternalValidationError('Invalid file type: choose a YAML file (.yaml or .yml)');
      }
    }
  };

  const handleLocalSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (isLoadingExternally) return;
    setInternalValidationError(null);

    if (isEditMode) {
      await onSubmit(cronExpression);
      return;
    }

    if (!scheduleId.trim()) {
      setInternalValidationError('Schedule ID is required.');
      return;
    }

    if (!workflowSourcePath) {
      setInternalValidationError('Workflow source file is required.');
      return;
    }

    const finalWorkflowSource = workflowSourcePath;

    const newSchedulePayload: NewSchedulePayload = {
      id: scheduleId.trim(),
      workflow_source: finalWorkflowSource,
      cron: cronExpression,
    };

    await onSubmit(newSchedulePayload);
  };

  if (!isOpen) return null;

  return (
    <ModalShell
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isLoadingExternally) onClose();
      }}
      size="md"
      purpose={isLoadingExternally ? 'required' : 'form'}
      title={isEditMode ? 'Edit schedule' : 'New schedule'}
      subtitle={schedule ? scheduleDisplayName(schedule.id) : undefined}
      scrollBody
      footer={
        <>
          <Button type="button" variant="outline" onClick={onClose} disabled={isLoadingExternally}>
            Cancel
          </Button>
          <Button type="submit" form="schedule-form" disabled={isLoadingExternally || !isValid}>
            {isLoadingExternally
              ? isEditMode
                ? 'Saving…'
                : 'Creating…'
              : isEditMode
                ? 'Save changes'
                : 'Create schedule'}
          </Button>
        </>
      }
    >
      <form id="schedule-form" onSubmit={handleLocalSubmit} className="flex flex-col gap-5 py-1">
        {apiErrorExternally && (
          <Note tone="danger" role="alert">
            {apiErrorExternally}
          </Note>
        )}
        {internalValidationError && (
          <Note tone="danger" role="alert">
            {internalValidationError}
          </Note>
        )}

        {!isEditMode && (
          <>
            <div>
              <label htmlFor="scheduleId-modal" className={FIELD_LABEL}>
                Name
              </label>
              <Input
                type="text"
                id="scheduleId-modal"
                value={scheduleId}
                onChange={(e) => setScheduleId(e.target.value)}
                placeholder="e.g., daily-summary-job"
                required
              />
            </div>

            <div>
              {/* One `Input` with a trailing ghost Browse button, NOT an input
                  wrapped in a `biorouter-modal-panel` — a bordered panel around
                  a bordered field inside a bordered dialog was the box in a box
                  in a box, in miniature. */}
              <label htmlFor="workflowSource-modal" className={FIELD_LABEL}>
                Workflow file
              </label>
              <div className="flex items-center gap-2">
                <Input
                  type="text"
                  id="workflowSource-modal"
                  value={workflowSourcePath}
                  onChange={(e) => {
                    setWorkflowSourcePath(e.target.value);
                    setInternalValidationError(null);
                  }}
                  placeholder="/path/to/workflow.yaml"
                  className="font-mono"
                />
                <Button
                  type="button"
                  variant="ghost"
                  shape="round"
                  onClick={handleBrowseFile}
                  title="Browse for a YAML file"
                  aria-label="Browse for a YAML file"
                >
                  <Folder />
                </Button>
              </div>
              <p className="mt-1.5 text-supporting text-text-muted">
                Select a YAML workflow file (.yaml or .yml)
              </p>
            </div>
          </>
        )}

        <div>
          <span className={FIELD_LABEL}>Schedule</span>
          <CronPicker schedule={schedule} onChange={setCronExpression} isValid={setIsValid} />
        </div>
      </form>
    </ModalShell>
  );
};
