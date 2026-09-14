import React from 'react';
import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

/**
 * Two composers on one page — a split — and each Send button must submit ITS
 * OWN composer.
 *
 * Every composer used to render `<form id="bior-chat-form">`, and Send sits
 * outside that form, bound to it with `form="bior-chat-form"`. A document
 * resolves a duplicated id to the FIRST element carrying it, so every Send in
 * the window submitted the left pane's form. Measured on 1.90.4 in a real
 * two-pane split: the right-hand Send's form owner was the left form (x=329, the
 * right pane's own form sat at x=906), and clicking it sent the left pane's
 * draft while the right pane's message stayed in its box. Enter was unaffected,
 * because the key handler calls the composer's own submit rather than going
 * through the button's form owner.
 *
 * These mount two REAL composers into one React root, the way the split does,
 * and click the button — `fireEvent.submit(form)` would bypass exactly the
 * binding under test. jsdom resolves a button's `form` attribute through the
 * document's id lookup, the same rule the browser applies, so the collision is
 * reproducible here.
 */

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: 'gpt-5.5', provider: 'versa_azure' })),
    currentModel: 'gpt-5.5',
    currentProvider: 'versa_azure',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./settings/models/bottom_bar/ModelsBottomBar', () => ({ default: () => null }));
vi.mock('./bottom_menu/BottomMenuExtensionSelection', () => ({
  BottomMenuExtensionSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({ BottomMenuSkillSelection: () => null }));
vi.mock('./bottom_menu/BottomMenuKnowledgeSelection', () => ({
  BottomMenuKnowledgeSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuReasoningEffort', () => ({
  BottomMenuReasoningEffort: () => null,
}));
vi.mock('./bottom_menu/CostTracker', () => ({ CostTracker: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
// What the queue case needs from the real drag-and-drop list is only WHICH
// messages each pane holds, so it stands in as a plain list.
vi.mock('./MessageQueue', () => ({
  default: ({ queuedMessages }: { queuedMessages: Array<{ id: string; content: string }> }) => (
    <ul data-testid="queue">
      {queuedMessages.map((msg) => (
        <li key={msg.id} data-testid="queued-content">
          {msg.content}
        </li>
      ))}
    </ul>
  ),
  canSteerMessage: () => false,
}));
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import type { DroppedFile } from '../hooks/useFileDrop';

type SubmitFn = (e: React.FormEvent) => void | Promise<boolean | void>;
type SubmitMock = Mock<SubmitFn>;

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, {
    appConfig: {
      get: (key: string) => (key === 'BIOROUTER_WORKING_DIR' ? '/w' : undefined),
    },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

interface PaneSpec {
  sessionId: string;
  text: string;
  chatState?: ChatState;
  droppedFiles?: DroppedFile[];
}

function composerFor(spec: PaneSpec, handleSubmit: SubmitMock) {
  return (
    <ChatInput
      sessionId={spec.sessionId}
      handleSubmit={handleSubmit}
      chatState={spec.chatState ?? ChatState.Idle}
      onStop={vi.fn()}
      initialValue={spec.text}
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={spec.droppedFiles ?? []}
      onFilesProcessed={vi.fn()}
      messagesLength={2}
      disableAnimation
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
}

/** Left and right pane, side by side in ONE React root, as the split renders them. */
function renderSplit(left: PaneSpec, right: PaneSpec) {
  const leftSubmit = vi.fn<SubmitFn>(async () => true);
  const rightSubmit = vi.fn<SubmitFn>(async () => true);
  render(
    <div>
      <section data-testid="pane-left">{composerFor(left, leftSubmit)}</section>
      <section data-testid="pane-right">{composerFor(right, rightSubmit)}</section>
    </div>
  );
  const pane = (side: 'left' | 'right') => {
    const root = () => screen.getByTestId(`pane-${side}`);
    return {
      textarea: () => within(root()).getByTestId('chat-input') as HTMLTextAreaElement,
      send: () => within(root()).getByRole('button', { name: 'Send message' }) as HTMLButtonElement,
      queued: () =>
        within(root())
          .queryAllByTestId('queued-content')
          .map((el) => el.textContent),
    };
  };
  return { leftSubmit, rightSubmit, left: pane('left'), right: pane('right') };
}

const submittedTexts = (handleSubmit: SubmitMock) =>
  handleSubmit.mock.calls.map((call) => (call[0] as unknown as CustomEvent).detail.value as string);

/**
 * Wait until a press has done SOMETHING in either pane, then report where. The
 * assertions compare the whole picture, so a press that lands in the wrong pane
 * fails by naming the message it sent there — not as a bare timeout.
 */
async function landing(split: ReturnType<typeof renderSplit>) {
  const picture = () => ({
    leftSent: submittedTexts(split.leftSubmit),
    rightSent: submittedTexts(split.rightSubmit),
    leftQueued: split.left.queued(),
    rightQueued: split.right.queued(),
  });
  await waitFor(() => {
    const now = picture();
    expect(
      now.leftSent.length + now.rightSent.length + now.leftQueued.length + now.rightQueued.length
    ).toBeGreaterThan(0);
  });
  return picture();
}

describe('Send in a split submits its own pane', () => {
  it("clicking the second composer's Send submits the second composer's text", async () => {
    const split = renderSplit(
      { sessionId: 'chat-left', text: 'left pane draft' },
      { sessionId: 'chat-right', text: 'right pane message' }
    );
    await waitFor(() => expect(split.right.send()).toBeEnabled());

    await userEvent.click(split.right.send());

    expect(await landing(split)).toEqual({
      leftSent: [],
      rightSent: ['right pane message'],
      leftQueued: [],
      rightQueued: [],
    });
    // The other pane is untouched: its draft is still in its box.
    expect(split.left.textarea().value).toBe('left pane draft');
  });

  it("clicking the first composer's Send submits only the first composer", async () => {
    const split = renderSplit(
      { sessionId: 'chat-left', text: 'left pane message' },
      { sessionId: 'chat-right', text: 'right pane draft' }
    );
    await waitFor(() => expect(split.left.send()).toBeEnabled());

    await userEvent.click(split.left.send());

    expect(await landing(split)).toEqual({
      leftSent: ['left pane message'],
      rightSent: [],
      leftQueued: [],
      rightQueued: [],
    });
    expect(split.right.textarea().value).toBe('right pane draft');
  });

  // The structural statement of the same rule, which says WHY the clicks above
  // land where they do: every Send's form owner is the form around its own
  // textarea, and no two composers share a form id for a lookup to confuse.
  it('binds every Send to the form around its own textarea, by an id no other composer holds', async () => {
    const { left, right } = renderSplit(
      { sessionId: 'chat-left', text: 'a' },
      { sessionId: 'chat-right', text: 'b' }
    );
    await waitFor(() => expect(right.send()).toBeEnabled());

    for (const pane of [left, right]) {
      const ownForm = pane.textarea().closest('form');
      expect(ownForm).not.toBeNull();
      expect(pane.send().form).toBe(ownForm);
    }
    const leftFormId = left.textarea().closest('form')!.id;
    const rightFormId = right.textarea().closest('form')!.id;
    expect(leftFormId).not.toBe('');
    expect(leftFormId).not.toBe(rightFormId);
    expect(document.querySelectorAll(`[id="${CSS.escape(leftFormId)}"]`)).toHaveLength(1);
  });

  it('keeps Enter-to-send per pane', async () => {
    const split = renderSplit(
      { sessionId: 'chat-left', text: 'left pane draft' },
      { sessionId: 'chat-right', text: 'right pane message' }
    );

    fireEvent.keyDown(split.right.textarea(), { key: 'Enter', code: 'Enter' });

    expect(await landing(split)).toEqual({
      leftSent: [],
      rightSent: ['right pane message'],
      leftQueued: [],
      rightQueued: [],
    });
    expect(split.left.textarea().value).toBe('left pane draft');
  });

  // Send while a turn runs QUEUES — and it has to queue in the pane whose button
  // was pressed. Through the shared id the press reached the idle left pane
  // instead, which started a turn there with whatever it held.
  it("queues the streaming pane's message in that pane, and starts nothing in the other", async () => {
    const split = renderSplit(
      { sessionId: 'chat-left', text: 'left pane draft' },
      { sessionId: 'chat-right', text: 'right follow-up', chatState: ChatState.Streaming }
    );
    await waitFor(() => expect(split.right.send()).toBeEnabled());

    await userEvent.click(split.right.send());

    expect(await landing(split)).toEqual({
      leftSent: [],
      rightSent: [],
      leftQueued: [],
      rightQueued: ['right follow-up'],
    });
    expect(split.right.textarea().value).toBe('');
    expect(split.left.textarea().value).toBe('left pane draft');
  });

  it("sends the second pane's attachment from the second pane's Send", async () => {
    const report: DroppedFile = {
      id: 'dropped-report',
      path: '/w/data/report.csv',
      name: 'report.csv',
      type: 'text/csv',
      isImage: false,
    };
    const split = renderSplit(
      { sessionId: 'chat-left', text: 'left pane draft' },
      { sessionId: 'chat-right', text: '', droppedFiles: [report] }
    );
    await waitFor(() => expect(split.right.send()).toBeEnabled());

    await userEvent.click(split.right.send());

    expect(await landing(split)).toEqual({
      leftSent: [],
      rightSent: ['/w/data/report.csv'],
      leftQueued: [],
      rightQueued: [],
    });
    expect(split.left.textarea().value).toBe('left pane draft');
  });
});
