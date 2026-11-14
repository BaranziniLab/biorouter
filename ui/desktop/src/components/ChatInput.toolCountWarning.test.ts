import { describe, expect, it, vi } from 'vitest';
import { toolCountWarning } from './alerts/toolCountWarning';

describe('toolCountWarning', () => {
  it('accurately scopes the mixed count and reviews this chat extensions', () => {
    const open = vi.fn();
    window.addEventListener('current-chat-extensions:open', open);

    const warning = toolCountWarning(61, 'chat-1');
    // The count is the one visible line; the line that restated the action button is gone, and
    // why it matters is the alert's info icon (owner, 2026-11-13; design.md 9.16).
    expect(warning?.message).toBe(
      'This chat can call 61 tools, including built-in capabilities (recommended: 60 or fewer).'
    );
    expect(warning?.help).toBe(
      'Too many tools can degrade performance. Capability defaults apply to a new chat.'
    );
    expect(warning?.message).not.toContain('Review currently attached extensions');
    expect(warning?.help).not.toContain('Review currently attached extensions');
    expect(warning?.action?.text).toBe('Review extensions');
    warning?.action?.onClick();

    expect(open).toHaveBeenCalledWith(expect.objectContaining({ detail: { sessionId: 'chat-1' } }));
    window.removeEventListener('current-chat-extensions:open', open);
  });

  it('does not offer chat management without an active chat', () => {
    expect(toolCountWarning(61, null)).toBeNull();
  });
});
