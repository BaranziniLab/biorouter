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
  runScheduleNow,
  ScheduledJob,
} from '../../schedule';
import { ScrollArea } from '../ui/scroll-area';
import { Button } from '../ui/button';
import { Plus, RefreshCw, Pause, Play, CircleDotDashed, StopSquare } from '../icons/app-icons';
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
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { IconAction, RowActions, RowContextMenu, type RowActionItem } from '../ui/row-actions';
import { ScheduleStatusDot, formatRunTime, readableCronOf, scheduleState } from './scheduleStatus';
import { DELETE_SCHEDULE_MESSAGE, scheduleCopy as copy } from './copy';

// Kept as an export of this module too: older imports read it from here.
export { DELETE_SCHEDULE_MESSAGE };

interface SchedulesViewProps {
  onClose?: () => void;
}

const errorText = (error: unknown, fallback: string) =>
  error instanceof Error && error.message ? error.message : fallback;

/**
 * The cron as words, with the expression itself one hover away (spec 3.10: "the full cron goes
 * in the tooltip of the cron phrase").
 */
function CronPhrase({ cron }: { cron: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span>{readableCronOf(cron)}</span>
      </TooltipTrigger>
      <TooltipContent>
        <span className="font-mono">{cron}</span>
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * One schedule, as a two-line hairline row (spec 3.10, Crew's content row).
 *
 * - A leading 8px status dot (Codex's per-thread circle) in place of the clock glyph.
 * - Line 1: the name (and Built-in). Line 2: ONE meta line, "{status} · {cron} · Last run
 *   {when}", in the sans face with tabular figures. A failed schedule's last error replaces
 *   that line in danger ink: it is the only record of a failure (issue #56), so it stays
 *   visible.
 * - Actions appear when wanted: Pause or Resume (Stop while running) and `⋯` with Edit, Run
 *   now, Inspect run, then Delete. Right-click and Shift+F10 open the same menu.
 * - The row's body is the button that opens the detail; the actions are their own controls,
 *   never nested inside it.
 */
const ScheduleRow: React.FC<{
  job: ScheduledJob;
  onNavigateToDetail: (id: string) => void;
  onEdit: (job: ScheduledJob) => void;
  onPause: (id: string) => void;
  onUnpause: (id: string) => void;
  onKill: (id: string) => void;
  onRunNow: (id: string) => void;
  onDelete: (id: string) => void;
  actionInProgress: boolean;
}> = ({
  job,
  onNavigateToDetail,
  onEdit,
  onPause,
  onUnpause,
  onKill,
  onRunNow,
  onDelete,
  actionInProgress,
}) => {
  const name = scheduleDisplayName(job.id);
  const { words, failed } = scheduleState(job);
  const running = Boolean(job.currently_running);
  const lastRun = formatRunTime(job.last_run);

  const menu: RowActionItem[] = [
    ...(running
      ? [{ label: copy.inspectRun, onSelect: () => onNavigateToDetail(job.id) } as RowActionItem]
      : []),
    {
      label: copy.edit,
      onSelect: () => onEdit(job),
      disabled: running || actionInProgress,
    },
    {
      label: copy.runNow,
      onSelect: () => onRunNow(job.id),
      disabled: running || actionInProgress,
    },
    { kind: 'separator' },
    {
      label: copy.delete,
      onSelect: () => onDelete(job.id),
      destructive: true,
      disabled: actionInProgress,
    },
  ];

  const primary = running ? (
    <IconAction
      icon={StopSquare}
      label={copy.stopNamed(name)}
      onSelect={() => onKill(job.id)}
      disabled={actionInProgress}
    />
  ) : job.paused ? (
    <IconAction
      icon={Play}
      label={copy.resumeNamed(name)}
      onSelect={() => onUnpause(job.id)}
      disabled={actionInProgress}
    />
  ) : (
    <IconAction
      icon={Pause}
      label={copy.pauseNamed(name)}
      onSelect={() => onPause(job.id)}
      disabled={actionInProgress}
    />
  );

  return (
    <RowContextMenu items={menu}>
      <div className="biorouter-list-row flex items-center gap-3" data-testid="schedule-row">
        <ScheduleStatusDot job={job} className="shrink-0" />
        <button
          type="button"
          className="min-w-0 flex-1 cursor-pointer rounded-inner text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
          onClick={() => onNavigateToDetail(job.id)}
          aria-label={copy.view(name)}
        >
          <span className="flex min-w-0 items-center gap-2">
            <h3 className="min-w-0 truncate text-label">{name}</h3>
            {isBuiltinSchedule(job.id) && <BuiltInBadge title={BUILTIN_RECREATED_TITLE} />}
          </span>
          {failed ? (
            <p className="truncate text-supporting text-text-danger">
              {words.join(' · ')} · {job.last_error}
            </p>
          ) : (
            <p className="truncate text-supporting text-text-muted tabular-nums">
              {words.join(' · ')} · <CronPhrase cron={job.cron} /> ·{' '}
              {lastRun ? (
                <span title={formatToLocalDateWithTimezone(job.last_run)}>
                  {copy.lastRun(lastRun)}
                </span>
              ) : (
                copy.notRunYet
              )}
            </p>
          )}
        </button>
        <RowActions primary={primary} menu={menu} menuLabel={copy.moreActionsNamed(name)} />
      </div>
    </RowContextMenu>
  );
};

/** Loading is rows that are the shape of rows, not a spinner in dead space. */
const ScheduleRowSkeleton: React.FC = () => (
  <div className="biorouter-list-row flex items-center gap-3">
    <Skeleton className="h-2 w-2 rounded-full" />
    <div className="min-w-0 flex-1">
      <Skeleton className="h-4 w-48" />
      <Skeleton className="mt-1 h-3 w-64" />
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
      setApiError(errorText(error, copy.loadFailed));
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
        toastSuccess({ title: copy.saved(scheduleDisplayName(editingSchedule.id)) });
      } else {
        const newPayload = payload as NewSchedulePayload;
        await createSchedule(newPayload);
      }
      await fetchSchedules();
      setIsModalOpen(false);
      setEditingSchedule(null);
    } catch (error) {
      console.error('Failed to save schedule:', error);
      setSubmitApiError(errorText(error, 'Unknown error saving schedule.'));
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteSchedule = async (id: string) => {
    if (!beginAction(id)) return;
    if (viewingScheduleId === id) setViewingScheduleId(null);

    try {
      await deleteSchedule(id);
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to delete schedule "${id}":`, error);
      toastError({
        title: copy.couldNotDelete(scheduleDisplayName(id)),
        msg: errorText(error, `Unknown error deleting "${id}".`),
      });
    } finally {
      finishAction(id);
      setScheduleToDeleteId(null);
    }
  };

  // Pause and Resume raise no success toast (spec 6.9): the row's status word is the
  // confirmation. A failure keeps its reason.
  const handlePauseSchedule = async (id: string) => {
    if (!beginAction(id)) return;
    try {
      await pauseSchedule(id);
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to pause schedule "${id}":`, error);
      toastError({
        title: copy.couldNotPause(scheduleDisplayName(id)),
        msg: errorText(error, `Unknown error pausing "${id}".`),
      });
    } finally {
      finishAction(id);
    }
  };

  const handleUnpauseSchedule = async (id: string) => {
    if (!beginAction(id)) return;
    try {
      await unpauseSchedule(id);
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to resume schedule "${id}":`, error);
      toastError({
        title: copy.couldNotResume(scheduleDisplayName(id)),
        msg: errorText(error, `Unknown error resuming "${id}".`),
      });
    } finally {
      finishAction(id);
    }
  };

  const handleKillRunningJob = async (id: string) => {
    if (!beginAction(id)) return;
    try {
      await killRunningJob(id);
      toastSuccess({ title: copy.runStopped });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to kill running job "${id}":`, error);
      toastError({
        title: copy.couldNotStop,
        msg: errorText(error, `Unknown error stopping "${id}".`),
      });
    } finally {
      finishAction(id);
    }
  };

  const handleRunNow = async (id: string) => {
    if (!beginAction(id)) return;
    try {
      const result = await runScheduleNow(id);
      toastSuccess({
        title: result === 'CANCELLED' ? copy.runStoppedWhileStarting : copy.runStarted,
      });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to run schedule "${id}":`, error);
      toastError({
        title: copy.couldNotRun(scheduleDisplayName(id)),
        msg: errorText(error, `Unknown error running "${id}".`),
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
      <MainPanelLayout removeTopPadding>
        <div className="flex min-h-0 flex-1 flex-col">
          {/* The band (spec 3.10): the title with its help in an InfoTip, Refresh
              as a ghost round icon, and "New schedule" as the view's one accent.
              The empty state below carries no second button. The list also polls
              every 15s, so Refresh is a convenience, not the only way to see a
              change. */}
          <PageHeader
            title={copy.title}
            info={copy.info}
            actions={
              <>
                <PageHeaderAction
                  icon={RefreshCw}
                  label={copy.refresh}
                  tooltip={copy.refreshTooltip}
                  onClick={handleRefresh}
                  disabled={isRefreshing || isLoading}
                />
                <Button onClick={openCreateModal}>
                  <Plus />
                  {copy.newSchedule}
                </Button>
              </>
            }
          />

          <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6 pt-2">
            <ScrollArea className="h-full">
              <div className="relative h-full pb-6">
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
                    title={copy.emptyTitle}
                    description={copy.emptyDescription}
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
                        onRunNow={handleRunNow}
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
        title={copy.deleteTitle(scheduleDisplayName(scheduleToDeleteId ?? ''))}
        message={DELETE_SCHEDULE_MESSAGE}
        confirmLabel={copy.deleteConfirm}
        cancelLabel={copy.cancel}
        confirmVariant="destructive"
        isSubmitting={scheduleToDeleteId !== null && actionsInProgress.has(scheduleToDeleteId)}
        onConfirm={() => scheduleToDeleteId && void handleDeleteSchedule(scheduleToDeleteId)}
        onCancel={() => setScheduleToDeleteId(null)}
      />
    </>
  );
};

export default SchedulesView;
