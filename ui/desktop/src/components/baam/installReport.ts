// The one failure report Browse skills keeps on screen, and what retracts it.
//
// ⚠ **An error toast outlives the thing it reports, so something has to own
// it.** Errors do not expire (`toastError` sets `autoClose: false`, on purpose:
// a failure that vanishes unread was never reported). But Browse skills raised
// a fresh one per run and never took one back, so after a retry succeeded the
// screen said both "Alignment Files was not installed" and "10 skills installed
// | … alignment-files (10 skills)" — and "3 selections were not installed"
// stayed up after two of the three had installed.
//
// So one report exists at a time, naming the rows that are STILL not installed.
// After each run it becomes: the rows it named that this run did not try again
// (and that nothing else has installed since), plus this run's failures. When
// that set changes, the old toast is dismissed and the new one raised; when it
// empties, the old toast is simply dismissed. A report the user closed has been
// read, and is not raised again for rows nobody retried.

import { toast } from 'react-toastify';
import { toastError } from '../../toasts';
import { failedToast, type FailedInstall } from './installCopy';

/** A failed marketplace row, by its registry id. */
export interface FailedRow extends FailedInstall {
  id: string;
}

let report: { toastId: string | number; failures: FailedRow[] } | null = null;

const sameReport = (a: readonly FailedRow[], b: readonly FailedRow[]) => {
  const x = failedToast(a);
  const y = failedToast(b);
  return x.title === y.title && x.msg === y.msg;
};

/**
 * The rows still not installed after a run: `previous` minus what the run
 * attempted or what has since landed, plus the run's own failures.
 */
export function stillNotInstalled(
  previous: readonly FailedRow[],
  run: {
    attempted: ReadonlySet<string>;
    failures: readonly FailedRow[];
    isInstalled: (id: string) => boolean;
  }
): FailedRow[] {
  return [
    ...previous.filter((row) => !run.attempted.has(row.id) && !run.isInstalled(row.id)),
    ...run.failures,
  ];
}

/** Bring the failure report up to date with one finished install run. */
export function reportInstallRun(run: {
  attempted: ReadonlySet<string>;
  failures: readonly FailedRow[];
  isInstalled: (id: string) => boolean;
}): void {
  const live = report !== null && toast.isActive(report.toastId);
  const previous = live && report ? report.failures : [];
  const next = stillNotInstalled(previous, run);

  if (live && report && sameReport(previous, next)) {
    // Same words on screen: keep the toast the user may be reading. Raising a
    // copy under the same id while the old one animates out is swallowed.
    report = { toastId: report.toastId, failures: next };
    return;
  }
  if (report) toast.dismiss(report.toastId);
  report = next.length > 0 ? { toastId: toastError(failedToast(next)), failures: next } : null;
}

/** Tests only: forget the report without touching any toast. */
export function resetInstallReport(): void {
  report = null;
}
