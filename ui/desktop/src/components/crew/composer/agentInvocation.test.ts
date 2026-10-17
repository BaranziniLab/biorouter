import { describe, expect, it } from 'vitest';
import type { Snapshot } from '../crewApi';
import {
  agentNameTaken,
  agentStartState,
  parseAgentInvocation,
  suggestionQuery,
  TASK_PROMPT_MAX_BYTES,
  taskBytes,
} from './agentInvocation';

const free = { agentNameTaken: false };
const taken = { agentNameTaken: true };

describe('parseAgentInvocation (OW-10)', () => {
  it.each([
    ['@agent do x', '@agent', 'do x'],
    ['  @Agent\n do x', '@agent', 'do x'],
    ['/agent do x', '/agent', 'do x'],
    ['/task summarize counts.csv', '/task', 'summarize counts.csv'],
    ['/TASK  spaced out  ', '/task', 'spaced out'],
    ['/agent', '/agent', ''],
    ['@agent\t', '@agent', ''],
  ])('reads %j as %s with the task %j', (body, trigger, prompt) => {
    expect(parseAgentInvocation(body, free)).toEqual({ trigger, prompt });
  });

  it.each([
    ['@agentx do'],
    ['@agent.'],
    ['@agent, please'],
    ['hi @agent do'],
    ['/agent/foo'],
    ['/pool1/data/x'],
    ['/tasks list'],
    ['agent do x'],
    ['@a​gent x'],
    ['＠agent x'],
    [''],
  ])('leaves %j a message', (body) => {
    expect(parseAgentInvocation(body, free)).toBeNull();
  });

  it('lets a person called agent keep @agent as their mention, and /agent still works', () => {
    expect(parseAgentInvocation('@agent x', taken)).toBeNull();
    expect(parseAgentInvocation('/agent x', taken)).toEqual({ trigger: '/agent', prompt: 'x' });
    expect(parseAgentInvocation('/task x', taken)).toEqual({ trigger: '/task', prompt: 'x' });
  });
});

describe('agentNameTaken', () => {
  const snapshot = (principals: { username: string }[], former: { username: string }[] = []) =>
    ({ principals, former_principals: former }) as unknown as Snapshot;

  it('finds a member or a former member whose username’s key is agent', () => {
    expect(agentNameTaken(snapshot([{ username: 'alice' }]))).toBe(false);
    expect(agentNameTaken(snapshot([{ username: 'agent' }]))).toBe(true);
    expect(agentNameTaken(snapshot([{ username: 'Agent' }]))).toBe(true);
    expect(agentNameTaken(snapshot([], [{ username: 'agent' }]))).toBe(true);
    expect(agentNameTaken(snapshot([{ username: 'agent2' }]))).toBe(false);
    expect(agentNameTaken(null)).toBe(false);
  });
});

describe('suggestionQuery', () => {
  const at = (body: string) => suggestionQuery(body, body.length);

  it('asks for the agent on a leading @ or / word', () => {
    expect(at('@')).toEqual({ kind: 'leading', sigil: '@', text: '', start: 0, end: 1 });
    expect(at('  @ag')).toEqual({ kind: 'leading', sigil: '@', text: 'ag', start: 2, end: 5 });
    expect(at('/t')).toEqual({ kind: 'leading', sigil: '/', text: 't', start: 0, end: 2 });
  });

  it('asks for people on an @ word after a space', () => {
    expect(at('thanks @bo')).toEqual({ kind: 'person', text: 'bo', start: 7, end: 10 });
    expect(at('hi @')).toEqual({ kind: 'person', text: '', start: 3, end: 4 });
  });

  it('asks for nothing on a path, an address, or once the word is done', () => {
    expect(at('/pool1')).toBeNull();
    expect(at('/agent/')).toBeNull();
    expect(at('mail@host')).toBeNull();
    expect(at('@agent ')).toBeNull();
    expect(at('hello')).toBeNull();
  });

  it('follows the caret, and only at the end of a word', () => {
    expect(suggestionQuery('@ag rest', 3)).toEqual({
      kind: 'leading',
      sigil: '@',
      text: 'ag',
      start: 0,
      end: 3,
    });
    expect(suggestionQuery('@agent', 3)).toBeNull();
  });
});

describe('agentStartState: what Enter does', () => {
  const ready = {
    invocation: { trigger: '@agent' as const, prompt: 'summarize counts.csv' },
    hasFiles: false,
    unknownOutcome: false,
    modelsKnown: true,
    hasModel: true,
    blockText: null,
  };

  it('starts at once when nothing stands in the way', () => {
    expect(agentStartState(ready)).toEqual({ kind: 'ready' });
  });

  it('opens Ask my agent for a trigger alone, an unknown outcome, or models still loading', () => {
    expect(agentStartState({ ...ready, invocation: { trigger: '/agent', prompt: '' } }).kind).toBe(
      'empty'
    );
    expect(agentStartState({ ...ready, unknownOutcome: true }).kind).toBe('unknown-outcome');
    expect(agentStartState({ ...ready, modelsKnown: false }).kind).toBe('loading');
  });

  it('opens the picker for no model or a refused one', () => {
    expect(agentStartState({ ...ready, hasModel: false }).kind).toBe('no-model');
    expect(agentStartState({ ...ready, blockText: 'lab is Private.' })).toEqual({
      kind: 'blocked',
      text: 'lab is Private.',
    });
  });

  it('starts nothing with files, or past the daemon’s 32 KB, counted in UTF-8 bytes', () => {
    expect(agentStartState({ ...ready, hasFiles: true }).kind).toBe('files');
    const atLimit = 'x'.repeat(TASK_PROMPT_MAX_BYTES);
    expect(
      agentStartState({ ...ready, invocation: { trigger: '@agent', prompt: atLimit } }).kind
    ).toBe('ready');
    // 16,385 two-byte letters are 32,770 bytes, though fewer characters than the limit.
    const wide = 'é'.repeat(TASK_PROMPT_MAX_BYTES / 2 + 1);
    expect(taskBytes(wide)).toBe(TASK_PROMPT_MAX_BYTES + 2);
    expect(
      agentStartState({ ...ready, invocation: { trigger: '@agent', prompt: wide } }).kind
    ).toBe('too-long');
  });
});
