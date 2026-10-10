import React, { useState, useEffect, FormEvent } from 'react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Note } from '../ui/note';
import { Field, fieldHelpId } from '../ui/field';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { ScheduledJob } from '../../schedule';
import { CronPicker } from './CronPicker';
import { getStorageDirectory } from '../../workflow/workflow_management';
import { Folder } from '../icons/app-icons';
import { ModalShell } from '../ModalShell';
import { scheduleDisplayName } from '../../utils/builtins';
import { scheduleNameProblem } from './scheduleName';
import { scheduleCopy } from './copy';

const copy = scheduleCopy.modal;

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
 * rung.
 *
 * Every field is the shared `Field` (Crew's recipe): a `text-label` label, the
 * control, then ONE line that is the helper or, when something is wrong, the
 * error in its place. The name's format rule stays visible as its helper (spec
 * 3.10): a hidden format rule costs a round trip.
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
  const [nameError, setNameError] = useState<string | null>(null);
  const [workflowError, setWorkflowError] = useState<string | null>(null);
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
      }
      setNameError(null);
      setWorkflowError(null);
    }
  }, [isOpen, schedule]);

  const handleBrowseFile = async () => {
    const defaultPath = getStorageDirectory(true);
    const filePath = await window.electron.selectFileOrDirectory(defaultPath);
    if (filePath) {
      if (filePath.endsWith('.yaml') || filePath.endsWith('.yml')) {
        setWorkflowSourcePath(filePath);
        setWorkflowError(null);
      } else {
        setWorkflowError(copy.wrongFileType);
      }
    }
  };

  const handleLocalSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (isLoadingExternally) return;
    setNameError(null);
    setWorkflowError(null);

    if (isEditMode) {
      await onSubmit(cronExpression);
      return;
    }

    // The rule is stated here so the user reads it at the field, not after a
    // round trip. The daemon's `validate_schedule_id` remains the authority —
    // it closes an arbitrary-file write — and its refusal reaches the Note at
    // the top through `createSchedule`, so a drift between the two costs a
    // round trip rather than an unreadable error. See `scheduleName.ts`.
    const nameProblem = scheduleNameProblem(scheduleId.trim());
    if (nameProblem) {
      setNameError(nameProblem);
      return;
    }

    if (!workflowSourcePath) {
      setWorkflowError(copy.workflowRequired);
      return;
    }

    const newSchedulePayload: NewSchedulePayload = {
      id: scheduleId.trim(),
      workflow_source: workflowSourcePath,
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
      title={isEditMode ? copy.editTitle : copy.newTitle}
      subtitle={schedule ? scheduleDisplayName(schedule.id) : undefined}
      scrollBody
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose} disabled={isLoadingExternally}>
            {copy.cancel}
          </Button>
          <Button type="submit" form="schedule-form" disabled={isLoadingExternally || !isValid}>
            {isLoadingExternally
              ? isEditMode
                ? copy.saving
                : copy.creating
              : isEditMode
                ? copy.save
                : copy.create}
          </Button>
        </>
      }
    >
      <form id="schedule-form" onSubmit={handleLocalSubmit} className="flex flex-col gap-4 py-1">
        {apiErrorExternally && (
          <Note tone="danger" role="alert">
            {apiErrorExternally}
          </Note>
        )}

        {!isEditMode && (
          <>
            <Field
              id="scheduleId-modal"
              label={copy.name}
              required
              helper={copy.nameHelper}
              error={nameError ? <span role="alert">{nameError}</span> : undefined}
            >
              <Input
                type="text"
                value={scheduleId}
                onChange={(e) => {
                  setScheduleId(e.target.value);
                  setNameError(null);
                }}
                placeholder={copy.namePlaceholder}
                required
              />
            </Field>

            {/* One `Input` with a trailing ghost Browse button. The row is the
                Field's child, so the label is pointed at the input by id. */}
            <Field
              id="workflowSource-modal"
              label={copy.workflowFile}
              error={workflowError ? <span role="alert">{workflowError}</span> : undefined}
            >
              <div id="workflowSource-modal-row" className="flex items-center gap-2">
                <Input
                  type="text"
                  id="workflowSource-modal"
                  aria-describedby={workflowError ? fieldHelpId('workflowSource-modal') : undefined}
                  aria-invalid={workflowError ? true : undefined}
                  value={workflowSourcePath}
                  onChange={(e) => {
                    setWorkflowSourcePath(e.target.value);
                    setWorkflowError(null);
                  }}
                  placeholder={copy.workflowPlaceholder}
                />
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      type="button"
                      variant="ghost"
                      shape="round"
                      onClick={handleBrowseFile}
                      aria-label={copy.browse}
                    >
                      <Folder />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>{copy.browse}</TooltipContent>
                </Tooltip>
              </div>
            </Field>
          </>
        )}

        <div className="flex min-w-0 flex-col gap-1.5">
          <span className="text-label text-text-default">{copy.when}</span>
          <CronPicker schedule={schedule} onChange={setCronExpression} isValid={setIsValid} />
        </div>
      </form>
    </ModalShell>
  );
};
