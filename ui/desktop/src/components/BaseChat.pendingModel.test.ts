import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../toasts', () => ({ toastError: vi.fn(), toastSuccess: vi.fn() }));

import { bindHeldChatModel, heldModelMayBeSent } from './BaseChat';
import type Model from './settings/models/modelInterface';
import type { ProviderDetails } from '../api';

const held: Model = { name: 'claude-fable-5-1', provider: 'claude_code', subtext: 'Claude Code' };

/** A provider entry shaped like `GET /config/providers` serves one. */
const entry = (name: string, tier?: 'private' | 'public') =>
  ({ name, metadata: { name, display_name: name, tier } }) as unknown as ProviderDetails;

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

// T3-SH-1: a public model held for an unsent chat was bound in the same click
// that sent the first message, so the non-private-model disclosure came up over
// the answer (DR-17 requirement 3 says before the first turn).
describe('heldModelMayBeSent', () => {
  const registry = vi.fn(async () => [
    entry('claude_code', 'public'),
    entry('versa_azure', 'private'),
  ]);

  it('holds the first message while a public held model is still to be disclosed', async () => {
    const acknowledged = vi.fn(async () => false);
    expect(await heldModelMayBeSent(held, registry, acknowledged)).toBe(false);
    expect(acknowledged).toHaveBeenCalledTimes(1);
  });

  it('lets it go once the disclosure was acknowledged', async () => {
    expect(await heldModelMayBeSent(held, registry, async () => true)).toBe(true);
  });

  it('sends at once on a private held model, without asking about the disclosure', async () => {
    const acknowledged = vi.fn(async () => false);
    const versa: Model = { name: 'gpt-5.5', provider: 'versa_azure' };
    expect(await heldModelMayBeSent(versa, registry, acknowledged)).toBe(true);
    expect(acknowledged).not.toHaveBeenCalled();
  });

  it('asks nothing when no model is held: the app-level gate covers the configured one', async () => {
    const acknowledged = vi.fn(async () => false);
    expect(await heldModelMayBeSent(null, registry, acknowledged)).toBe(true);
    expect(acknowledged).not.toHaveBeenCalled();
  });

  it('counts a provider it cannot classify as public', async () => {
    const unknown: Model = { name: 'm', provider: 'mystery' };
    expect(await heldModelMayBeSent(unknown, registry, async () => false)).toBe(false);
    const untiered = vi.fn(async () => [entry('claude_code')]);
    expect(await heldModelMayBeSent(held, untiered, async () => false)).toBe(false);
    const broken = vi.fn(async (): Promise<ProviderDetails[]> => {
      throw new Error('daemon gone');
    });
    expect(await heldModelMayBeSent(held, broken, async () => false)).toBe(false);
  });

  it('does not hold a message the dialog could never be shown for', async () => {
    // The daemon could not serve the disclosure, so no dialog can come up
    // (useDisclosure renders nothing without its copy). Refusing would leave
    // the message unsendable behind no visible reason.
    expect(await heldModelMayBeSent(held, registry, async () => null)).toBe(true);
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

  it('asks about the disclosure before the chat is created, let alone sent to', () => {
    const disclosed = submit.indexOf(
      'if (!(await heldModelMayBeSent(chosenModel, getProviders))) return false;'
    );
    const create = submit.indexOf('await createSession(');
    const send = submit.indexOf('navigateWithViewTransition(');
    expect(disclosed).toBeGreaterThan(-1);
    expect(disclosed).toBeLessThan(create);
    expect(disclosed).toBeLessThan(send);
  });

  it("keys this chat's disclosure gate on the held model until the chat has a session", () => {
    expect(source).toMatch(
      /<NonPrivateModelDisclosureGate\s+providerName=\{session\?\.provider_name \?\? \(sessionId \? null : pendingChatModel\?\.provider\)\}/
    );
  });

  it('holds the pick under the chat tab, so it outlives the trip to the provider catalog', () => {
    expect(source).toMatch(/useHeldChatModel\(terminalKey\)/);
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
    expect(source).toMatch(
      /sessionId \? null : \{ choose: setPendingChatModel, tabId: terminalKey \}/
    );
  });
});
