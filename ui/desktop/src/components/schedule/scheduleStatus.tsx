import React from 'react';
import cronstrue from 'cronstrue';
import { ScheduledJob } from '../../schedule';
import { cn } from '../../utils';

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

type Tone = 'success' | 'muted' | 'danger';

const DOT: Record<Tone, string> = {
  success: 'bg-background-success',
  muted: 'bg-background-medium',
  danger: 'bg-background-danger',
};

const INK: Record<Tone, string> = {
  success: 'text-text-muted',
  muted: 'text-text-muted',
  danger: 'text-text-danger',
};

/**
 * One state, said as a dot plus a word.
 *
 * ⚠ **Never a filled pill.** The three states used to be `rounded-md` chips on
 * hand-mixed `bg-background-{tone}/15` washes — the alphas the settings
 * vocabulary bans (rule 4), and a fill §2.5 reserves for a `Note` rather than
 * for a word inside a row. The dot carries the hue; the word carries the
 * meaning; the row keeps its own ground.
 */
function StatusItem({ tone, label, pulse }: { tone: Tone; label: string; pulse?: boolean }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-supporting', INK[tone])}>
      <span
        className={cn('h-1.5 w-1.5 shrink-0 rounded-full', DOT[tone], pulse && 'animate-pulse')}
        aria-hidden
      />
      {label}
    </span>
  );
}

/**
 * Every state a schedule can be in, in the one vocabulary both Scheduler
 * surfaces read.
 *
 * The states are not exclusive and are deliberately not collapsed into one
 * word: a paused schedule whose last manual run failed is both paused and
 * failed, and picking one of those to show would hide the other. `Scheduled` is
 * the resting state and is stated rather than left blank — a row that says
 * nothing does not tell you whether the schedule is live.
 *
 * Motion means "still going" (astryx §4.4): only `Running` pulses.
 */
export function ScheduleStatus({ job, className }: { job: ScheduledJob; className?: string }) {
  const running = job.currently_running;
  // Issue #56: a failed tick leaves no chat to open, so `last_error` is the
  // only record that a run went wrong. It is cleared by the next success.
  const failed = Boolean(job.last_error) && !running;
  const items: React.ReactNode[] = [];

  if (running) items.push(<StatusItem key="running" tone="success" label="Running" pulse />);
  if (job.paused) items.push(<StatusItem key="paused" tone="muted" label="Paused" />);
  if (failed) items.push(<StatusItem key="failed" tone="danger" label="Failed" />);
  if (items.length === 0) {
    items.push(<StatusItem key="scheduled" tone="success" label="Scheduled" />);
  }

  return <span className={cn('inline-flex shrink-0 items-center gap-3', className)}>{items}</span>;
}
