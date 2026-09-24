import { describe, expect, it } from 'vitest';
import type { CrewMessage, ObservedRun } from '../crewApi';
import { newestTaskIn } from './newestTask';

const run = (run_id: string, channel_id = 'general'): ObservedRun => ({
  run_id,
  channel_id,
  session_id: `session-${run_id}`,
  status: 'running',
});

const post = (sequence: string, run_id?: string, channel_id = 'general'): CrewMessage => ({
  id: `message-${sequence}`,
  sequence,
  channel_id,
  actor_id: 'alice',
  ...(run_id ? { run_id } : {}),
  body: `message ${sequence}`,
  created_at: 1_700_000_000,
  restricted: false,
  source_channels: [channel_id],
  attachments: [],
});

describe('newestTaskIn', () => {
  it('finds nothing when the viewer has no task in the channel', () => {
    expect(newestTaskIn([], [], 'general')).toBeNull();
    expect(newestTaskIn([run('a', 'methods')], [post('1', 'a', 'methods')], 'general')).toBeNull();
  });

  it('dates a task by its first loaded message, whatever order the daemon lists it in', () => {
    const older = run('older');
    const newer = run('newer');
    const messages = [post('1', 'older'), post('2'), post('3', 'newer'), post('4', 'older')];
    expect(newestTaskIn([newer, older], messages, 'general')).toBe(newer);
    expect(newestTaskIn([older, newer], messages, 'general')).toBe(newer);
  });

  it("ignores other channels' tasks and messages", () => {
    const here = run('here');
    const elsewhere = run('elsewhere', 'methods');
    const messages = [post('1', 'here'), post('2', 'elsewhere', 'methods')];
    expect(newestTaskIn([here, elsewhere], messages, 'general')).toBe(here);
  });

  it('prefers a task the log shows over one with no loaded message', () => {
    const shown = run('shown');
    const unseen = run('unseen');
    expect(newestTaskIn([shown, unseen], [post('1', 'shown')], 'general')).toBe(shown);
  });

  it('falls back to the last listed task, the row the timeline draws last', () => {
    const first = run('first');
    const last = run('last');
    expect(newestTaskIn([first, last], [post('1')], 'general')).toBe(last);
  });
});
