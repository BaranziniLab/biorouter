import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationActivityReport } from '../../utils/dictationIndicator';
import {
  resetDictationSessionForTests,
  setDictationLevelForTests,
  setDictationSnapshotForTests,
} from './dictationSession';
import { installIndicatorReporter } from './indicatorReport';

describe('what a window tells main for the indicator', () => {
  let now = 0;
  let reports: DictationActivityReport[];
  let stop: () => void;
  beforeEach(() => {
    resetDictationSessionForTests();
    now = 0;
    reports = [];
    stop = installIndicatorReporter(
      (report) => reports.push(report),
      () => now
    );
  });
  afterEach(() => {
    stop();
    resetDictationSessionForTests();
  });

  it('sends each activity once, as it changes', () => {
    setDictationSnapshotForTests({ key: 'a', activity: 'arming' });
    setDictationSnapshotForTests({ key: 'a', activity: 'arming', elapsedS: 1 });
    setDictationSnapshotForTests({ key: 'a', activity: 'recording' });
    setDictationSnapshotForTests({ key: 'a', activity: 'transcribing' });
    setDictationSnapshotForTests({ key: 'a', activity: 'idle' });
    expect(reports.map((report) => report.activity)).toEqual([
      'arming',
      'recording',
      'transcribing',
      'idle',
    ]);
  });

  it('sends the level on the meter scale while recording, at most ten times a second', () => {
    setDictationSnapshotForTests({ key: 'a', activity: 'recording' });
    reports = [];
    now = 50;
    setDictationLevelForTests(0.05);
    expect(reports).toEqual([]);
    now = 100;
    setDictationLevelForTests(0.1);
    setDictationLevelForTests(0.2);
    now = 199;
    setDictationLevelForTests(0.3);
    expect(reports).toHaveLength(1);
    expect(reports[0].activity).toBe('recording');
    // -20 dBFS is two thirds of the way up a -60 to 0 dBFS meter.
    expect(reports[0].level).toBeCloseTo(2 / 3, 5);
  });

  it('sends no level outside a recording, and idle when it stops listening', () => {
    setDictationLevelForTests(0.5);
    expect(reports).toEqual([]);
    setDictationSnapshotForTests({ key: 'a', activity: 'recording' });
    stop();
    expect(reports[reports.length - 1]).toEqual({ activity: 'idle', level: 0 });
    stop = vi.fn();
  });
});
