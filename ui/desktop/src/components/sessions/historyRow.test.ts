import { describe, expect, it } from 'vitest';
import { folderName, historyRowTitle } from './historyRow';
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

describe('messageCount', () => {
  it('agrees in number', () => {
    expect(messageCount(1)).toBe('1 message');
    expect(messageCount(0)).toBe('0 messages');
    expect(messageCount(12)).toBe('12 messages');
  });
});
