import React from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

/**
 * V2-R8-3. A new chat's first send must not withdraw what the composer states
 * and then state it again.
 *
 * Measured in the running app at 1440: 81 ms after Enter in a new chat the
 * composer read "Tools" (no number) and "gpt-5.5" (no padlock); at 128 ms it
 * read "Tools 5" and the padlock again. The cause is a REMOUNT, not the null to
 * id step the chips already carried: `BaseChat` draws the composer centred
 * under the greeting while the chat is empty and in the bar under the
 * transcript once it is not (`renderChatInput()` in both branches), and the
 * send leaves the empty state in the commit that starts creating the chat. So
 * a second `ChatInput` mounts, and every state of the first is gone.
 *
 * `ChatPane` below has `BaseChat`'s shape at that seam (a ternary holding the
 * composer in its empty branch, and a bar after it holding the composer
 * otherwise) and nothing else. The REAL composer, Tools chip and model chip
 * run; the three lists are stand-ins that answer per mount, a moment after
 * they are asked, as the real ones do.
 */

const lists = vi.hoisted(() => ({ extensions: 2, skills: 2, knowledge: 1, answerAfterMs: 15 }));

/** True once this mount's list has answered for `sessionId`. */
function useAnsweredFor(sessionId: string | null): boolean {
  const [answeredFor, setAnsweredFor] = React.useState<{ chat: string | null } | null>(null);
  React.useEffect(() => {
    const timer = setTimeout(() => setAnsweredFor({ chat: sessionId }), lists.answerAfterMs);
    return () => clearTimeout(timer);
  }, [sessionId]);
  return answeredFor !== null && answeredFor.chat === sessionId;
}

vi.mock('./bottom_menu/BottomMenuExtensionSelection', () => ({
  useExtensionPicker: (sessionId: string | null) => ({
    activeCount: lists.extensions,
    answered: useAnsweredFor(sessionId),
    setQuery: () => {},
  }),
  ExtensionsPanel: () => null,
}));
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({
  useSkillPicker: (sessionId: string | null) => ({
    activeCount: lists.skills,
    answered: useAnsweredFor(sessionId),
    setQuery: () => {},
  }),
  SkillsPanel: () => null,
}));
vi.mock('./bottom_menu/BottomMenuKnowledgeSelection', () => ({
  useKnowledgePicker: () => ({
    visibleCount: lists.knowledge,
    answered: useAnsweredFor(null),
    setQuery: () => {},
  }),
  KnowledgePanel: () => null,
}));

// The app's catalog as `ConfigContext` holds it once anything has read it:
// `getProviders(false)` answers from this very list. One object, as the
// context's value is: a new function per render would re-run every effect that
// depends on it.
const config = vi.hoisted(() => {
  const VERSA = {
    name: 'versa_azure',
    is_configured: true,
    provider_type: 'Builtin',
    metadata: {
      name: 'versa_azure',
      display_name: 'Versa API Azure',
      tier: 'private',
      runs_locally: false,
    },
    affiliation: null,
    resolved_tier: 'private',
  };
  const providersList = [VERSA];
  return {
    getProviders: async () => providersList,
    read: async () => '',
    providersList,
  };
});
vi.mock('./ConfigContext', () => ({
  useConfig: () => config,
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: async () => ({ model: 'gpt-5.5', provider: 'versa_azure' }),
    currentModel: 'gpt-5.5',
    currentProvider: 'versa_azure',
    currentModelSupportsVision: true,
    currentModelSupportedInputMimeTypes: null,
    getCurrentModelAndProviderForDisplay: async () => ({
      model: 'gpt-5.5',
      provider: 'Versa API Azure',
    }),
    getCurrentModelDisplayName: async () => 'gpt-5.5',
    getCurrentProviderDisplayName: async () => 'Versa API Azure',
  }),
}));
vi.mock('./settings/models/subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: () => null,
}));
vi.mock('./settings/models/subcomponents/LeadWorkerSettings', () => ({
  LeadWorkerSettings: () => null,
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
// The footer as the folder it names, and a way to pick one.
vi.mock('./bottom_menu/ComposerFooter', () => ({
  ComposerFooter: ({
    workingDir,
    onWorkingDirChange,
  }: {
    workingDir: string | null;
    onWorkingDirChange: (dir: string) => void;
  }) => (
    <div>
      <span data-testid="footer-folder">{workingDir ?? ''}</span>
      <button type="button" onClick={() => onWorkingDirChange(CHOSEN_DIR)}>
        Pick folder
      </button>
    </div>
  ),
}));
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  // The new chat's row: created in the folder picked before the send.
  getSession: vi.fn(async ({ path }: { path: { session_id: string } }) => ({
    data: {
      id: path.session_id,
      working_dir: path.session_id === OTHER_CHAT ? OTHER_DIR : CHOSEN_DIR,
      privacy_tier: 'public',
    },
  })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
  getPrivacyDisclosure: vi.fn(async () => ({
    data: { title_template: '{provider}', long: 'LONG', short: 'SHORT', acknowledged: true },
  })),
  ackPrivacyDisclosure: vi.fn(async () => ({ data: {} })),
}));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../toasts', () => ({
  toastWarning: vi.fn(),
  toastError: vi.fn(),
  toastInfo: vi.fn(),
  toastSuccess: vi.fn(),
  toastLoading: vi.fn(),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { composerHandOffSizeForTests } from './composer/composerHandOff';
import { __resetDisclosureStoreForTests } from './privacy/disclosureCopy';

const DEFAULT_DIR = '/default/workdir';
const CHOSEN_DIR = '/Users/wgu/Desktop/data';
const TAB = 'tab-1';
const NEW_CHAT = 'chat-new';
const OTHER_CHAT = 'chat-b';
const OTHER_DIR = '/Users/wgu/Projects/b';

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  __resetDisclosureStoreForTests();
  Object.assign(window, {
    appConfig: {
      get: (key: string) => (key === 'BIOROUTER_WORKING_DIR' ? DEFAULT_DIR : undefined),
    },
    electron: {
      directoryChooser: vi.fn(async () => ({ canceled: true, filePaths: [] })),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

interface PaneProps {
  /** `BaseChat`'s `isCleanConversation`: false from the commit that starts creating the chat. */
  clean: boolean;
  sessionId: string | null;
  /** `BaseChat`'s tab-scoped `reasoningDraftKey`, the same before and after the chat exists. */
  composerKey?: string;
}

/** `BaseChat` at the seam that matters: one composer, drawn in one of two places. */
function ChatPane({ clean, sessionId, composerKey = TAB }: PaneProps) {
  const input = (
    <ChatInput
      sessionId={sessionId}
      handleSubmit={vi.fn()}
      chatState={ChatState.Idle}
      onStop={() => undefined}
      draftKey={sessionId ? `${composerKey}:${sessionId}` : composerKey}
      reasoningDraftKey={composerKey}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      sessionCosts={undefined}
      toolCount={0}
    />
  );
  return (
    <div className="pane">
      <div className="column">
        {clean ? (
          <div className="clean-conversation">
            <div className="clean-conversation-content">
              <p>Greeting</p>
              {input}
            </div>
          </div>
        ) : (
          <div className="transcript" />
        )}
      </div>
      {!clean && <div className="composer-bar">{input}</div>}
    </div>
  );
}

/** What the composer states in one frame: the Tools chip, its name, the padlock, the folder. */
function frame(): string {
  const tools = screen.queryByTestId('composer-tools-chip');
  const model = screen.queryByTestId('model-chip');
  const lock = model?.querySelector('[data-testid="privacy-badge"]') ? '+lock' : 'no lock';
  const folder = screen.queryByTestId('footer-folder')?.textContent ?? '-';
  return `${tools?.textContent ?? '-'} | ${tools?.getAttribute('aria-label') ?? '-'} | ${lock} | ${folder}`;
}

/** Every distinct committed state from now on (the observer reports per commit). */
function recordFrames() {
  const frames = [frame()];
  const observer = new MutationObserver(() => {
    const next = frame();
    if (frames[frames.length - 1] !== next) frames.push(next);
  });
  observer.observe(document.body, {
    subtree: true,
    childList: true,
    characterData: true,
    attributes: true,
  });
  return { frames, stop: () => observer.disconnect() };
}

const settle = (ms: number) =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });

const SETTLED = `Tools5 | Tools: 2 extensions, 2 skills, 1 knowledge base | +lock | ${CHOSEN_DIR}`;

/** A new tab, answered, with a folder picked before the first message. */
async function openNewChatWithPickedFolder() {
  const view = render(<ChatPane clean sessionId={null} />);
  fireEvent.click(screen.getByRole('button', { name: 'Pick folder' }));
  await waitFor(() => expect(frame()).toBe(SETTLED));
  return view;
}

describe('a new chat’s first send keeps what the composer states (V2-R8-3)', () => {
  it('keeps "Tools 5", its name, the padlock and the picked folder in every commit', async () => {
    const view = await openNewChatWithPickedFolder();
    const fieldBefore = screen.getByTestId('chat-input');
    const record = recordFrames();

    // Enter: the chat starts being created, and the composer leaves the centre.
    view.rerender(<ChatPane clean={false} sessionId={null} />);
    // The instrument: this IS a second composer, as in `BaseChat`.
    expect(screen.getByTestId('chat-input')).not.toBe(fieldBefore);
    await settle(5);
    // The chat exists.
    view.rerender(<ChatPane clean={false} sessionId={NEW_CHAT} />);
    // Its lists answer, and its row (the folder) lands.
    await settle(lists.answerAfterMs * 3);
    record.stop();

    expect(record.frames).toEqual([SETTLED]);
  });

  it('hands over nothing to a composer that mounts later, or for another chat', async () => {
    const view = await openNewChatWithPickedFolder();
    view.unmount();
    // Every entry left with its composer: nothing is kept for a later visit.
    expect(composerHandOffSizeForTests()).toBe(0);

    // The same tab, opened again: it asks for itself.
    const again = render(<ChatPane clean sessionId={null} />);
    expect(screen.getByTestId('composer-tools-chip')).toHaveTextContent(/^Tools$/);
    expect(screen.getByTestId('composer-tools-chip')).toHaveAccessibleName('Tools');
    expect(screen.getByTestId('footer-folder')).toHaveTextContent(DEFAULT_DIR);
    again.unmount();

    // A composer settled on one chat, replaced by one bound to ANOTHER chat:
    // nothing it stated is carried over.
    const chatA = render(<ChatPane clean={false} sessionId={NEW_CHAT} />);
    await waitFor(() => expect(frame()).toBe(SETTLED));
    chatA.rerender(<ChatPane clean sessionId={OTHER_CHAT} />);
    expect(screen.getByTestId('composer-tools-chip')).toHaveTextContent(/^Tools$/);
    expect(screen.getByTestId('composer-tools-chip')).toHaveAccessibleName('Tools');
    expect(screen.getByTestId('footer-folder')).toHaveTextContent(/^$/);
    // Then its own figures and folder, once they are read.
    await waitFor(() =>
      expect(frame()).toBe(
        `Tools5 | Tools: 2 extensions, 2 skills, 1 knowledge base | +lock | ${OTHER_DIR}`
      )
    );
    chatA.unmount();
    expect(composerHandOffSizeForTests()).toBe(0);
  });
});
