import { readFileSync } from 'node:fs';
import path from 'node:path';
import * as ts from 'typescript';
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
    const send = submit.indexOf('navigateToRoute(');
    expect(create).toBeGreaterThan(-1);
    expect(bind).toBeGreaterThan(create);
    expect(send).toBeGreaterThan(bind);
  });

  it('asks about the disclosure before the chat is created, let alone sent to', () => {
    const disclosed = submit.indexOf(
      'if (!(await heldModelMayBeSent(chosenModel, getProviders))) return false;'
    );
    const create = submit.indexOf('await createSession(');
    const send = submit.indexOf('navigateToRoute(');
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

/** Execute the current submit callback with its React collaborators injected. */
describe('the actual pre-session accepted destination callback', () => {
  const source = readFileSync(path.join(process.cwd(), 'src/components/BaseChat.tsx'), 'utf8');
  const tree = ts.createSourceFile(
    'BaseChat.tsx',
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX
  );
  let callback: ts.ArrowFunction | undefined;
  const visit = (node: ts.Node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === 'handleFormSubmit' &&
      node.initializer &&
      ts.isArrowFunction(node.initializer)
    ) {
      if (callback) throw new Error('Expected exactly one handleFormSubmit callback');
      callback = node.initializer;
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  if (!callback) throw new Error('Missing the actual handleFormSubmit callback');
  const actualCallback = callback.getText(tree);

  function harness() {
    const calls: string[] = [];
    let resolveBind!: (accepted: boolean) => void;
    const changeModel = vi.fn(() => {
      calls.push('bind');
      return new Promise<boolean>((resolve) => {
        resolveBind = resolve;
      });
    });
    const onAcceptedSession = vi.fn((_sessionId: string) => {
      calls.push('accepted');
    });
    const createSession = vi.fn(async () => {
      calls.push('create');
      return { id: 'actual-created-session' };
    });
    const navigateToRoute = vi.fn(() => {
      calls.push('navigate');
    });
    const firstSend = { begin: vi.fn(), created: vi.fn(), end: vi.fn() };
    const setIsCreatingSession = vi.fn();
    const handleCreateSessionError = vi.fn(() => false);
    const bindings = {
      session: null,
      sessionId: null,
      isCreatingSession: false,
      reasoningDraftKey: 'tab:origin',
      draftReasoningScope: () => 'draft:origin',
      getReasoningEffort: () => undefined,
      pendingChatModel: held,
      confirmNewChatModel: vi.fn(async () => true),
      heldModelMayBeSent: vi.fn(async () => true),
      getProviders: vi.fn(async () => []),
      stageComposerMotion: vi.fn(),
      setIsCreatingSession,
      firstSend,
      createSession,
      // #484's new chat start: no decision models drafted, nothing to confirm.
      draftDecisions: undefined,
      draftDecisionStartOptions: () => ({}),
      currentProvider: null,
      currentModel: null,
      publicDecisionConfirm: { confirm: vi.fn(async () => true) },
      pendingWorkingDir: undefined,
      resolveNewSessionWorkingDir: () => '/work',
      getInitialWorkingDir: () => '/work',
      extensionsList: [],
      bindHeldChatModel,
      changeModel,
      adoptDraftReasoningEffort: vi.fn(),
      navigateToRoute,
      navigate: vi.fn(),
      terminalKey: 'origin-tab',
      handleCreateSessionError,
    };
    const code = ts.transpileModule(`return (${actualCallback});`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
    }).outputText;
    const submit = new Function(...Object.keys(bindings), code)(...Object.values(bindings)) as (
      event: unknown
    ) => Promise<boolean>;
    const event = new CustomEvent('submit', {
      detail: { value: 'Check the EPAS1 gene', attachments: [], onAcceptedSession },
    });
    return {
      calls,
      changeModel,
      createSession,
      onAcceptedSession,
      navigateToRoute,
      firstSend,
      setIsCreatingSession,
      handleCreateSessionError,
      submit,
      event,
      resolveBind: (accepted: boolean) => resolveBind(accepted),
    };
  }

  const settle = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  };

  it('reports the created destination only after its held model bind is accepted and before navigation', async () => {
    const h = harness();
    const submitted = h.submit(h.event);
    await settle();
    expect(h.createSession).toHaveBeenCalledTimes(1);
    expect(h.changeModel).toHaveBeenCalledWith('actual-created-session', held, { quiet: true });
    expect(h.onAcceptedSession).not.toHaveBeenCalled();
    expect(h.navigateToRoute).not.toHaveBeenCalled();
    h.resolveBind(true);
    expect(await submitted).toBe(true);
    expect(h.onAcceptedSession).toHaveBeenCalledExactlyOnceWith('actual-created-session');
    expect(h.calls).toEqual(['create', 'bind', 'accepted', 'navigate']);
    const navigation = h.navigateToRoute.mock.calls[0] as unknown as [
      unknown,
      string,
      Record<string, unknown>,
    ];
    expect(navigation[1]).toBe('/pair?resumeSessionId=actual-created-session');
    expect(navigation[2]).toEqual({
      resumeSessionId: 'actual-created-session',
      initialMessage: 'Check the EPAS1 gene',
      initialAttachments: [],
      originTabId: 'origin-tab',
    });
  });

  it('reports no accepted destination when the model bind is refused', async () => {
    const h = harness();
    const submitted = h.submit(h.event);
    await settle();
    h.resolveBind(false);
    expect(await submitted).toBe(false);
    expect(h.onAcceptedSession).not.toHaveBeenCalled();
    expect(h.navigateToRoute).not.toHaveBeenCalled();
    expect(h.firstSend.end).toHaveBeenCalledTimes(1);
    expect(h.setIsCreatingSession).toHaveBeenLastCalledWith(false);
  });

  it('reports no accepted destination if session creation fails', async () => {
    const h = harness();
    h.createSession.mockRejectedValueOnce(new Error('session creation failed'));
    expect(await h.submit(h.event)).toBe(false);
    expect(h.changeModel).not.toHaveBeenCalled();
    expect(h.onAcceptedSession).not.toHaveBeenCalled();
    expect(h.navigateToRoute).not.toHaveBeenCalled();
    expect(h.handleCreateSessionError).toHaveBeenCalledTimes(1);
  });
});
