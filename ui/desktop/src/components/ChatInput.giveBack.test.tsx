import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, fireEvent, within, waitFor } from '@testing-library/react';

/**
 * "Your message was kept" has to be true of the MESSAGE, in the composer the
 * person typed it into, and of nothing else. Four ways it was not, each measured
 * in the dev app on 1.90.4 with `POST /agent/start` failing from a fresh tab:
 *
 *   1. a failed start in the LEFT pane of a split filled BOTH panes' new-tab
 *      composers, 2 of 2 — replacing "MY OWN UNSENT DRAFT" in the right one —
 *      because the give-back was a window-wide event addressed `''`;
 *   2. a pasted image was gone after the failure, while the text came back;
 *   3. a new tab's unsent text was gone after a tab switch, and after going to
 *      Settings and back;
 *   (4, the toast's punctuation, is `utils/startChatFailure.test.ts`.)
 *
 * These tests drive the REAL composer through what a person does — type, paste,
 * drop, press Enter — against the smallest stand-in for a new chat's surface:
 * its send resolves `false` when the start fails (ChatInput's "not taken"), and
 * the failure moves the composer between two slots, which remounts it, exactly
 * as `BaseChat`'s `isCreatingSession` does. A composer is addressed only by the
 * key it is mounted with; the keys here are arbitrary strings on purpose.
 */

vi.mock('../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  toastWarning: vi.fn(),
  toastInfo: vi.fn(),
  toastService: { error: vi.fn(), configure: vi.fn() },
}));
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: null,
    currentProvider: null,
    // A model that reads images, so a paste is staged the way it is in the app.
    currentModelSupportsVision: true,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
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
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import {
  giveBackToComposer,
  readComposerDraft,
  resetComposerDraftsForTests,
  unsentComposerTabs,
} from '../utils/composerDrafts';

const deleteTempFile = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  resetComposerDraftsForTests();
  let saved = 0;
  Object.assign(window, {
    appConfig: { get: () => '/w' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
      saveDataUrlToTemp: vi.fn(async (_dataUrl: string, id: string) => ({
        id,
        filePath: `/tmp/biorouter-test/pasted-${++saved}.png`,
      })),
      readTempImageAsBase64: vi.fn(async () => ({ data: 'AAAA', mimeType: 'image/png' })),
      deleteTempFile,
    },
  });
});

const nextTask = () => new Promise((resolve) => setTimeout(resolve, 0));

/**
 * A new chat's surface. `fail()` decides each start; a failure moves the
 * composer to the other slot and back, which remounts it, and resolves `false`.
 */
function NewChat({
  draftKey,
  fail = () => true,
  label,
}: {
  draftKey?: string;
  fail?: () => boolean;
  label: string;
}) {
  const [isCreatingSession, setIsCreatingSession] = React.useState(false);
  const [started, setStarted] = React.useState(false);
  const handleSubmit = async (): Promise<boolean> => {
    setIsCreatingSession(true);
    await Promise.resolve();
    if (!fail()) {
      setStarted(true);
      return true;
    }
    setIsCreatingSession(false);
    return false;
  };
  const composer = (
    <ChatInput
      sessionId={started ? 'sess-started' : ''}
      draftKey={started ? undefined : draftKey}
      handleSubmit={handleSubmit}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      messagesLength={0}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
  const clean = !started && !isCreatingSession;
  return (
    <section aria-label={label}>
      {clean ? <div data-slot="empty-state">{composer}</div> : null}
      {!clean && <div data-slot="under-transcript">{composer}</div>}
    </section>
  );
}

/** Home's composer and an existing chat's: a composer with no key. */
function PlainComposer({
  sessionId,
  label,
  refuse = false,
}: {
  sessionId: string | null;
  label: string;
  refuse?: boolean;
}) {
  return (
    <section aria-label={label}>
      <ChatInput
        sessionId={sessionId}
        handleSubmit={async () => {
          await Promise.resolve();
          return !refuse;
        }}
        chatState={ChatState.Idle}
        onStop={vi.fn()}
        initialValue=""
        setView={vi.fn()}
        totalTokens={0}
        accumulatedInputTokens={0}
        accumulatedOutputTokens={0}
        messagesLength={0}
        disableAnimation={false}
        toolCount={0}
        onWorkingDirChange={vi.fn()}
      />
    </section>
  );
}

const pane = (label: string) => screen.getByRole('region', { name: label });
const box = (label: string) => within(pane(label)).getByRole('textbox') as HTMLTextAreaElement;

const type = async (label: string, text: string) => {
  await act(async () => {
    fireEvent.change(box(label), { target: { value: text } });
  });
};

const pressEnter = async (label: string) => {
  await act(async () => {
    fireEvent.keyDown(box(label), { key: 'Enter', code: 'Enter' });
    await nextTask();
    await nextTask();
  });
};

const pasteImage = async (label: string) => {
  const file = new File([new Uint8Array([137, 80, 78, 71])], 'shot.png', { type: 'image/png' });
  await act(async () => {
    fireEvent.paste(box(label), { clipboardData: { files: [file], items: [], getData: () => '' } });
  });
  await waitFor(() =>
    expect(within(pane(label)).getAllByAltText(/^Pasted image/).length).toBeGreaterThan(0)
  );
};

const dropFile = async (label: string, path: string) => {
  const zone = pane(label).querySelector('[data-drop-zone="true"]') as HTMLElement;
  await act(async () => {
    fireEvent.drop(zone, {
      dataTransfer: {
        files: [],
        getData: (kind: string) => (kind === 'text/uri-list' ? `file://${path}` : ''),
      },
    });
  });
};

const pastedImages = (label: string) => within(pane(label)).queryAllByAltText(/^Pasted image/);
const fileChip = (label: string, name: string) => within(pane(label)).queryByTitle(name);

describe('1 — a failed start gives the message back to ITS composer and no other', () => {
  it('leaves the other pane’s new tab exactly as it was', async () => {
    render(
      <>
        <NewChat label="left" draftKey="tab:one-left" />
        <NewChat label="right" draftKey="tab:one-right" />
      </>
    );
    await type('right', 'MY OWN UNSENT DRAFT');
    await type('left', 'ANOTHER PANE MESSAGE');

    await pressEnter('left');

    // Measured on 1.90.4: this read "ANOTHER PANE MESSAGE".
    expect(box('right').value).toBe('MY OWN UNSENT DRAFT');
    expect(box('left').value).toBe('ANOTHER PANE MESSAGE');
  });

  it('does not reach an EMPTY new tab in another pane either', async () => {
    render(
      <>
        <NewChat label="left" draftKey="tab:two-left" />
        <NewChat label="right" draftKey="tab:two-right" />
      </>
    );
    await type('left', 'only mine');
    await pressEnter('left');

    expect(box('right').value).toBe('');
    expect(box('left').value).toBe('only mine');
  });

  it('never replaces what an existing chat’s composer holds with a restore for that chat', async () => {
    // The one broadcast left (`returnInitialMessageToComposer`) names a real
    // chat. It merges: the person's own unsent text stays, the returned message
    // is put before it.
    render(<PlainComposer label="chat" sessionId="sess-9" />);
    await type('chat', 'what I am typing now');

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('restore-chat-input', {
          detail: { sessionId: 'sess-9', value: 'the refused message', attachments: [] },
        })
      );
    });

    expect(box('chat').value).toContain('what I am typing now');
    expect(box('chat').value).toContain('the refused message');
  });

  it('a restore with no chat id reaches neither a new tab nor Home', async () => {
    render(
      <>
        <NewChat label="new" draftKey="tab:three" />
        <PlainComposer label="home" sessionId={null} />
      </>
    );
    await type('new', 'new tab draft');
    await type('home', 'home draft');

    await act(async () => {
      for (const sessionId of ['', null, undefined]) {
        window.dispatchEvent(
          new CustomEvent('restore-chat-input', { detail: { sessionId, value: 'INTRUDER' } })
        );
      }
    });

    expect(box('new').value).toBe('new tab draft');
    expect(box('home').value).toBe('home draft');
  });
});

describe('2 — the message kept is the whole message: text, images and files', () => {
  it('a new tab gets its pasted image and dropped file back after a failed start', async () => {
    render(<NewChat label="new" draftKey="tab:four" />);
    await pasteImage('new');
    await dropFile('new', '/Users/me/cohort.csv');
    await type('new', 'look at these');
    expect(fileChip('new', 'cohort.csv')).not.toBeNull();

    await pressEnter('new');

    // Measured on 1.90.4: the text returned and the image did not.
    expect(pastedImages('new')).toHaveLength(1);
    expect(fileChip('new', 'cohort.csv')).not.toBeNull();
    expect(box('new').value).toBe('look at these');
    // Nothing the message still needs was deleted on the way.
    expect(deleteTempFile).not.toHaveBeenCalled();
  });

  it('Home’s composer (no key, not remounted) gets its dropped file back too', async () => {
    render(<PlainComposer label="home" sessionId={null} refuse />);
    await pasteImage('home');
    await dropFile('home', '/Users/me/notes.txt');
    await type('home', 'home message');

    await pressEnter('home');

    expect(box('home').value).toBe('home message');
    expect(pastedImages('home')).toHaveLength(1);
    expect(fileChip('home', 'notes.txt')).not.toBeNull();
  });
});

describe('3 — a new tab’s unsent message outlives its composer', () => {
  it('is still there when the tab’s composer is mounted again (a tab switch)', async () => {
    const first = render(<NewChat label="tab" draftKey="tab:six" />);
    await pasteImage('tab');
    await dropFile('tab', '/Users/me/plan.md');
    await type('tab', 'typed, never sent');

    await act(async () => first.unmount());

    render(<NewChat label="tab" draftKey="tab:six" />);
    expect(box('tab').value).toBe('typed, never sent');
    expect(pastedImages('tab')).toHaveLength(1);
    expect(fileChip('tab', 'plan.md')).not.toBeNull();
    // The composer that went did not delete the staged image: the tab owns it.
    expect(deleteTempFile).not.toHaveBeenCalled();
  });

  it('is still there after a failed start and a tab switch', async () => {
    const first = render(<NewChat label="tab" draftKey="tab:seven" />);
    await type('tab', 'failed, then I switched tabs');
    await pressEnter('tab');

    await act(async () => first.unmount());
    render(<NewChat label="tab" draftKey="tab:seven" />);

    expect(box('tab').value).toBe('failed, then I switched tabs');
  });

  it('is not shown in another tab', async () => {
    const first = render(<NewChat label="tab" draftKey="tab:eight" />);
    await type('tab', 'belongs to eight');
    await act(async () => first.unmount());

    render(<NewChat label="tab" draftKey="tab:nine" />);

    expect(box('tab').value).toBe('');
  });
});

/**
 * The pane tree is keyed by its SHAPE, so splitting a pane, closing the other
 * half of a split or dragging a tab into a new pane rebuilds a composer in ONE
 * commit: the replacement renders — and reads its seed — before the old one's
 * unmount runs. `Pane` stands in for that: changing `shape` swaps the key.
 */
function Pane({ shape, children }: { shape: string; children: React.ReactNode }) {
  return <div key={shape}>{children}</div>;
}

describe('D2 — a composer rebuilt in one commit shows what the box held, not an older save', () => {
  it('a new tab that was never saved keeps its text when its pane is rebuilt', async () => {
    // Measured in the production bundle: type into a new tab, drag it into a new
    // pane — the composer was empty, and still empty after switching away and
    // back.
    const view = render(
      <Pane shape="single">
        <NewChat label="tab" draftKey="tab:d2-drag" />
      </Pane>
    );
    await type('tab', 'NEVER SAVED THEN DRAGGED');

    view.rerender(
      <Pane shape="split-right">
        <NewChat label="tab" draftKey="tab:d2-drag" />
      </Pane>
    );
    expect(box('tab').value).toBe('NEVER SAVED THEN DRAGGED');

    // And the rebuilt composer did not save an empty box over it on its way out.
    view.unmount();
    render(<NewChat label="tab" draftKey="tab:d2-drag" />);
    expect(box('tab').value).toBe('NEVER SAVED THEN DRAGGED');
  });

  it('text typed since the last save survives a split and then a collapse', async () => {
    // Measured: "PROD A" saved by a tab switch, then " PROD B" typed; a split
    // showed "PROD A". Then " PROD C" typed and the split collapsed: "PROD A PROD
    // B" came back over "PROD A PROD C" on screen.
    const first = render(<NewChat label="tab" draftKey="tab:d2-prod" />);
    await type('tab', 'PROD A');
    first.unmount();

    const view = render(
      <Pane shape="single">
        <NewChat label="tab" draftKey="tab:d2-prod" />
      </Pane>
    );
    expect(box('tab').value).toBe('PROD A');
    await type('tab', 'PROD A PROD B');

    view.rerender(
      <Pane shape="split">
        <NewChat label="tab" draftKey="tab:d2-prod" />
      </Pane>
    );
    expect(box('tab').value).toBe('PROD A PROD B');

    await type('tab', 'PROD A PROD B PROD C');
    view.rerender(
      <Pane shape="collapsed">
        <NewChat label="tab" draftKey="tab:d2-prod" />
      </Pane>
    );
    expect(box('tab').value).toBe('PROD A PROD B PROD C');
  });

  it('a staged image is in the rebuilt composer and is not deleted', async () => {
    const view = render(
      <Pane shape="single">
        <NewChat label="tab" draftKey="tab:d2-image" />
      </Pane>
    );
    await pasteImage('tab');
    await type('tab', 'DRAG ME WITH MY DRAFT');

    view.rerender(
      <Pane shape="split">
        <NewChat label="tab" draftKey="tab:d2-image" />
      </Pane>
    );

    expect(box('tab').value).toBe('DRAG ME WITH MY DRAFT');
    expect(pastedImages('tab')).toHaveLength(1);
    expect(deleteTempFile).not.toHaveBeenCalled();
  });

  it('the store knows what a composer still on screen holds, as it is typed', async () => {
    // Where a started chat may go is decided from the store
    // (`unsentComposerTabs`), at a moment the composer holding a draft may never
    // have been unmounted — a launcher message arriving while the person types
    // into a blank tab. A draft saved only on the way out was invisible there.
    render(<NewChat label="tab" draftKey="tab:on-screen" />);
    await type('tab', 'still typing');
    expect(readComposerDraft('tab:on-screen')?.text).toBe('still typing');
    expect(unsentComposerTabs().drafted).toEqual(['on-screen']);

    await pasteImage('tab');
    expect(readComposerDraft('tab:on-screen')?.images).toHaveLength(1);

    await type('tab', '');
    expect(readComposerDraft('tab:on-screen')?.text).toBe('');
  });

  it('a give-back that lands after the composer read its seed is shown, not overwritten', async () => {
    // Between a composer's render and its commit nothing of it is listening
    // yet. `LateGiveBack` renders after the composer in the same pass and hands
    // a message back under its key in exactly that gap.
    let handed = false;
    function LateGiveBack() {
      if (!handed) {
        handed = true;
        giveBackToComposer('tab:d2-late', {
          text: 'returned in the gap',
          images: [],
          files: [],
        });
      }
      return null;
    }
    render(
      <>
        <NewChat label="tab" draftKey="tab:d2-late" />
        <LateGiveBack />
      </>
    );

    await waitFor(() => expect(box('tab').value).toBe('returned in the gap'));
    expect(readComposerDraft('tab:d2-late')?.text).toBe('returned in the gap');
  });
});

describe('D4 — a message still in flight when its composer goes is handed back to the next one', () => {
  it('fails after the tab was left, and is there when it is shown again', async () => {
    let answer!: (taken: boolean) => void;
    function Sender() {
      return (
        <section aria-label="tab">
          <ChatInput
            sessionId=""
            draftKey="tab:d4-away"
            handleSubmit={() => new Promise<boolean>((resolve) => (answer = resolve))}
            chatState={ChatState.Idle}
            onStop={vi.fn()}
            initialValue=""
            setView={vi.fn()}
            totalTokens={0}
            accumulatedInputTokens={0}
            accumulatedOutputTokens={0}
            messagesLength={0}
            disableAnimation={false}
            toolCount={0}
            onWorkingDirChange={vi.fn()}
          />
        </section>
      );
    }
    const first = render(<Sender />);
    await pasteImage('tab');
    await type('tab', 'sent, then I went to Settings');
    await pressEnter('tab');
    expect(box('tab').value).toBe('');

    first.unmount();
    await act(async () => {
      answer(false);
      await nextTask();
    });

    render(<Sender />);
    expect(box('tab').value).toBe('sent, then I went to Settings');
    expect(pastedImages('tab')).toHaveLength(1);
    expect(deleteTempFile).not.toHaveBeenCalled();
  });
});
