import { describe, expect, it, vi } from 'vitest';
import { TOOL_COUNT_COPY, TOOLS_MAX_SUGGESTED, toolCountWarning } from './alerts/toolCountWarning';

describe('toolCountWarning', () => {
  it('speaks only past the suggested count, and reviews this chat’s extensions', () => {
    const open = vi.fn();
    window.addEventListener('current-chat-extensions:open', open);
    try {
      expect(toolCountWarning(TOOLS_MAX_SUGGESTED, 'chat-1')).toBeNull();
      const warning = toolCountWarning(61, 'chat-1');
      expect(warning?.count).toBe(61);
      warning?.review();
      expect(open).toHaveBeenCalledWith(
        expect.objectContaining({ detail: { sessionId: 'chat-1' } })
      );
    } finally {
      window.removeEventListener('current-chat-extensions:open', open);
    }
  });

  it('does not offer chat management without an active chat or a count', () => {
    expect(toolCountWarning(61, null)).toBeNull();
    expect(toolCountWarning(null, 'chat-1')).toBeNull();
  });

  it('is one line with its reason in a two-sentence tip that no longer says defaults only reach new chats', () => {
    expect(TOOL_COUNT_COPY.line(61)).toBe('This chat can call 61 tools; 60 or fewer works best.');
    expect(TOOL_COUNT_COPY.help.split(/(?<=\.)\s/)).toHaveLength(2);
    // Per-chat switches for built-in capabilities exist (2026-11-13), so the old "Capability
    // defaults apply to a new chat" would now be wrong.
    expect(TOOL_COUNT_COPY.help).not.toMatch(/new chat/i);
    expect(TOOL_COUNT_COPY.help).toMatch(/built-in capabilities/);
  });
});
