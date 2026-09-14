import { beforeEach, describe, expect, it } from 'vitest';
import {
  SESSION_TYPE_MEMORY_LIMIT,
  clearSessionTypeMemory,
  forgetSessionTypesExcept,
  recallSessionTypes,
  rememberSessionType,
  rememberedSessionTypeCount,
} from './sessionTypeMemory';

/**
 * The module-scope memory of each tab's session type. The shell's own suite
 * (`ChatGroupsShell.subagentKind.test.tsx`) pins what it is FOR — the Bot glyph
 * on the first render of a remount. This pins that it cannot grow without
 * bound.
 */
describe('sessionTypeMemory', () => {
  beforeEach(() => clearSessionTypeMemory());

  it('recalls only the chats asked about, with the latest answer', () => {
    rememberSessionType('a', 'sub_agent');
    rememberSessionType('b', 'user');
    rememberSessionType('b', 'sub_agent');
    expect(recallSessionTypes(['a', 'b', 'missing'])).toEqual({ a: 'sub_agent', b: 'sub_agent' });
  });

  it('forgets every chat no tab holds', () => {
    rememberSessionType('a', 'sub_agent');
    rememberSessionType('b', 'sub_agent');
    forgetSessionTypesExcept(new Set(['b']));
    expect(recallSessionTypes(['a', 'b'])).toEqual({ b: 'sub_agent' });
    expect(rememberedSessionTypeCount()).toBe(1);
  });

  it('caps itself, dropping the oldest answer first', () => {
    for (let n = 0; n < SESSION_TYPE_MEMORY_LIMIT + 50; n++) {
      rememberSessionType(`s${n}`, 'sub_agent');
    }
    expect(rememberedSessionTypeCount()).toBe(SESSION_TYPE_MEMORY_LIMIT);
    expect(recallSessionTypes(['s0', 's49'])).toEqual({});
    expect(recallSessionTypes(['s50', `s${SESSION_TYPE_MEMORY_LIMIT + 49}`])).toEqual({
      s50: 'sub_agent',
      [`s${SESSION_TYPE_MEMORY_LIMIT + 49}`]: 'sub_agent',
    });
  });

  it('counts a repeated answer as the newest, so a chat still open is not evicted first', () => {
    rememberSessionType('open', 'sub_agent');
    for (let n = 0; n < SESSION_TYPE_MEMORY_LIMIT - 1; n++) rememberSessionType(`s${n}`, 'user');
    rememberSessionType('open', 'sub_agent');
    rememberSessionType('one-more', 'user');
    expect(recallSessionTypes(['open'])).toEqual({ open: 'sub_agent' });
    expect(recallSessionTypes(['s0'])).toEqual({});
  });
});
