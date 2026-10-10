import { Session } from '../api';
import { groupByChatDate } from './chatDateBuckets';

export interface DateGroup {
  /** Stable bucket key (`today`, `yesterday`, `week`, `month30`, `2026-08`, `unknown`). */
  key: string;
  /** Sentence-case label: Today, Yesterday, Previous 7 days, Previous 30 days, a month. */
  label: string;
  sessions: Session[];
}

/**
 * Chat history's date groups: the same coarse buckets the sidebar uses
 * (`chatDateBuckets.ts`), so a chat sits under the same heading in both places.
 *
 * This replaced one heading per day in a third format ("Tuesday, October 7"),
 * which made a long history a column of near-identical headings. Buckets come
 * newest first; inside one, sessions keep the order they arrive in (the list
 * route's `updated_at` descending).
 */
export function groupSessionsByDate(sessions: Session[], now: number = Date.now()): DateGroup[] {
  return groupByChatDate(sessions, (session) => session.updated_at, now).map(
    ({ key, label, items }) => ({ key, label, sessions: items })
  );
}
