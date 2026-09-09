import { describe, expect, it } from 'vitest';
import { MAX_SCHEDULE_NAME_LENGTH, scheduleNameProblem } from './scheduleName';

describe('scheduleNameProblem', () => {
  it('accepts the names the placeholder advertises', () => {
    for (const name of ['daily-summary-job', 'nightly_cohort', 'probe2', 'A-1_b']) {
      expect(scheduleNameProblem(name)).toBeNull();
    }
  });

  /**
   * The reported bug: this name reached the daemon, was refused by
   * `validate_schedule_id`, and came back as "Unexpected response format" —
   * a transport-shaped report for a validation problem the user could fix.
   */
  it('names the rule for a hostile string instead of letting it be submitted', () => {
    expect(scheduleNameProblem('<img src=x onerror=alert(1)>')).toBe(
      "The name may only contain letters, digits, '-' and '_'."
    );
    expect(scheduleNameProblem('{{ 7*7 }}')).toBe(
      "The name may only contain letters, digits, '-' and '_'."
    );
  });

  /**
   * The character set is the arbitrary-file-write refusal, so the separators
   * matter more than the angle brackets do: the name is interpolated into a
   * filename, and `Path::join` throws its base away for an absolute argument.
   */
  it('rejects every separator that could steer a path', () => {
    for (const name of ['/tmp/pwned', '../escape', 'a b', 'a.b', 'C:\\x', 'a\nb']) {
      expect(scheduleNameProblem(name)).not.toBeNull();
    }
  });

  /**
   * `$` without the `m` flag ends the string in JavaScript, but a rule this
   * cheap to get wrong is worth pinning: a trailing newline must not pass.
   */
  it('does not let a trailing newline through', () => {
    expect(scheduleNameProblem('ok-name\n')).not.toBeNull();
  });

  it('reports an empty name rather than an unreadable one', () => {
    expect(scheduleNameProblem('')).toBe('Enter a name for this schedule.');
  });

  it('holds the daemon length limit', () => {
    expect(MAX_SCHEDULE_NAME_LENGTH).toBe(64);
    expect(scheduleNameProblem('a'.repeat(MAX_SCHEDULE_NAME_LENGTH))).toBeNull();
    expect(scheduleNameProblem('a'.repeat(MAX_SCHEDULE_NAME_LENGTH + 1))).toBe(
      'The name must be 64 characters or fewer.'
    );
  });
});
