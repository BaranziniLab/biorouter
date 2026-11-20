/**
 * Tells main what this window's dictation is doing, for the on-screen indicator
 * (`utils/dictationIndicator.ts`) and for routing the fn gestures (`utils/dictationFnKey.ts`): an
 * activity at once when it changes, and the microphone level while recording, at most ten times a
 * second. Only an activity word and a number leave the window, never audio or text.
 */
import {
  INDICATOR_REPORT_INTERVAL_MS,
  type DictationActivity,
  type DictationActivityReport,
} from '../../utils/dictationIndicator';
import { levelToMeter } from './level';
import {
  getDictationLevel,
  getDictationSnapshot,
  isDictationRunning,
  subscribeDictation,
  subscribeDictationLevel,
  subscribeDictationRunning,
} from './dictationSession';

/**
 * This window's activity: the current dictation's, or `transcribing` while a dictation another
 * pane displaced still finishes in the background.
 */
export function windowDictationActivity(): DictationActivity {
  const { activity } = getDictationSnapshot();
  if (activity === 'arming' || activity === 'recording') return activity;
  if (activity === 'transcribing' || isDictationRunning()) return 'transcribing';
  return 'idle';
}

export function installIndicatorReporter(
  send: ((report: DictationActivityReport) => void) | undefined = window.electron
    ?.dictationIndicator,
  now: () => number = () => performance.now()
): () => void {
  if (!send) return () => undefined;
  let last: DictationActivity = 'idle';
  let lastLevelAt = -Infinity;
  const report = (activity: DictationActivity) =>
    send({
      activity,
      level: activity === 'recording' ? levelToMeter(getDictationLevel()) : 0,
    });
  const onActivity = () => {
    const activity = windowDictationActivity();
    if (activity === last) return;
    last = activity;
    lastLevelAt = now();
    report(activity);
  };
  const onLevel = () => {
    if (last !== 'recording') return;
    const at = now();
    if (at - lastLevelAt < INDICATOR_REPORT_INTERVAL_MS) return;
    lastLevelAt = at;
    report('recording');
  };
  const offActivity = subscribeDictation(onActivity);
  const offRunning = subscribeDictationRunning(onActivity);
  const offLevel = subscribeDictationLevel(onLevel);
  return () => {
    offActivity();
    offRunning();
    offLevel();
    if (last !== 'idle') send({ activity: 'idle', level: 0 });
  };
}
