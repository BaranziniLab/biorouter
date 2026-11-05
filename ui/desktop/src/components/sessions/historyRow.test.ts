import { describe, expect, it } from 'vitest';
import {
  folderName,
  historyRowTitle,
  runModelName,
  subagentOutcomeOf,
  SUBAGENT_OUTCOME_KEY,
} from './historyRow';
import { messageCount } from './copy';

describe('folderName', () => {
  it('names the last folder of a path', () => {
    expect(folderName('/Users/wgu/data')).toBe('data');
    expect(folderName('/Users/wgu/data/')).toBe('data');
    expect(folderName('C:\\Users\\wgu\\proj')).toBe('proj');
  });

  it('reads the root as itself and nothing as nothing', () => {
    expect(folderName('/')).toBe('/');
    expect(folderName('')).toBe('');
    expect(folderName(undefined)).toBe('');
  });
});

/**
 * V1-R2-10: a row's title does not repeat the kind its glyph already says, and does not show a
 * machine string where a name belongs.
 */
describe('historyRowTitle', () => {
  it('leaves an ordinary chat exactly as named', () => {
    expect(historyRowTitle({ name: 'Initial greeting' })).toBe('Initial greeting');
    expect(historyRowTitle({ name: 'Subagent haiku task', session_type: 'user' })).toBe(
      'Subagent haiku task'
    );
    expect(historyRowTitle({ name: '' })).toBe('');
    expect(historyRowTitle({})).toBe('');
  });

  it('shows an app chat by the app name, in sentence case', () => {
    expect(historyRowTitle({ name: 'app:living-news-router' })).toBe('Living news router');
    expect(historyRowTitle({ name: 'app:ucsf-versa-gpt55-kb-visualizer-smoke' })).toBe(
      'Ucsf versa gpt55 kb visualizer smoke'
    );
    expect(historyRowTitle({ name: 'app:snake_game' })).toBe('Snake game');
  });

  it('names the worker profile of a profile chat', () => {
    expect(historyRowTitle({ name: 'app:research-dashboard:Analyst-2' })).toBe(
      'Research dashboard (analyst 2)'
    );
  });

  it('keeps an app name it cannot read, rather than drawing an empty row', () => {
    expect(historyRowTitle({ name: 'app:' })).toBe('app:');
    expect(historyRowTitle({ name: 'app:---' })).toBe('app:---');
  });

  it('drops "Subagent:" from a sub-agent run, whose glyph and rail already say so', () => {
    const run = { session_type: 'sub_agent', parent_session_id: 'p1' };
    expect(historyRowTitle({ ...run, name: 'Subagent: Task: find pictures of dogs' })).toBe(
      'Task: find pictures of dogs'
    );
    expect(historyRowTitle({ ...run, name: 'subagent:   audit the cohort ' })).toBe(
      'audit the cohort'
    );
    // Whatever marks it as a run: the type, or a parent.
    expect(historyRowTitle({ parent_session_id: 'p1', name: 'Subagent: audit' })).toBe('audit');
  });

  it('keeps a sub-agent name that is only the prefix, so a row is never empty', () => {
    expect(historyRowTitle({ session_type: 'sub_agent', name: 'Subagent:' })).toBe('Subagent:');
    expect(historyRowTitle({ session_type: 'sub_agent', name: 'Subagent: ' })).toBe('Subagent: ');
  });

  it('leaves a sub-agent name that does not carry the prefix', () => {
    expect(historyRowTitle({ session_type: 'sub_agent', name: 'Subagent task' })).toBe(
      'Subagent task'
    );
    expect(historyRowTitle({ session_type: 'sub_agent', name: 'Orphan run' })).toBe('Orphan run');
  });
});

/**
 * V1-R3-6: a scheduled run says "scheduled" in its clock glyph, so the title does not say it a
 * second time (a search for a job listed ten rows reading "Scheduled job: daily-meditation").
 */
describe('historyRowTitle for a scheduled run', () => {
  const run = { session_type: 'scheduled' };

  it('shows the job id the Scheduler names the job by', () => {
    expect(historyRowTitle({ ...run, name: 'Scheduled job: daily-meditation' })).toBe(
      'daily-meditation'
    );
    expect(historyRowTitle({ ...run, name: 'scheduled job:   Weekly digest ' })).toBe(
      'Weekly digest'
    );
  });

  it('keeps a name that is only the prefix, so a row is never empty', () => {
    expect(historyRowTitle({ ...run, name: 'Scheduled job:' })).toBe('Scheduled job:');
    expect(historyRowTitle({ ...run, name: 'Scheduled job: ' })).toBe('Scheduled job: ');
  });

  it('leaves a run the person renamed, which carries no prefix', () => {
    expect(historyRowTitle({ ...run, name: 'Morning reading list' })).toBe('Morning reading list');
    expect(historyRowTitle({ ...run, name: 'Scheduled jobs I like' })).toBe(
      'Scheduled jobs I like'
    );
  });

  it('is keyed on the kind: an ordinary chat that happens to be titled so keeps its title', () => {
    expect(historyRowTitle({ session_type: 'user', name: 'Scheduled job: notes' })).toBe(
      'Scheduled job: notes'
    );
    expect(historyRowTitle({ name: 'Scheduled job: notes' })).toBe('Scheduled job: notes');
  });
});

describe('messageCount', () => {
  it('agrees in number', () => {
    expect(messageCount(1)).toBe('1 message');
    expect(messageCount(0)).toBe('0 messages');
    expect(messageCount(12)).toBe('12 messages');
  });
});

/**
 * A nested run's status cell reads the child's durable outcome record (`subagent_roster.rs`:
 * `subagent_outcome` . `v1` in `extension_data`). It draws only an END it recognises, and says
 * nothing rather than guess.
 */
describe('subagentOutcomeOf', () => {
  const data = (record: unknown) => ({ [SUBAGENT_OUTCOME_KEY]: record });

  it('reads the key the daemon writes', () => {
    expect(SUBAGENT_OUTCOME_KEY).toBe('subagent_outcome.v1');
  });

  it('reads each end, with the run’s own line when it left one', () => {
    for (const status of ['completed', 'blocked', 'failed', 'incomplete'] as const) {
      expect(subagentOutcomeOf(data({ status, ended_at: '2026-09-05T17:00:00Z' }))).toEqual({
        status,
        resultLine: null,
      });
    }
    expect(
      subagentOutcomeOf(data({ status: 'blocked', result_line: '  Which cohort did you mean? ' }))
    ).toEqual({ status: 'blocked', resultLine: 'Which cohort did you mean?' });
  });

  it('says nothing for no record, a malformed one, or a status that is not an end', () => {
    expect(subagentOutcomeOf(undefined)).toBeNull();
    expect(subagentOutcomeOf(null)).toBeNull();
    expect(subagentOutcomeOf({})).toBeNull();
    expect(subagentOutcomeOf(data('completed'))).toBeNull();
    expect(subagentOutcomeOf(data({ status: 42 }))).toBeNull();
    for (const status of [
      'running',
      'queued',
      'cancelling',
      'interrupted',
      'unknown',
      'Completed',
    ]) {
      expect(subagentOutcomeOf(data({ status })), status).toBeNull();
    }
    // A blank line is no line.
    expect(subagentOutcomeOf(data({ status: 'failed', result_line: '   ' }))).toEqual({
      status: 'failed',
      resultLine: null,
    });
  });
});

describe('runModelName', () => {
  it('names the model a run used, or nothing', () => {
    expect(runModelName({ model_config: { model_name: 'gpt-5.5' } })).toBe('gpt-5.5');
    expect(runModelName({ model_config: { model_name: '  ' } })).toBeNull();
    expect(runModelName({ model_config: null })).toBeNull();
    expect(runModelName({})).toBeNull();
  });
});
