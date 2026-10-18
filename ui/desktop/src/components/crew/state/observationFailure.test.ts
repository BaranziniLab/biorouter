import { describe, expect, it } from 'vitest';
import { crewObservationCopy } from './copy';
import {
  ANSWERED_OBSERVATION_CODES,
  isFinalObservationEnd,
  mayBeConnectionLoss,
  observationFrameText,
  WORKSPACE_DELETED_CODE,
} from './observationFailure';

describe('an observation that ended workspace_deleted (OW-5)', () => {
  const names = { workspace: 'lab', channel: '#general' };

  it('is the broker’s answer, never a dropped connection', () => {
    expect(WORKSPACE_DELETED_CODE).toBe('workspace_deleted');
    expect(ANSWERED_OBSERVATION_CODES).toContain(WORKSPACE_DELETED_CODE);
    expect(mayBeConnectionLoss(WORKSPACE_DELETED_CODE)).toBe(false);
  });

  it('is final whether or not this session saw the connection verified', () => {
    expect(isFinalObservationEnd(WORKSPACE_DELETED_CODE, false)).toBe(true);
    expect(isFinalObservationEnd(WORKSPACE_DELETED_CODE, true)).toBe(true);
  });

  it('says who deleted what, in the bar’s words', () => {
    expect(observationFrameText(WORKSPACE_DELETED_CODE, names)).toBe(
      crewObservationCopy.workspaceDeleted('lab')
    );
    expect(crewObservationCopy.workspaceDeleted('lab')).toBe('The host deleted lab.');
  });
});
