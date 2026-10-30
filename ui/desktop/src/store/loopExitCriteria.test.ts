import { beforeEach, describe, expect, it, vi } from 'vitest';
import { setLoopExitCriteria, type ExitCriteria } from '../api';
import {
  adoptDraftExitCriteria,
  getDraftExitCriteria,
  setDraftExitCriteria,
} from './loopExitCriteria';
vi.mock('../api', () => ({ setLoopExitCriteria: vi.fn() }));
const criteria: ExitCriteria = {
  timing: 'post_response',
  prompt: 'On track?',
  provider: 'fixture',
  model: 'joey-1',
  decision: { type: 'boolean', interrupt_when: false },
};
beforeEach(() => {
  vi.mocked(setLoopExitCriteria)
    .mockReset()
    .mockResolvedValue({ data: { criteria } } as never);
});
describe('draft loop exit criteria', () => {
  it('scopes drafts by composer', () => {
    setDraftExitCriteria('draft:a', criteria);
    expect(getDraftExitCriteria('draft:a')).toEqual(criteria);
    expect(getDraftExitCriteria('draft:b')).toBeNull();
  });
  it('saves before clearing a successfully adopted draft', async () => {
    setDraftExitCriteria('draft:adopt', criteria);
    await adoptDraftExitCriteria('draft:adopt', 'chat-one', criteria);
    expect(setLoopExitCriteria).toHaveBeenCalledWith({
      path: { session_id: 'chat-one' },
      body: { criteria },
      throwOnError: true,
    });
    expect(getDraftExitCriteria('draft:adopt')).toBeNull();
  });
  it('keeps criteria and rejects adoption if saving fails', async () => {
    setDraftExitCriteria('draft:failed', criteria);
    vi.mocked(setLoopExitCriteria).mockRejectedValueOnce(new Error('refused'));
    await expect(adoptDraftExitCriteria('draft:failed', 'chat-one', criteria)).rejects.toThrow(
      'refused'
    );
    expect(getDraftExitCriteria('draft:failed')).toEqual(criteria);
  });
  it('keeps a newer edit while adoption awaits', async () => {
    setDraftExitCriteria('draft:race', criteria);
    const edited = { ...criteria, prompt: 'New criterion' };
    vi.mocked(setLoopExitCriteria).mockImplementationOnce(async () => {
      setDraftExitCriteria('draft:race', edited);
      return { data: { criteria } } as never;
    });
    await adoptDraftExitCriteria('draft:race', 'chat-one', criteria);
    expect(getDraftExitCriteria('draft:race')).toEqual(edited);
  });
  it('does not request configuration for disabled criteria', async () => {
    await adoptDraftExitCriteria('draft:off', 'chat-one', null);
    expect(setLoopExitCriteria).not.toHaveBeenCalled();
  });
  it('refuses malformed persisted drafts rather than treating them as disabled', () => {
    sessionStorage.setItem('biorouter.loopExitCriteria.v1:draft:corrupt', 'false');
    expect(() => getDraftExitCriteria('draft:corrupt')).toThrow('invalid');
  });
  it('preserves pre-send timing when adopting a new-chat draft', async () => {
    const preSend: ExitCriteria = { ...criteria, timing: 'pre_send' };
    setDraftExitCriteria('draft:pre-send', preSend);
    await adoptDraftExitCriteria('draft:pre-send', 'chat-one', preSend);
    expect(setLoopExitCriteria).toHaveBeenCalledWith(
      expect.objectContaining({ body: { criteria: preSend } })
    );
  });
});
