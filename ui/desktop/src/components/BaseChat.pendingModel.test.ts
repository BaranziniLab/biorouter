import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../toasts', () => ({ toastError: vi.fn(), toastSuccess: vi.fn() }));

import { bindHeldChatModel } from './BaseChat';
import type Model from './settings/models/modelInterface';

const held: Model = { name: 'claude-fable-5-1', provider: 'claude_code', subtext: 'Claude Code' };

// W2-PRV-6: a model picked in a chat that had not been sent yet rewrote the model
// every new chat starts on. The pick is now held and bound to the chat when its
// first message creates it.
describe('bindHeldChatModel', () => {
  it('leaves a chat with no held model on what /agent/start bound', async () => {
    const changeModel = vi.fn();
    expect(await bindHeldChatModel('s-1', null, changeModel)).toBe(true);
    expect(changeModel).not.toHaveBeenCalled();
  });

  it('binds the held model to that chat only, quietly', async () => {
    const changeModel = vi.fn().mockResolvedValue(true);
    expect(await bindHeldChatModel('s-1', held, changeModel)).toBe(true);
    expect(changeModel).toHaveBeenCalledWith('s-1', held, { quiet: true });
  });

  it('refuses to proceed when the bind was refused', async () => {
    const changeModel = vi.fn().mockResolvedValue(false);
    expect(await bindHeldChatModel('s-1', held, changeModel)).toBe(false);
  });
});

/** BaseChat cannot be mounted in jsdom; its pre-session submit is read at the source. */
describe('the pre-session submit', () => {
  const source = readFileSync(path.join(process.cwd(), 'src/components/BaseChat.tsx'), 'utf8');
  const submit = source.slice(
    source.indexOf('const handleFormSubmit = async'),
    source.indexOf('if (workflow && textValue.trim())')
  );

  it('binds the held model after the chat exists and before the message is sent', () => {
    const create = submit.indexOf('await createSession(');
    const bind = submit.indexOf('await bindHeldChatModel(newSession.id, chosenModel, changeModel)');
    const send = submit.indexOf('navigateWithViewTransition(');
    expect(create).toBeGreaterThan(-1);
    expect(bind).toBeGreaterThan(create);
    expect(send).toBeGreaterThan(bind);
  });

  it('does not re-check the app-wide selection when the chat has a model of its own', () => {
    expect(submit).toMatch(
      /if \(!chosenModel && !\(await confirmNewChatModel\(\)\)\) return false;/
    );
  });

  it('offers the chat scope only while the chat has no session', () => {
    expect(source).toMatch(
      /<PendingChatModelContext\.Provider value=\{pendingChatModelScope\}>\s*<ChatInput/
    );
    expect(source).toMatch(/sessionId \? null : \{ choose: setPendingChatModel \}/);
  });
});
