import React, { useState, useEffect, useRef } from 'react';
import { useLocation } from 'react-router-dom';
import { useSameRouteReset } from '../../hooks/useSameRouteReset';
import {
  listSchedules,
  createSchedule,
  deleteSchedule,
  pauseSchedule,
  unpauseSchedule,
  updateSchedule,
  killRunningJob,
  inspectRunningJob,
  ScheduledJob,
} from '../../schedule';
import { ScrollArea } from '../ui/scroll-area';
import { Button } from '../ui/button';
import {
  Plus,
  RefreshCw,
  Pause,
  Play,
  Edit,
  Square,
  Eye,
  CircleDotDashed,
  Trash2,
  Clock,
} from '../icons/app-icons';
import { NewSchedulePayload, ScheduleModal } from './ScheduleModal';
import ScheduleDetailView from './ScheduleDetailView';
import { toastError, toastSuccess } from '../../toasts';
import { formatToLocalDateWithTimezone } from '../../utils/date';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ViewOptions } from '../../utils/navigationUtils';
import BuiltInBadge from '../ui/BuiltInBadge';
import {
  BUILTIN_RECREATED_TITLE,
  isBuiltinSchedule,
  scheduleDisplayName,
} from '../../utils/builtins';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader } from '../Layout/PageHeader';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { ScheduleStatus, readableCronOf } from './scheduleStatus';

/**
 * What deleting a schedule actually does, said in the confirmation.
 *
 * ⚠ It no longer removes the workflow, so it must no longer imply that it
 * might. The previous sentence — "This permanently removes the schedule and its
 * run configuration" — was written when a delete unlinked `job.source`
 * unconditionally, and for a schedule added from a workflow row that source was
 * the user's own workflow file. `scheduler::scheduler_owns_source` now confines
 * the unlink to the private copy the scheduler made for itself, so the
 * confirmation names the file that survives instead of leaving the reader to
 * work out what "run configuration" covered.
 *
 * Exported so `SchedulesView.test.tsx` asserts the promise the dialog makes
 * against the behaviour the scheduler tests pin.
 */
export const DELETE_SCHEDULE_MESSAGE =
  'This removes the schedule and stops its future runs. The workflow it runs is left in place. This action cannot be undone.';

interface SchedulesViewProps {
  onClose?: () => void;
}

/**
 * One schedule, as a hairline row on the canvas.
 *
 * There is no card here and no card around the list: §3.10's "a list gets no
 * container" and design.md's P2 (rows, not cards). Status is TEXT beside a
 * status dot rather than a filled pill — the alpha-mixed 15% background pills
 * this replaced are exactly the hand-mixed fills the settings vocabulary bans, and
 * §2.5 reserves translucent status fills for a `Note`, not for a word in a row.
 */
const ScheduleRow: React.FC<{
  job: ScheduledJob;
  onNavigateToDetail: (id: string) => void;
  onEdit: (job: ScheduledJob) => void;
  onPause: (id: string) => void;
  onUnpause: (id: string) => void;
  onKill: (id: string) => void;
  onInspect: (id: string) => void;
  onDelete: (id: string) => void;
  actionInProgress: boolean;
}> = ({
  job,
  onNavigateToDetail,
  onEdit,
  onPause,
  onUnpause,
  onKill,
  onInspect,
  onDelete,
  actionInProgress,
}) => {
  const readableCron = readableCronOf(job.cron);
  const formattedLastRun = formatToLocalDateWithTimezone(job.last_run);

  return (
    <div className="biorouter-list-row group flex items-start justify-between gap-3 px-3 py-3">
      <button
        type="button"
        className="min-w-0 flex-1 cursor-pointer rounded-inner text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
        onClick={() => onNavigateToDetail(job.id)}
        aria-label={`View schedule ${scheduleDisplayName(job.id)}`}
      >
        <div className="flex min-w-0 items-center gap-1.5">
          <Clock className="h-4 w-4 shrink-0 text-text-muted" aria-hidden />
          <h3 className="min-w-0 truncate text-label" title={job.id}>
            {scheduleDisplayName(job.id)}
          </h3>
          {isBuiltinSchedule(job.id) && <BuiltInBadge title={BUILTIN_RECREATED_TITLE} />}
          <ScheduleStatus job={job} />
        </div>
        <p className="mt-0.5 line-clamp-1 text-supporting text-text-muted" title={readableCron}>
          {readableCron}
        </p>
        <p className="mt-1 text-supporting text-text-muted">
          Last run <span className="font-mono tabular-nums">{formattedLastRun}</span>
        </p>
        {/*
          Issue #56. A schedule whose last tick failed looks identical to a
          healthy one on this list — a fresh session is minted per run, so
          there is nothing else here to notice. Cleared by the next success.
        */}
        {job.last_error && (
          <p
            className="mt-1 line-clamp-2 text-supporting text-text-danger [overflow-wrap:anywhere]"
            title={job.last_error}
          >
            {job.last_error}
          </p>
        )}
      </button>

      <div className="flex shrink-0 items-center gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100">
        {!job.currently_running && (
          <>
            <Button
              onClick={(e) => {
                e.stopPropagation();
                onEdit(job);
              }}
              disabled={actionInProgress}
              variant="ghost"
              shape="round"
              title="Edit schedule"
              aria-label={`Edit ${scheduleDisplayName(job.id)}`}
            >
              <Edit />
            </Button>
            <Button
              onClick={(e) => {
                e.stopPropagation();
                if (job.paused) {
                  onUnpause(job.id);
                } else {
                  onPause(job.id);
                }
              }}
              disabled={actionInProgress}
              variant="ghost"
              shape="round"
              title={job.paused ? 'Resume this schedule' : 'Pause this schedule'}
              aria-label={`${job.paused ? 'Resume' : 'Pause'} ${scheduleDisplayName(job.id)}`}
            >
              {job.paused ? <Play /> : <Pause />}
            </Button>
          </>
        )}
        {job.currently_running && (
          <>
            <Button
              onClick={(e) => {
                e.stopPropagation();
                onInspect(job.id);
              }}
              disabled={actionInProgress}
              variant="ghost"
              shape="round"
              title="Show the current run"
              aria-label={`Inspect ${scheduleDisplayName(job.id)}`}
            >
              <Eye />
            </Button>
            <Button
              onClick={(e) => {
                e.stopPropagation();
                onKill(job.id);
              }}
              disabled={actionInProgress}
              variant="ghost"
              shape="round"
              title="Stop the running job"
              aria-label={`Stop ${scheduleDisplayName(job.id)}`}
            >
              <Square />
            </Button>
          </>
        )}
        <Button
          onClick={(e) => {
            e.stopPropagation();
            onDelete(job.id);
          }}
          disabled={actionInProgress}
          variant="ghost"
          shape="round"
          className="text-text-danger"
          title="Delete schedule"
          aria-label={`Delete ${scheduleDisplayName(job.id)}`}
        >
          <Trash2 />
        </Button>
      </div>
    </div>
  );
};

/** Loading is rows that are the shape of rows, not a spinner in dead space. */
const ScheduleRowSkeleton: React.FC = () => (
  <div className="biorouter-list-row flex items-start justify-between gap-3 px-3 py-3">
    <div className="min-w-0 flex-1">
      <Skeleton className="h-4 w-48" />
      <Skeleton className="mt-2 h-3 w-32" />
      <Skeleton className="mt-2 h-3 w-56" />
    </div>
  </div>
);

const SchedulesView: React.FC<SchedulesViewProps> = ({ onClose: _onClose }) => {
  const location = useLocation();
  const [schedules, setSchedules] = useState<ScheduledJob[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [apiError, setApiError] = useState<string | null>(null);
  const [submitApiError, setSubmitApiError] = useState<string | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [editingSchedule, setEditingSchedule] = useState<ScheduledJob | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [actionsInProgress, setActionsInProgress] = useState<Set<string>>(new Set());
  const actionsInProgressRef = useRef<Set<string>>(new Set());
  const [viewingScheduleId, setViewingScheduleId] = useState<string | null>(null);
  const [scheduleToDeleteId, setScheduleToDeleteId] = useState<string | null>(null);

  // Defect 3.3. `viewingScheduleId` (and the session history nested inside
  // `ScheduleDetailView`) is local state, not a URL — so re-selecting Scheduler
  // in the rail reconciled this component unchanged and left the user parked in
  // a run detail. Dropping the id here unmounts the detail view, which takes
  // its own `selectedSession` with it.
  useSameRouteReset('/schedules', () => {
    setViewingScheduleId(null);
    setScheduleToDeleteId(null);
  });

  const beginAction = (id: string) => {
    if (actionsInProgressRef.current.has(id)) return false;
    actionsInProgressRef.current.add(id);
    setActionsInProgress(new Set(actionsInProgressRef.current));
    return true;
  };

  const finishAction = (id: string) => {
    actionsInProgressRef.current.delete(id);
    setActionsInProgress(new Set(actionsInProgressRef.current));
  };

  const fetchSchedules = async () => {
    setIsLoading(true);
    setApiError(null);
    try {
      const fetchedSchedules = await listSchedules();
      setSchedules(fetchedSchedules);
    } catch (error) {
      console.error('Failed to fetch schedules:', error);
      setApiError(
        error instanceof Error
          ? error.message
          : 'An unknown error occurred while fetching schedules.'
      );
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    if (viewingScheduleId === null) {
      fetchSchedules();

      const locationState = location.state as ViewOptions | null;
      if (locationState?.pendingScheduleDeepLink) {
        setIsModalOpen(true);
        window.history.replaceState({}, document.title);
      }
    }
  }, [viewingScheduleId, location.state]);

  useEffect(() => {
    if (viewingScheduleId !== null || actionsInProgress.size > 0) return;

    const intervalId = setInterval(() => {
      if (viewingScheduleId === null && !isRefreshing && !isLoading && !isSubmitting) {
        fetchSchedules();
      }
    }, 15000);

    return () => clearInterval(intervalId);
  }, [viewingScheduleId, isRefreshing, isLoading, isSubmitting, actionsInProgress.size]);

  const handleRefresh = async () => {
    setIsRefreshing(true);
    try {
      await fetchSchedules();
    } finally {
      setIsRefreshing(false);
    }
  };

  const handleModalSubmit = async (payload: NewSchedulePayload | string) => {
    setIsSubmitting(true);
    setSubmitApiError(null);
    try {
      if (editingSchedule) {
        await updateSchedule(editingSchedule.id, payload as string);
        toastSuccess({
          title: 'Schedule updated',
          msg: `Updated schedule "${editingSchedule.id}"`,
        });
      } else {
        const newPayload = payload as NewSchedulePayload;
        await createSchedule(newPayload);
      }
      await fetchSchedules();
      setIsModalOpen(false);
      setEditingSchedule(null);
    } catch (error) {
      console.error('Failed to save schedule:', error);
      const errorMsg = error instanceof Error ? error.message : 'Unknown error saving schedule.';
      setSubmitApiError(errorMsg);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteSchedule = async (id: string) => {
    if (!beginAction(id)) return;
    if (viewingScheduleId === id) setViewingScheduleId(null);
    setApiError(null);

    try {
      await deleteSchedule(id);
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to delete schedule "${id}":`, error);
      const errorMsg = error instanceof Error ? error.message : `Unknown error deleting "${id}".`;
      setApiError(errorMsg);
    } finally {
      finishAction(id);
      setScheduleToDeleteId(null);
    }
  };

  const handlePauseSchedule = async (id: string) => {
    if (!beginAction(id)) return;
    setApiError(null);

    try {
      await pauseSchedule(id);
      toastSuccess({
        title: 'Schedule paused',
        msg: `Paused schedule "${id}"`,
      });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to pause schedule "${id}":`, error);
      const errorMsg = error instanceof Error ? error.message : `Unknown error pausing "${id}".`;
      setApiError(errorMsg);
      toastError({
        title: 'Pause schedule error',
        msg: errorMsg,
      });
    } finally {
      finishAction(id);
    }
  };

  const handleUnpauseSchedule = async (id: string) => {
    if (!beginAction(id)) return;
    setApiError(null);

    try {
      await unpauseSchedule(id);
      toastSuccess({
        title: 'Schedule unpaused',
        msg: `Resumed schedule "${id}"`,
      });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to unpause schedule "${id}":`, error);
      const errorMsg = error instanceof Error ? error.message : `Unknown error unpausing "${id}".`;
      setApiError(errorMsg);
      toastError({
        title: 'Unpause schedule error',
        msg: errorMsg,
      });
    } finally {
      finishAction(id);
    }
  };

  const handleKillRunningJob = async (id: string) => {
    if (!beginAction(id)) return;
    setApiError(null);

    try {
      const result = await killRunningJob(id);
      toastSuccess({
        title: 'Job stopped',
        msg: result.message,
      });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to kill running job "${id}":`, error);
      const errorMsg =
        error instanceof Error ? error.message : `Unknown error killing job "${id}".`;
      setApiError(errorMsg);
      toastError({
        title: 'Could not stop the job',
        msg: errorMsg,
      });
    } finally {
      finishAction(id);
    }
  };

  const handleInspectRunningJob = async (id: string) => {
    if (!beginAction(id)) return;
    setApiError(null);

    try {
      const result = await inspectRunningJob(id);
      if (result.sessionId) {
        const duration = result.runningDurationSeconds
          ? `${Math.floor(result.runningDurationSeconds / 60)}m ${result.runningDurationSeconds % 60}s`
          : 'Unknown';
        toastSuccess({
          title: 'Job inspection',
          msg: `Session ID: ${result.sessionId}\nRunning for: ${duration}`,
        });
      } else {
        toastSuccess({
          title: 'Job inspection',
          msg: 'No detailed information available for this job',
        });
      }
    } catch (error) {
      console.error(`Failed to inspect running job "${id}":`, error);
      const errorMsg =
        error instanceof Error ? error.message : `Unknown error inspecting job "${id}".`;
      setApiError(errorMsg);
      toastError({
        title: 'Inspect job error',
        msg: errorMsg,
      });
    } finally {
      finishAction(id);
    }
  };

  const handleNavigateToDetail = (id: string) => {
    setViewingScheduleId(id);
  };

  const openCreateModal = () => {
    setSubmitApiError(null);
    setIsModalOpen(true);
  };

  if (viewingScheduleId) {
    return (
      <ScheduleDetailView
        scheduleId={viewingScheduleId}
        onNavigateBack={() => setViewingScheduleId(null)}
      />
    );
  }

  return (
    <>
      <MainPanelLayout>
        <div className="flex-1 flex flex-col min-h-0">
          {/* ⚠ The two actions sit on their OWN LINE under the description, not
              on the title row — the operator's decision, 2026-09-07, naming
              Workflows / Extensions / Skills / Built apps as the shape the rest
              of the app should match. This view shipped the §4.2 original
              (right-aligned on the title row) and §4.2 is amended rather than
              quietly contradicted; `PageHeader` owns the placement now, so the
              Scheduler cannot drift from its siblings again.

              `New schedule` comes FIRST because the strip reads left to right
              and the primary leads it, the same order Workflows uses (Create,
              then Import). The reading column is still the chat measure, for
              the reason it always was: this is a column of rows, not a
              document, so width past the measure buys margin. */}
          <PageHeader
            title="Scheduler"
            description="Run a saved workflow automatically, at the time you choose."
            actions={
              <>
                <Button onClick={openCreateModal}>
                  <Plus />
                  New schedule
                </Button>
                <Button
                  onClick={handleRefresh}
                  disabled={isRefreshing || isLoading}
                  variant="ghost"
                  shape="round"
                  title="Refresh"
                  aria-label="Refresh schedules"
                >
                  <RefreshCw className={isRefreshing ? 'animate-spin' : undefined} />
                </Button>
              </>
            }
          />

          <ReadableContent size="chat" className="flex-1 min-h-0 relative px-6 pt-6">
            <ScrollArea className="h-full">
              <div className="h-full relative pb-8">
                {apiError && (
                  <Note tone="danger" role="alert" className="mb-4">
                    {apiError}
                  </Note>
                )}

                {isLoading && schedules.length === 0 && (
                  <div className="biorouter-list-shell" aria-hidden>
                    <ScheduleRowSkeleton />
                    <ScheduleRowSkeleton />
                    <ScheduleRowSkeleton />
                  </div>
                )}

                {!isLoading && !apiError && schedules.length === 0 && (
                  <EmptyState
                    icon={CircleDotDashed}
                    title="No schedules yet"
                    description="Create a schedule to run a saved workflow automatically at the time you choose."
                    actions={
                      <Button onClick={openCreateModal}>
                        <Plus />
                        New schedule
                      </Button>
                    }
                  />
                )}

                {/* ⚠ NOT `!isLoading && schedules.length > 0`, which is what this
                    was. `fetchSchedules` sets `isLoading` on the 15-second poll
                    as well as on first load, and with rows already on screen
                    none of these three branches then matched — so the list
                    UNMOUNTED for the length of every poll's request and came
                    back. The three stay mutually exclusive without it: the
                    skeletons require an empty list, and the empty state
                    requires the load to have finished. */}
                {schedules.length > 0 && (
                  <div className="biorouter-list-shell">
                    {schedules.map((job) => (
                      <ScheduleRow
                        key={job.id}
                        job={job}
                        onNavigateToDetail={handleNavigateToDetail}
                        onEdit={(schedule) => {
                          setEditingSchedule(schedule);
                          setSubmitApiError(null);
                          setIsModalOpen(true);
                        }}
                        onPause={handlePauseSchedule}
                        onUnpause={handleUnpauseSchedule}
                        onKill={handleKillRunningJob}
                        onInspect={handleInspectRunningJob}
                        onDelete={setScheduleToDeleteId}
                        actionInProgress={actionsInProgress.has(job.id) || isSubmitting}
                      />
                    ))}
                  </div>
                )}
              </div>
            </ScrollArea>
          </ReadableContent>
        </div>
      </MainPanelLayout>

      <ScheduleModal
        isOpen={isModalOpen}
        onClose={() => {
          setIsModalOpen(false);
          setEditingSchedule(null);
          setSubmitApiError(null);
        }}
        onSubmit={handleModalSubmit}
        schedule={editingSchedule}
        isLoadingExternally={isSubmitting}
        apiErrorExternally={submitApiError}
      />
      <ConfirmationModal
        isOpen={scheduleToDeleteId !== null}
        title={`Delete "${scheduleToDeleteId ?? ''}"?`}
        message={DELETE_SCHEDULE_MESSAGE}
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        isSubmitting={scheduleToDeleteId !== null && actionsInProgress.has(scheduleToDeleteId)}
        onConfirm={() => scheduleToDeleteId && void handleDeleteSchedule(scheduleToDeleteId)}
        onCancel={() => setScheduleToDeleteId(null)}
      />
    </>
  );
};

export default SchedulesView;
