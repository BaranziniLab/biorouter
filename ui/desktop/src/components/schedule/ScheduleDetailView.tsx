import React, { useState, useEffect } from 'react';
import { Button } from '../ui/button';
import { ScrollArea } from '../ui/scroll-area';
import BackButton from '../ui/BackButton';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { EmptyState } from '../ui/empty-state';
import {
  getScheduleSessions,
  runScheduleNow,
  pauseSchedule,
  unpauseSchedule,
  updateSchedule,
  listSchedules,
  killRunningJob,
  inspectRunningJob,
  ScheduledJob,
} from '../../schedule';
import SessionHistoryView from '../sessions/SessionHistoryView';
import { ScheduleModal, NewSchedulePayload } from './ScheduleModal';
import { ScheduleStatus, readableCronOf } from './scheduleStatus';
import { toastError, toastSuccess } from '../../toasts';
import { Pause, Play, Edit, Square, Eye, MessageSquareText, Target } from '../icons/app-icons';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ReadableContent } from '../Layout/ReadableContent';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { formatToLocalDateWithTimezone } from '../../utils/date';
import { billedSessionTokenEstimate, formatBilledTokenEstimate } from '../../utils/billedTokens';
import { scheduleDisplayName } from '../../utils/builtins';
import { getSession, Session } from '../../api';
import { userActionHeaders } from '../../utils/userAction';

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
 * A definition row: the label on the left, the fact on the right.
 *
 * This is astryx §4.5's definition-row pattern, and it replaces the
 * `**Label:** value` colon sentences inside a `<Card>` that this view used to
 * stack. It is literally `.biorouter-settings-row` — the same 40px hairline row
 * Settings uses for a label and the control it names — because a fact and a
 * control are the same shape of thing on the page, and giving the Scheduler its
 * own near-miss of that row is how the two drift.
 *
 * ⚠ The rows must be DIRECT children of `.biorouter-settings-list`, so this
 * component returns the row itself and never a wrapper:
 * `.biorouter-settings-row:last-child` is relative to a row's own parent, and a
 * per-row wrapper makes every row a `:last-child` and suppresses every hairline
 * in the list.
 *
 * At narrow widths the value wraps beneath the label rather than being squeezed
 * against it, which is what `flex-wrap` plus `justify-between` buys.
 */
function DefinitionRow({
  label,
  children,
  mono = false,
  tone,
  title,
}: {
  label: string;
  children: React.ReactNode;
  mono?: boolean;
  tone?: 'danger';
  title?: string;
}) {
  return (
    <div className="biorouter-settings-row flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1 px-3 py-2.5 text-text-default">
      <span className="text-label text-text-muted">{label}</span>
      <span
        title={title}
        className={[
          'min-w-0 text-label [overflow-wrap:anywhere]',
          mono ? 'font-mono' : '',
          tone === 'danger' ? 'text-text-danger' : 'text-text-default',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        {children}
      </span>
    </div>
  );
}

/**
 * One past run, as a hairline row.
 *
 * ⚠ This is a SMALLER row than `SessionListView`'s, deliberately. That row is
 * welded to History's machinery — the right-click context menu, the row-action
 * builders, the selection checkbox, the search highlighter and a `sessionRef`
 * registry — none of which exists here, and lifting it would have meant either
 * importing all of it or splitting it out in a PR that is about a visual
 * vocabulary. What IS shared is everything that decides how a chat LOOKS: the
 * kind glyph (`ChatKindIcon`), the billed-token selection and its format, and
 * `.biorouter-list-row`.
 */
function RunRow({ session, onOpen }: { session: ScheduleSessionMeta; onOpen: () => void }) {
  // `SessionDisplayInfo` is camelCase and `billedSessionTokenEstimate` reads the
  // session row's snake_case columns, so the mapping happens here rather than
  // the figure being re-derived: issue #1's whole point is that ONE helper
  // decides billed-vs-last-turn, and a second call site picking
  // `accumulatedTotalTokens` by hand is how the two answers diverge again.
  const billed = billedSessionTokenEstimate({
    accumulated_total_tokens: session.accumulatedTotalTokens,
    accumulated_input_tokens: session.accumulatedInputTokens,
    accumulated_output_tokens: session.accumulatedOutputTokens,
    total_tokens: session.totalTokens,
  });

  return (
    <div className="biorouter-list-row group flex items-center gap-3 px-3 py-2">
      <button
        type="button"
        onClick={onOpen}
        className="min-w-0 flex-1 cursor-pointer rounded-inner text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
        aria-label={`Open run ${session.name || session.id}`}
      >
        <div className="flex min-w-0 items-center gap-1.5">
          <ChatKindIcon
            session={{ name: session.name, session_type: 'scheduled' }}
            className="h-4 w-4"
          />
          <h3 className="min-w-0 truncate text-label" title={session.name || session.id}>
            {session.name || <span className="font-mono">{session.id}</span>}
          </h3>
        </div>
        {session.workingDir && (
          <p
            className="mt-0.5 truncate font-mono text-supporting text-text-muted"
            title={session.workingDir}
          >
            {session.workingDir}
          </p>
        )}
      </button>

      {/* §3.10, one optical axis per row: a 20px box with `items-center`, and
          `tabular-nums` in min-width cells so the figures form real columns. */}
      <div className="flex h-5 shrink-0 items-center gap-3 font-mono text-supporting text-text-muted tabular-nums">
        <span className="whitespace-nowrap">
          {session.createdAt ? formatToLocalDateWithTimezone(session.createdAt) : '—'}
        </span>
        {session.messageCount !== undefined && (
          <span className="flex items-center gap-2">
            <MessageSquareText className="h-3 w-3" />
            <span className="sr-only">Messages: </span>
            <span className="min-w-8 whitespace-nowrap text-right">{session.messageCount}</span>
          </span>
        )}
        {billed && (
          <span
            className="flex items-center gap-2"
            title={
              billed.lowerBound
                ? 'At least this many tokens; only last-turn usage is available for this older chat'
                : 'Billed tokens across every turn, including recorded cache usage'
            }
          >
            <Target className="h-3 w-3" />
            <span className="sr-only">Billed tokens: </span>
            <span className="min-w-12 whitespace-nowrap text-right">
              {formatBilledTokenEstimate(billed)}
            </span>
          </span>
        )}
      </div>
    </div>
  );
}

const RunRowSkeleton: React.FC = () => (
  <div className="biorouter-list-row flex items-center gap-3 px-3 py-2">
    <div className="min-w-0 flex-1">
      <Skeleton className="h-4 w-56" />
      <Skeleton className="mt-2 h-3 w-40" />
    </div>
  </div>
);

/**
 * One schedule, rebuilt on the standard scaffold (astryx §4.5).
 *
 * What it stopped being: an `h-screen w-full … bg-background-muted` shell — the
 * exact anti-pattern `MainPanelLayout`'s own comment warns breaks embedded
 * panes — holding a `<Card>` of `**Label:** value` sentences, three
 * per-semantic tinted button variants that exist nowhere else in the app, and a
 * three-column grid of cards for the run history. That was the box in a box in
 * a box.
 *
 * What it is now: `MainPanelLayout` → `ReadableContent size="chat"`, a §4.2
 * header whose title IS the schedule, one hairline list of definition rows, one
 * control strip, and the runs as a hairline list of rows.
 */
const ScheduleDetailView: React.FC<ScheduleDetailViewProps> = ({ scheduleId, onNavigateBack }) => {
  const [sessions, setSessions] = useState<ScheduleSessionMeta[]>([]);
  const [isLoadingSessions, setIsLoadingSessions] = useState(false);
  const [sessionsError, setSessionsError] = useState<string | null>(null);

  const [scheduleDetails, setScheduleDetails] = useState<ScheduledJob | null>(null);
  const [isLoadingSchedule, setIsLoadingSchedule] = useState(false);
  const [scheduleError, setScheduleError] = useState<string | null>(null);

  const [isActionLoading, setIsActionLoading] = useState(false);
  const [isRunPending, setIsRunPending] = useState(false);

  const [selectedSession, setSelectedSession] = useState<Session | null>(null);
  const [isLoadingSession, setIsLoadingSession] = useState(false);
  const [sessionError, setSessionError] = useState<string | null>(null);

  const [isModalOpen, setIsModalOpen] = useState(false);

  const fetchSessions = async (sId: string) => {
    setIsLoadingSessions(true);
    setSessionsError(null);
    try {
      const data = await getScheduleSessions(sId, 20);
      setSessions(data);
    } catch (err) {
      setSessionsError(
        err instanceof Error ? err.message : 'Could not load the chats for this schedule'
      );
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
      } else {
        setScheduleError('Schedule not found');
      }
    } catch (err) {
      setScheduleError(err instanceof Error ? err.message : 'Failed to fetch schedule');
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
      if (newSessionId === 'CANCELLED') {
        toastSuccess({ title: 'Job stopped', msg: 'The job was stopped while starting up.' });
      } else {
        toastSuccess({ title: 'Schedule triggered', msg: `New chat session ID: ${newSessionId}` });
      }
      await fetchSessions(scheduleId);
      await fetchSchedule(scheduleId);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to trigger schedule';
      toastError({
        title: 'Run schedule error',
        msg: errorMsg,
      });
    } finally {
      setIsRunPending(false);
      setIsActionLoading(false);
    }
  };

  const handlePauseToggle = async () => {
    if (!scheduleId || !scheduleDetails) return;
    setIsActionLoading(true);
    try {
      if (scheduleDetails.paused) {
        await unpauseSchedule(scheduleId);
        toastSuccess({ title: 'Schedule unpaused', msg: `Unpaused "${scheduleId}"` });
      } else {
        await pauseSchedule(scheduleId);
        toastSuccess({ title: 'Schedule paused', msg: `Paused "${scheduleId}"` });
      }
      await fetchSchedule(scheduleId);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Operation failed';
      toastError({
        title: 'Pause/Unpause Error',
        msg: errorMsg,
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleKill = async () => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      const result = await killRunningJob(scheduleId);
      toastSuccess({ title: 'Job stopped', msg: result.message });
      await fetchSchedule(scheduleId);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to kill job';
      toastError({
        title: 'Could not stop the job',
        msg: errorMsg,
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleInspect = async () => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      const result = await inspectRunningJob(scheduleId);
      if (result.sessionId) {
        const duration = result.runningDurationSeconds
          ? `${Math.floor(result.runningDurationSeconds / 60)}m ${result.runningDurationSeconds % 60}s`
          : 'Unknown';
        toastSuccess({
          title: 'Job inspection',
          msg: `Session ID: ${result.sessionId}\nRunning for: ${duration}`,
        });
      } else {
        toastSuccess({ title: 'Job inspection', msg: 'No detailed information available' });
      }
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to inspect job';
      toastError({
        title: 'Inspect job error',
        msg: errorMsg,
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleModalSubmit = async (payload: NewSchedulePayload | string) => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      await updateSchedule(scheduleId, payload as string);
      toastSuccess({ title: 'Schedule updated', msg: `Updated "${scheduleId}"` });
      await fetchSchedule(scheduleId);
      setIsModalOpen(false);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to update schedule';
      toastError({
        title: 'Update schedule error',
        msg: errorMsg,
      });
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
      const msg = err instanceof Error ? err.message : 'Could not load this chat';
      setSessionError(msg);
      toastError({ title: 'Failed to load chat', msg });
    } finally {
      setIsLoadingSession(false);
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

  if (!scheduleId) {
    return (
      <MainPanelLayout>
        <ReadableContent size="chat" className="px-6 pt-6">
          <BackButton onClick={onNavigateBack} />
          <h1 className="text-title mt-6 mb-1">Schedule not found</h1>
          <p className="text-secondary text-text-muted">
            No schedule id was provided. Go back to the schedule list.
          </p>
        </ReadableContent>
      </MainPanelLayout>
    );
  }

  const readableCron = scheduleDetails ? readableCronOf(scheduleDetails.cron) : '';
  const running = scheduleDetails?.currently_running ?? false;

  return (
    <>
      <MainPanelLayout>
        <div className="flex-1 flex flex-col min-h-0">
          {/* §4.2 — one page header: a FULL-BLEED hairline, `text-title`, and
              one supporting line. The title is the schedule, not the word
              "Schedule Details"; the id it used to spell out in a "Viewing
              Schedule ID:" sentence is a definition row below. */}
          <div className="flex-shrink-0 border-b border-border-subtle">
            <ReadableContent size="chat" className="px-6 pt-6 pb-6">
              <BackButton onClick={onNavigateBack} />
              <h1 className="text-title mt-6 mb-1 min-w-0 break-words">
                {scheduleDisplayName(scheduleId)}
              </h1>
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-supporting text-text-muted">
                <span>{readableCron || 'Loading…'}</span>
                {scheduleDetails && <ScheduleStatus job={scheduleDetails} />}
              </div>
            </ReadableContent>
          </div>

          <ReadableContent size="chat" className="flex-1 min-h-0 relative px-6">
            <ScrollArea className="h-full">
              <div className="pb-8">
                <div className="biorouter-settings-section">
                  <div className="biorouter-settings-section-header">
                    <h2 className="text-caps text-text-muted">Schedule</h2>
                  </div>
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
                      <div className="biorouter-settings-row flex items-center justify-between gap-3 px-3 py-2.5">
                        <Skeleton className="h-4 w-24" />
                        <Skeleton className="h-4 w-32" />
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
                      <DefinitionRow label="Runs">{readableCron}</DefinitionRow>
                      <DefinitionRow label="Cron" mono>
                        {scheduleDetails.cron}
                      </DefinitionRow>
                      <DefinitionRow label="Workflow" mono title={scheduleDetails.source}>
                        {scheduleDetails.source}
                      </DefinitionRow>
                      <DefinitionRow label="Last run">
                        {formatToLocalDateWithTimezone(scheduleDetails.last_run)}
                      </DefinitionRow>
                      {/*
                        Issue #56. Without this the only record of a repeatedly
                        failing job is a daemon log line: each run mints a fresh
                        session, so there is no chat to open and read either.
                      */}
                      {scheduleDetails.last_error && (
                        <DefinitionRow label="Last error" tone="danger">
                          {scheduleDetails.last_error}
                        </DefinitionRow>
                      )}
                      {running && scheduleDetails.current_session_id && (
                        <DefinitionRow label="Current chat" mono>
                          {scheduleDetails.current_session_id}
                        </DefinitionRow>
                      )}
                      {running && scheduleDetails.process_start_time && (
                        <DefinitionRow label="Started">
                          {formatToLocalDateWithTimezone(scheduleDetails.process_start_time)}
                        </DefinitionRow>
                      )}
                      <DefinitionRow label="Id" mono>
                        {scheduleDetails.id}
                      </DefinitionRow>
                    </div>
                  )}
                </div>

                <div className="biorouter-settings-section">
                  <div className="biorouter-settings-section-header">
                    <h2 className="text-caps text-text-muted">Actions</h2>
                  </div>
                  {/* One control strip and one button ladder. The per-semantic
                      blue and green outlined variants this replaced exist
                      nowhere else in the app, and what they were encoding —
                      "this schedule is paused" — is already said by the status
                      line in the header. */}
                  <div className="biorouter-settings-control-strip">
                    <Button onClick={handleRunNow} disabled={isActionLoading || running}>
                      Run now
                    </Button>

                    {scheduleDetails && !running && (
                      <>
                        <Button
                          onClick={handlePauseToggle}
                          variant="secondary"
                          disabled={isActionLoading}
                        >
                          {scheduleDetails.paused ? <Play /> : <Pause />}
                          {scheduleDetails.paused ? 'Unpause' : 'Pause'}
                        </Button>
                        <Button
                          onClick={() => setIsModalOpen(true)}
                          variant="secondary"
                          disabled={isActionLoading}
                        >
                          <Edit />
                          Edit
                        </Button>
                      </>
                    )}

                    {running && (
                      <>
                        <Button
                          onClick={handleInspect}
                          variant="secondary"
                          disabled={isActionLoading}
                        >
                          <Eye />
                          Inspect run
                        </Button>
                        {/* The app's tinted danger fill, not a hand-rolled
                            danger-bordered outline: stopping a run throws away
                            work in flight, which is what `destructive` is for. */}
                        <Button
                          onClick={handleKill}
                          variant="destructive"
                          disabled={isActionLoading}
                        >
                          <Square />
                          Stop run
                        </Button>
                      </>
                    )}
                  </div>

                  {isRunPending && (
                    <Note role="status" className="mt-3">
                      Waiting for the scheduled run to finish. This can take several minutes.
                    </Note>
                  )}

                  {/* One note, only when it applies. These were two sentences in
                      raw `text-text-warning` under the buttons. */}
                  {running && (
                    <Note tone="warning" icon={Pause} className="mt-3">
                      This schedule is running. It cannot be triggered again or edited until the run
                      finishes.
                    </Note>
                  )}
                  {!running && scheduleDetails?.paused && (
                    <Note tone="warning" icon={Pause} className="mt-3">
                      This schedule is paused and will not run automatically. Run it now to trigger
                      it once, or unpause to resume automatic runs.
                    </Note>
                  )}
                </div>

                <div className="biorouter-settings-section">
                  <div className="biorouter-settings-section-header">
                    <h2 className="text-caps text-text-muted">Recent chats</h2>
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
                      title="No runs yet"
                      description="Each run of this schedule starts a new chat. They will appear here once it has run."
                    />
                  )}
                  {sessions.length > 0 && (
                    <div className="biorouter-list-shell">
                      {sessions.map((session) => (
                        <RunRow
                          key={session.id}
                          session={session}
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
    </>
  );
};

export default ScheduleDetailView;
