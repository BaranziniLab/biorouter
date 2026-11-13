import { describe, expect, it } from 'vitest';
import {
  Check,
  CircleDotDashed,
  CircleHelp,
  CircleIcon,
  MoreHorizontal,
  PauseCircle,
  StopCircle,
  X,
} from '../icons/app-icons';
import { SUBAGENT_RUN_STATUSES } from '../../utils/sessionSubagents';
import { SUBAGENT_STATUS_GLYPHS, SUBAGENT_STATUS_WORDS } from './subagentStatus';

describe('subagent status map', () => {
  it('gives every status a glyph and a word', () => {
    for (const status of SUBAGENT_RUN_STATUSES) {
      expect(SUBAGENT_STATUS_GLYPHS[status], status).toBeTruthy();
      expect(SUBAGENT_STATUS_WORDS[status]?.trim(), status).toBeTruthy();
    }
    expect(Object.keys(SUBAGENT_STATUS_GLYPHS).sort()).toEqual([...SUBAGENT_RUN_STATUSES].sort());
    expect(Object.keys(SUBAGENT_STATUS_WORDS).sort()).toEqual([...SUBAGENT_RUN_STATUSES].sort());
  });

  it('pins the table', () => {
    expect(SUBAGENT_STATUS_GLYPHS).toEqual({
      queued: CircleIcon,
      running: CircleDotDashed,
      cancelling: StopCircle,
      completed: Check,
      blocked: CircleHelp,
      incomplete: StopCircle,
      failed: X,
      interrupted: PauseCircle,
      unknown: MoreHorizontal,
    });
    expect(SUBAGENT_STATUS_WORDS).toEqual({
      queued: 'Waiting to start',
      running: 'Running',
      cancelling: 'Stopping',
      completed: 'Completed',
      blocked: 'Needs an answer',
      incomplete: 'Stopped',
      failed: 'Failed',
      interrupted: 'Interrupted',
      unknown: 'Status unknown',
    });
  });

  it('never draws a stopped run with the running ring', () => {
    expect(SUBAGENT_STATUS_GLYPHS.incomplete).not.toBe(SUBAGENT_STATUS_GLYPHS.running);
    for (const status of SUBAGENT_RUN_STATUSES) {
      if (status !== 'running') {
        expect(SUBAGENT_STATUS_GLYPHS[status], status).not.toBe(CircleDotDashed);
      }
    }
  });
});
