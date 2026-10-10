import React, { useState, useEffect } from 'react';
import { Button } from '../ui/button';
import { ScrollArea } from '../ui/scroll-area';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { EmptyState } from '../ui/empty-state';
import { Spinner } from '../ui/spinner';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import type { RowActionItem } from '../ui/row-actions';
import {
  getScheduleSessions,
  runScheduleNow,
  pauseSchedule,
  unpauseSchedule,
  updateSchedule,
  listSchedules,
  killRunningJob,
  inspectRunningJob,
  deleteSchedule,
  ScheduledJob,
} from '../../schedule';
import SessionHistoryView from '../sessions/SessionHistoryView';
import { ScheduleModal, NewSchedulePayload } from './ScheduleModal';
import { ScheduleStatus, formatRunTime, readableCronOf } from './scheduleStatus';
import { toastError, toastSuccess } from '../../toasts';
import { Pause, Play, MessageSquareText, StopSquare } from '../icons/app-icons';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader, PageHeaderAction, PageHeaderMenu } from '../Layout/PageHeader';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { formatToLocalDateWithTimezone } from '../../utils/date';
import { billedSessionTokenEstimate, formatBilledTokenEstimate } from '../../utils/billedTokens';
import { scheduleDisplayName } from '../../utils/builtins';
import { getSession, Session, type SessionClassification } from '../../api';
import { useSessionListTiers } from '../privacy/useSessionListTiers';
import { userActionHeaders } from '../../utils/userAction';
import { DELETE_SCHEDULE_MESSAGE, scheduleCopy } from './copy';

const copy = scheduleCopy.detail;

const errorText = (error: unknown, fallback: string) =>
  error instanceof Error && error.message ? error.message : fallback;

/** The file a path names, for a row that shows names rather than paths. */
const fileNameOf = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

interface ScheduleSessionMeta {
  id: string;
  name: string;
  createdAt: string;
  workingDir?: string;
  scheduleId?: string | null;
  messageCount?: number;
  totalTokens?: number | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  accumulatedTotalTokens?: number | null;
  accumulatedInputTokens?: number | null;
  accumulatedOutputTokens?: number | null;
}

interface ScheduleDetailViewProps {
  scheduleId: string | null;
  onNavigateBack: () => void;
}

/**
 * A fact row: the label on the left, the fact on its trailing edge (astryx §4.5's definition
 * row, on the shared 40px `.biorouter-settings-row`). Names, never ids (principle 10): a path or
 * a raw cron is one hover away in `tooltip`, never on the row.
 *
 * ⚠ The rows must be DIRECT children of `.biorouter-settings-list`, so this component returns
 * the row itself and never a wrapper: `.biorouter-settings-row:last-child` is relative to a
 * row's own parent, and a per-row wrapper makes every row a `:last-child`.
 */
function DefinitionRow({
  label,
  children,
  tone,
  tooltip,
}: {
  label: string;
  children: React.ReactNode;
  tone?: 'danger';
  tooltip?: React.ReactNode;
}) {
  const value = (
    <span
      className={[
        'min-w-0 text-body tabular-nums [overflow-wrap:anywhere]',
        tone === 'danger' ? 'text-text-danger' : 'text-text-default',
      ].join(' ')}
    >
      {children}
    </span>
  );
  return (
    <div className="biorouter-settings-row flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1 px-3 py-2.5">
      <span className="text-body text-text-muted">{label}</span>
      {tooltip ? (
        <Tooltip>
          <TooltipTrigger asChild>{value}</TooltipTrigger>
          <TooltipContent>{tooltip}</TooltipContent>
        </Tooltip>
      ) : (
        value
      )}
    </div>
  );
}

/**
 * One past run, as a two-line hairline row: the run's name, then "{when} · {n} messages ·
 * {tokens} tokens" in the sans face with tabular figures (spec 3.10: run rows are sans
 * tabular). The working directory is a path, so it is not a row line (principle 10); the
 * opened transcript shows it.
 *
 * ⚠ **`tier` comes from the session-list cache, because this row's own endpoint carries none.**
 * `GET /schedule/{id}/sessions` returns `SessionDisplayInfo`, which has no `privacy_tier`, and
 * until 2026-09-14 this row passed the glyph no tier at all, which the glyph then drew as
 * `data-privacy="public"` on EVERY run, private ones included. A run the list does not carry
 * (one that has recorded no message yet) is drawn as not yet known, never as Public.
 */
function RunRow({
  session,
  tier,
  onOpen,
}: {
  session: ScheduleSessionMeta;
  tier: SessionClassification | undefined;
  onOpen: () => void;
}) {
  // `SessionDisplayInfo` is camelCase and `billedSessionTokenEstimate` reads the session row's
  // snake_case columns, so the mapping happens here rather than the figure being re-derived:
  // ONE helper decides billed-vs-last-turn (issue #1).
  const billed = billedSessionTokenEstimate({
    accumulated_total_tokens: session.accumulatedTotalTokens,
    accumulated_input_tokens: session.accumulatedInputTokens,
    accumulated_output_tokens: session.accumulatedOutputTokens,
    total_tokens: session.totalTokens,
  });
  const when = formatRunTime(session.createdAt);
  const meta: React.ReactNode[] = [];
  if (when) meta.push(<span key="when">{when}</span>);
  if (session.messageCount !== undefined) {
    meta.push(<span key="messages">{copy.messages(session.messageCount)}</span>);
  }
  if (billed) {
    meta.push(
      <span key="tokens" title={billed.lowerBound ? copy.tokensLowerBound : copy.tokensBilled}>
        {copy.tokens(formatBilledTokenEstimate(billed))}
      </span>
    );
  }

  return (
    <div className="biorouter-list-row flex items-center gap-3">
      <button
        type="button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 rounded-inner text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
        aria-label={copy.openRun(session.name || session.id)}
      >
        <ChatKindIcon
          session={{ name: session.name, session_type: 'scheduled' }}
          tier={tier}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1">
          <span className="block min-w-0 truncate text-label">
            {session.name || <span className="font-mono text-code">{session.id}</span>}
          </span>
          {meta.length > 0 && (
            <span className="block truncate text-supporting text-text-muted tabular-nums">
              {meta.flatMap((item, index) => (index === 0 ? [item] : [' · ', item]))}
            </span>
          )}
        </span>
      </button>
    </div>
  );
}

const RunRowSkeleton: React.FC = () => (
  <div className="biorouter-list-row flex items-center gap-3">
    <Skeleton className="h-4 w-4" />
    <div className="min-w-0 flex-1">
      <Skeleton className="h-4 w-56" />
      <Skeleton className="mt-1 h-3 w-40" />
    </div>
  </div>
);

/**
 * A disabled control still says why (spec 3.10: "the disabled controls explain themselves in
 * tooltips"). A disabled button takes no pointer events, so the reason hangs on a focusable
 * wrapper; the reason is also its accessible description, so it never depends on a hover.
 */
function DisabledReason({
  reason,
  children,
}: {
  reason: string | null;
  children: React.ReactElement;
}) {
  const id = React.useId();
  if (!reason) return children;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span tabIndex={0} aria-describedby={id} className="inline-flex rounded-element">
          {children}
          <span id={id} className="sr-only">
            {reason}
          </span>
        </span>
      </TooltipTrigger>
      <TooltipContent side="bottom">{reason}</TooltipContent>
    </Tooltip>
  );
}

/**
 * One schedule, on the band (spec 3.10).
 *
 * The band holds Back, the schedule's name and its status, with Run now as the view's one
 * accent, Pause or Resume (Stop while a run is in flight) and `⋯`. Below it: one hairline list
 * of facts, by name rather than id, and the recent runs. The "Actions" section and the three
 * notes that explained the state are gone: the status says Running or Paused, and a control
 * that cannot act says why in its tooltip.
 *
 * What it stopped being earlier still holds: no `h-screen` shell (the anti-pattern
 * `MainPanelLayout`'s own comment warns breaks embedded panes), no `<Card>` of `**Label:**
 * value` sentences, no grid of run cards.
 */
const ScheduleDetailView: React.FC<ScheduleDetailViewProps> = ({ scheduleId, onNavigateBack }) => {
  const [sessions, setSessions] = useState<ScheduleSessionMeta[]>([]);
  // Each run's tier, from the session list — see `RunRow`.
  const runTiers = useSessionListTiers();
  const [isLoadingSessions, setIsLoadingSessions] = useState(false);
  const [sessionsError, setSessionsError] = useState<string | null>(null);

  const [scheduleDetails, setScheduleDetails] = useState<ScheduledJob | null>(null);
  const [isLoadingSchedule, setIsLoadingSchedule] = useState(false);
  const [scheduleError, setScheduleError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);

  const [isActionLoading, setIsActionLoading] = useState(false);
  const [isRunPending, setIsRunPending] = useState(false);

  const [selectedSession, setSelectedSession] = useState<Session | null>(null);
  const [isLoadingSession, setIsLoadingSession] = useState(false);
  const [sessionError, setSessionError] = useState<string | null>(null);

  const [isModalOpen, setIsModalOpen] = useState(false);
  const [isDeleteOpen, setIsDeleteOpen] = useState(false);

  const name = scheduleId ? scheduleDisplayName(scheduleId) : '';

  const fetchSessions = async (sId: string) => {
    setIsLoadingSessions(true);
    setSessionsError(null);
    try {
      const data = await getScheduleSessions(sId, 20);
      setSessions(data);
    } catch (err) {
      setSessionsError(errorText(err, copy.runsLoadFailed));
    } finally {
      setIsLoadingSessions(false);
    }
  };

  const fetchSchedule = async (sId: string) => {
    setIsLoadingSchedule(true);
    setScheduleError(null);
    try {
      const allSchedules = await listSchedules();
      const schedule = allSchedules.find((s) => s.id === sId);
      if (schedule) {
        setScheduleDetails(schedule);
        setNotFound(false);
      } else {
        setNotFound(true);
      }
    } catch (err) {
      setScheduleError(errorText(err, copy.loadFailed));
    } finally {
      setIsLoadingSchedule(false);
    }
  };

  useEffect(() => {
    if (scheduleId && !selectedSession) {
      fetchSessions(scheduleId);
      fetchSchedule(scheduleId);
    }
  }, [scheduleId, selectedSession]);

  const handleRunNow = async () => {
    if (!scheduleId) return;
    setIsRunPending(true);
    setIsActionLoading(true);
    try {
      const newSessionId = await runScheduleNow(scheduleId);
      toastSuccess({
        title:
          newSessionId === 'CANCELLED'
            ? scheduleCopy.runStoppedWhileStarting
            : scheduleCopy.runStarted,
      });
      await fetchSessions(scheduleId);
      await fetchSchedule(scheduleId);
    } catch (err) {
      toastError({
        title: scheduleCopy.couldNotRun(name),
        msg: errorText(err, 'Failed to trigger schedule'),
      });
    } finally {
      setIsRunPending(false);
      setIsActionLoading(false);
    }
  };

  // No success toast (spec 6.9): the status in the band is the confirmation.
  const handlePauseToggle = async () => {
    if (!scheduleId || !scheduleDetails) return;
    setIsActionLoading(true);
    const resuming = Boolean(scheduleDetails.paused);
    try {
      if (resuming) {
        await unpauseSchedule(scheduleId);
      } else {
        await pauseSchedule(scheduleId);
      }
      await fetchSchedule(scheduleId);
    } catch (err) {
      toastError({
        title: resuming ? scheduleCopy.couldNotResume(name) : scheduleCopy.couldNotPause(name),
        msg: errorText(err, 'Operation failed'),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleKill = async () => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      await killRunningJob(scheduleId);
      toastSuccess({ title: scheduleCopy.runStopped });
      await fetchSchedule(scheduleId);
    } catch (err) {
      toastError({ title: scheduleCopy.couldNotStop, msg: errorText(err, 'Failed to stop run') });
    } finally {
      setIsActionLoading(false);
    }
  };

  const loadSession = async (sessionId: string) => {
    setIsLoadingSession(true);
    setSessionError(null);
    try {
      const response = await getSession<true>({
        path: { session_id: sessionId },
        // Issue #56 Task 58: reading a private chat needs the proof-of-user.
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      setSelectedSession(response.data);
    } catch (err) {
      const msg = errorText(err, copy.chatLoadFailed);
      setSessionError(msg);
      toastError({ title: copy.chatLoadFailed, msg });
    } finally {
      setIsLoadingSession(false);
    }
  };

  // Inspect opens the running chat itself, rather than toasting its session id. The id comes
  // from the schedule when the list carried it, and from the daemon otherwise.
  const handleInspect = async () => {
    if (!scheduleId) return;
    const known = scheduleDetails?.current_session_id;
    if (known) {
      await loadSession(known);
      return;
    }
    setIsActionLoading(true);
    try {
      const result = await inspectRunningJob(scheduleId);
      if (result.sessionId) await loadSession(result.sessionId);
      else toastError({ title: scheduleCopy.couldNotInspect, msg: copy.noRunningChat });
    } catch (err) {
      toastError({ title: scheduleCopy.couldNotInspect, msg: errorText(err, copy.noRunningChat) });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleModalSubmit = async (payload: NewSchedulePayload | string) => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      await updateSchedule(scheduleId, payload as string);
      toastSuccess({ title: scheduleCopy.saved(name) });
      await fetchSchedule(scheduleId);
      setIsModalOpen(false);
    } catch (err) {
      toastError({
        title: scheduleCopy.couldNotSave(name),
        msg: errorText(err, 'Failed to update schedule'),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleDelete = async () => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      await deleteSchedule(scheduleId);
      setIsDeleteOpen(false);
      onNavigateBack();
    } catch (err) {
      toastError({
        title: scheduleCopy.couldNotDelete(name),
        msg: errorText(err, `Unknown error deleting "${scheduleId}".`),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  if (selectedSession) {
    return (
      <SessionHistoryView
        session={selectedSession}
        isLoading={isLoadingSession}
        error={sessionError}
        onBack={() => setSelectedSession(null)}
        onRetry={() => loadSession(selectedSession.id)}
        showActionButtons={true}
      />
    );
  }

  if (!scheduleId || notFound) {
    return (
      <MainPanelLayout removeTopPadding>
        <PageHeader title={copy.notFoundTitle} onBack={onNavigateBack} backLabel={copy.back} />
        <ReadableContent size="chat" className="px-6 pt-2">
          <EmptyState
            compact
            icon={MessageSquareText}
            title={copy.notFoundTitle}
            description={copy.notFoundDescription}
            actions={
              <Button variant="link" onClick={onNavigateBack}>
                {copy.back}
              </Button>
            }
          />
        </ReadableContent>
      </MainPanelLayout>
    );
  }

  const running = scheduleDetails?.currently_running ?? false;
  const paused = Boolean(scheduleDetails?.paused);
  const busyReason = running ? copy.availableAfterRun : null;

  const menu: RowActionItem[] = [
    ...(running ? [{ label: scheduleCopy.inspectRun, onSelect: () => void handleInspect() }] : []),
    {
      label: scheduleCopy.edit,
      onSelect: () => setIsModalOpen(true),
      disabled: running || isActionLoading || !scheduleDetails,
    },
    { kind: 'separator' },
    {
      label: scheduleCopy.delete,
      onSelect: () => setIsDeleteOpen(true),
      destructive: true,
      disabled: isActionLoading,
    },
  ];

  return (
    <>
      <MainPanelLayout removeTopPadding>
        <div className="flex min-h-0 flex-1 flex-col">
          <PageHeader
            title={name}
            onBack={onNavigateBack}
            backLabel={copy.back}
            adornment={scheduleDetails ? <ScheduleStatus job={scheduleDetails} /> : null}
            actions={
              <>
                {isRunPending && (
                  <span
                    role="status"
                    className="inline-flex items-center gap-1.5 text-supporting text-text-muted"
                  >
                    <Spinner size={14} />
                    {copy.running}
                  </span>
                )}
                {scheduleDetails &&
                  (running ? (
                    <PageHeaderAction
                      icon={StopSquare}
                      label={scheduleCopy.stopRun}
                      onClick={handleKill}
                      disabled={isActionLoading}
                    />
                  ) : (
                    <PageHeaderAction
                      icon={paused ? Play : Pause}
                      label={paused ? scheduleCopy.resume : scheduleCopy.pause}
                      tooltip={paused ? copy.resumeTooltip : copy.pauseTooltip}
                      onClick={handlePauseToggle}
                      disabled={isActionLoading}
                    />
                  ))}
                <PageHeaderMenu items={menu} />
                <DisabledReason reason={busyReason}>
                  <Button onClick={handleRunNow} disabled={isActionLoading || running}>
                    {scheduleCopy.runNow}
                  </Button>
                </DisabledReason>
              </>
            }
          />

          <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6 pt-2">
            <ScrollArea className="h-full">
              <div className="pb-6">
                {isLoadingSchedule && !scheduleDetails && (
                  <div className="biorouter-settings-list" aria-hidden>
                    <div className="biorouter-settings-row flex items-center justify-between gap-3 px-3 py-2.5">
                      <Skeleton className="h-4 w-24" />
                      <Skeleton className="h-4 w-40" />
                    </div>
                    <div className="biorouter-settings-row flex items-center justify-between gap-3 px-3 py-2.5">
                      <Skeleton className="h-4 w-24" />
                      <Skeleton className="h-4 w-56" />
                    </div>
                  </div>
                )}
                {scheduleError && (
                  <Note tone="danger" role="alert">
                    {scheduleError}
                  </Note>
                )}
                {scheduleDetails && (
                  <div className="biorouter-settings-list">
                    <DefinitionRow
                      label={copy.runs}
                      tooltip={<span className="font-mono">{scheduleDetails.cron}</span>}
                    >
                      {readableCronOf(scheduleDetails.cron)}
                    </DefinitionRow>
                    <DefinitionRow
                      label={copy.workflow}
                      tooltip={<span className="font-mono">{scheduleDetails.source}</span>}
                    >
                      {fileNameOf(scheduleDetails.source)}
                    </DefinitionRow>
                    <DefinitionRow
                      label={copy.lastRun}
                      tooltip={
                        scheduleDetails.last_run
                          ? formatToLocalDateWithTimezone(scheduleDetails.last_run)
                          : undefined
                      }
                    >
                      {formatRunTime(scheduleDetails.last_run) ?? scheduleCopy.notRunYet}
                    </DefinitionRow>
                    {/* Issue #56. Without this the only record of a repeatedly
                        failing job is a daemon log line: each run mints a fresh
                        session, so there is no chat to open and read either. */}
                    {scheduleDetails.last_error && (
                      <DefinitionRow label={copy.lastError} tone="danger">
                        {scheduleDetails.last_error}
                      </DefinitionRow>
                    )}
                    {running && scheduleDetails.process_start_time && (
                      <DefinitionRow
                        label={copy.started}
                        tooltip={formatToLocalDateWithTimezone(scheduleDetails.process_start_time)}
                      >
                        {formatRunTime(scheduleDetails.process_start_time)}
                      </DefinitionRow>
                    )}
                    {running && (
                      <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-1">
                        <span className="text-body text-text-muted">
                          {scheduleCopy.status.running}
                        </span>
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => void handleInspect()}
                          disabled={isActionLoading}
                        >
                          {copy.openChat}
                        </Button>
                      </div>
                    )}
                  </div>
                )}

                <div className="biorouter-settings-section">
                  <div className="biorouter-settings-section-header">
                    <h2 className="text-caps text-text-muted">{copy.recentRuns}</h2>
                  </div>
                  {isLoadingSessions && sessions.length === 0 && (
                    <div className="biorouter-list-shell" aria-hidden>
                      <RunRowSkeleton />
                      <RunRowSkeleton />
                      <RunRowSkeleton />
                    </div>
                  )}
                  {sessionsError && (
                    <Note tone="danger" role="alert">
                      {sessionsError}
                    </Note>
                  )}
                  {!isLoadingSessions && !sessionsError && sessions.length === 0 && (
                    <EmptyState
                      compact
                      icon={MessageSquareText}
                      title={copy.noRunsTitle}
                      description={copy.noRunsDescription}
                    />
                  )}
                  {sessions.length > 0 && (
                    <div className="biorouter-list-shell">
                      {sessions.map((session) => (
                        <RunRow
                          key={session.id}
                          session={session}
                          tier={runTiers[session.id]}
                          onOpen={() => loadSession(session.id)}
                        />
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </ScrollArea>
          </ReadableContent>
        </div>
      </MainPanelLayout>

      <ScheduleModal
        isOpen={isModalOpen}
        onClose={() => setIsModalOpen(false)}
        onSubmit={handleModalSubmit}
        schedule={scheduleDetails}
        isLoadingExternally={isActionLoading}
        apiErrorExternally={null}
        initialDeepLink={null}
      />
      <ConfirmationModal
        isOpen={isDeleteOpen}
        title={scheduleCopy.deleteTitle(name)}
        message={DELETE_SCHEDULE_MESSAGE}
        confirmLabel={scheduleCopy.deleteConfirm}
        cancelLabel={scheduleCopy.cancel}
        confirmVariant="destructive"
        isSubmitting={isActionLoading}
        onConfirm={() => void handleDelete()}
        onCancel={() => setIsDeleteOpen(false)}
      />
    </>
  );
};

export default ScheduleDetailView;
