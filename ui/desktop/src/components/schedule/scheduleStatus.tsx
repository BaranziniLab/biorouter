import cronstrue from 'cronstrue';
import { ScheduledJob } from '../../schedule';
import { cn } from '../../utils';
import { StatusDot, type StatusDotTone } from '../ui/status-dot';
import { scheduleCopy } from './copy';

/**
 * The cron expression as a sentence, with the expression itself as the
 * fallback. One definition, because the list row and the detail header both
 * print it and a schedule that reads "At 03:00 AM" on one screen and
 * `0 0 3 * * *` on the other is two answers to one question.
 */
export function readableCronOf(cron: string): string {
  try {
    return cronstrue.toString(cron);
  } catch {
    return cron;
  }
}

/**
 * A schedule's states, in the one vocabulary both Scheduler surfaces read.
 *
 * The states are not exclusive and are deliberately not collapsed into one word: a paused
 * schedule whose last manual run failed is both paused and failed, and picking one of those to
 * show would hide the other. `Scheduled` is the resting state and is stated rather than left
 * blank: a row that says nothing does not tell you whether the schedule is live.
 *
 * The dot shows the one state that matters most (running, then failed, then paused, then
 * scheduled); the words say all of them.
 */
export function scheduleState(job: ScheduledJob): {
  tone: StatusDotTone;
  live: boolean;
  words: string[];
  failed: boolean;
} {
  const running = Boolean(job.currently_running);
  // Issue #56: a failed tick leaves no chat to open, so `last_error` is the
  // only record that a run went wrong. It is cleared by the next success.
  const failed = Boolean(job.last_error) && !running;
  const words: string[] = [];
  if (running) words.push(scheduleCopy.status.running);
  if (job.paused) words.push(scheduleCopy.status.paused);
  if (failed) words.push(scheduleCopy.status.failed);
  if (words.length === 0) words.push(scheduleCopy.status.scheduled);

  // Spec 3.10: Running pulses, Paused is muted, Failed is danger, Scheduled is success.
  // Motion means "still going" (astryx §4.4): only Running is live.
  const tone: StatusDotTone = running
    ? 'success'
    : failed
      ? 'danger'
      : job.paused
        ? 'idle'
        : 'success';
  return { tone, live: running, words, failed };
}

/**
 * The 8px status dot that leads a schedule row (Codex's per-thread circle). Decorative: the
 * row says the state in words on its second line, so the dot is hidden from assistive
 * technology rather than read twice.
 */
export function ScheduleStatusDot({ job, className }: { job: ScheduledJob; className?: string }) {
  const { tone, live } = scheduleState(job);
  return <StatusDot tone={tone} live={live} className={className} />;
}

/**
 * The dot plus the words: a schedule's state said once, for the detail band. Never a filled
 * pill (the hand-mixed 15% washes the settings vocabulary bans); the dot carries the hue, the
 * words carry the meaning.
 */
export function ScheduleStatus({ job, className }: { job: ScheduledJob; className?: string }) {
  const { words, failed } = scheduleState(job);
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1.5 text-supporting',
        failed ? 'text-text-danger' : 'text-text-muted',
        className
      )}
    >
      <ScheduleStatusDot job={job} />
      {words.join(' · ')}
    </span>
  );
}

/**
 * A run time for a meta line: "Oct 7, 2:00 PM", in the sans face with tabular figures. The full
 * date with seconds and the time zone goes in a tooltip (`formatToLocalDateWithTimezone`).
 * Returns null for a missing or unreadable date, so the caller can say "Not run yet".
 */
export function formatRunTime(iso?: string | null): string | null {
  if (!iso) return null;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}
