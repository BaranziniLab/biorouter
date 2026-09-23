import React from 'react';
import { sendQuotedText, findQuotes } from '../utils/quotedText';
import { act, cleanup } from '@testing-library/react';
import { existingChatComposerDraftKey, resetComposerDraftsForTests } from '../utils/composerDrafts';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// Issue #65 — the composer's half of the reference-tag ruling.
//
// `<biorouter-ref …>` is the canonical form because it is the only one that can
// carry a name with a space in it. That makes it ~45 characters of XML, so the
// composer has to show a chip instead, and the chip has to stay deletable: an
// undeletable reference is worse than the markup it replaced.
//
// The rendered contract asserted here: the textarea holds only prose, the rail
// holds the chips, and the message that leaves the composer holds the tags.

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
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({
  BottomMenuSkillSelection: () => null,
}));
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
import { labelledRefTag, refTag } from '../utils/resourceRefs';

beforeEach(() => {
  vi.clearAllMocks();
  resetComposerDraftsForTests();
  Object.assign(window, {
    appConfig: { get: () => '/w' },
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

// #360 and #362 each added a THIRD positional parameter here and gave it a
// different meaning — a session id and a draft key. Taking either side alone
// would leave the other's tests passing an argument the helper reads as
// something else, which is a merge that stays green and stops testing what it
// names. Both are named, and the one call site that meant `sessionId` says so.
const renderComposer = (
  initialValue: string,
  handleSubmit = vi.fn(),
  draftKey?: string,
  sessionId = 'session-1'
) => {
  render(
    <ChatInput
      sessionId={sessionId}
      draftKey={draftKey}
      handleSubmit={handleSubmit}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue={initialValue}
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
  return handleSubmit;
};

const composer = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;
const submittedText = (handleSubmit: ReturnType<typeof vi.fn>) => {
  const calls = handleSubmit.mock.calls;
  return (calls[calls.length - 1][0] as CustomEvent).detail.value as string;
};

describe('the composer draws references as chips', () => {
  it('shows a chip and keeps the markup out of the textarea', async () => {
    renderComposer(`please run ${refTag('skill', 'my skill')}`);

    await waitFor(() => expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument());
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('my skill');
    expect(composer().value).toBe('please run');
    expect(composer().value).not.toContain('biorouter-ref');
  });

  it('reads a knowledge base by the name the user picked, not its slug', async () => {
    renderComposer(labelledRefTag('knowledge_base', 'soul-body-2024', 'Soul & Body'));

    await waitFor(() =>
      expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('Soul & Body')
    );
  });

  it('draws no rail at all when nothing is attached', () => {
    renderComposer('just a message');

    expect(screen.queryByTestId('composer-reference-rail')).not.toBeInTheDocument();
    expect(composer().value).toBe('just a message');
  });

  // The reference has to survive editing the prose around it — this is the
  // seam where a naive implementation loses the tag on the next keystroke.
  it('keeps the reference while the prose is edited', async () => {
    const handleSubmit = renderComposer(`hello ${refTag('skill', 'my skill')}`);

    await waitFor(() => expect(composer().value).toBe('hello'));
    fireEvent.change(composer(), { target: { value: 'hello there' } });

    expect(composer().value).toBe('hello there');
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('my skill');

    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submittedText(handleSubmit)).toBe(`hello there ${refTag('skill', 'my skill')}`);
  });

  // Typing must not gain or lose a character. The composer stores the tags in
  // the same string the textarea is bound to, so a separator accounted to the
  // body instead of the suffix would show up in the textarea — and React would
  // then reassign the controlled value and drop the caret behind it.
  it('adds no phantom character to the text being typed', async () => {
    renderComposer(refTag('skill', 'my skill'));

    await waitFor(() => expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument());
    expect(composer().value).toBe('');

    await userEvent.type(composer(), 'hi');
    expect(composer().value).toBe('hi');
  });

  it('sends the reference the user never typed a character of', async () => {
    const handleSubmit = renderComposer(refTag('skill', 'my skill'));

    await waitFor(() => expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument());
    fireEvent.change(composer(), { target: { value: 'run it' } });
    fireEvent.submit(composer().closest('form')!);

    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submittedText(handleSubmit)).toBe(`run it ${refTag('skill', 'my skill')}`);
  });
});

describe('the composer keeps references removable', () => {
  it('drops the reference the user removes and keeps the rest', async () => {
    const handleSubmit = renderComposer(
      `go ${refTag('skill', 'first')} ${refTag('extension', 'second')}`
    );

    await waitFor(() => expect(screen.getAllByTestId('resource-ref-chip')).toHaveLength(2));
    await userEvent.click(screen.getAllByRole('button', { name: /^remove/i })[0]);

    await waitFor(() => expect(screen.getAllByTestId('resource-ref-chip')).toHaveLength(1));
    expect(composer().value).toBe('go');

    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submittedText(handleSubmit)).toBe(`go ${refTag('extension', 'second')}`);
  });

  it('leaves the message alone when the last reference is removed', async () => {
    const handleSubmit = renderComposer(`still here ${refTag('skill', 'only')}`);

    await waitFor(() => expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument());
    await userEvent.click(screen.getByRole('button', { name: /^remove/i }));

    await waitFor(() => expect(screen.queryByTestId('resource-ref-chip')).not.toBeInTheDocument());
    fireEvent.submit(composer().closest('form')!);

    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submittedText(handleSubmit)).toBe('still here');
  });
});

// A reference is invisible in the textarea by design, so a command the user
// typed has to be recognised from the prose alone. Reading the whole message
// would make an attached chip silently defeat a command that looks — to the
// user, correctly — like the only thing in the box.
describe('the composer recognises a command with a reference attached', () => {
  it('keeps a typed /diverge in a new chat instead of silently discarding it', async () => {
    const handleSubmit = renderComposer('', vi.fn(), undefined, '');
    fireEvent.change(composer(), { target: { value: '/diverge' } });
    fireEvent.submit(composer().closest('form')!);
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(composer().value).toBe('/diverge');
  });

  it('still diverges', async () => {
    const handleSubmit = renderComposer(refTag('skill', 'my skill'));

    await waitFor(() => expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument());
    fireEvent.change(composer(), { target: { value: '/diverge' } });
    fireEvent.submit(composer().closest('form')!);

    // /diverge branches the conversation instead of becoming a message.
    expect(handleSubmit).not.toHaveBeenCalled();
    await waitFor(() => expect(composer().value).toBe(''));
    expect(screen.queryByTestId('resource-ref-chip')).not.toBeInTheDocument();
  });

  it('still reads an interruption', async () => {
    const handleSubmit = vi.fn();
    render(
      <ChatInput
        sessionId="session-1"
        handleSubmit={handleSubmit}
        chatState={ChatState.Streaming}
        onStop={vi.fn()}
        initialValue={refTag('skill', 'my skill')}
        setView={vi.fn()}
        totalTokens={0}
        accumulatedInputTokens={0}
        accumulatedOutputTokens={0}
        droppedFiles={[]}
        onFilesProcessed={vi.fn()}
        messagesLength={0}
        disableAnimation={false}
        toolCount={0}
        onWorkingDirChange={vi.fn()}
      />
    );

    await waitFor(() => expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument());
    fireEvent.change(composer(), { target: { value: 'nevermind' } });
    fireEvent.submit(composer().closest('form')!);

    // An interruption is queued behind a stop, never submitted directly.
    expect(handleSubmit).not.toHaveBeenCalled();
    await waitFor(() => expect(composer().value).toBe(''));
  });
});

describe('the composer degrades a tag it cannot read', () => {
  // Never a blank and never a crash: a truncated tag stays visible as the text
  // it is, which is also honest — the backend will not resolve it either.
  it('leaves a malformed tag in the textarea as written', async () => {
    const broken = `<biorouter-ref type="skill" name="never closed`;
    renderComposer(broken);

    await waitFor(() => expect(composer().value).toBe(broken));
    expect(screen.queryByTestId('resource-ref-chip')).not.toBeInTheDocument();
  });
});

describe('selected quotation in a composer', () => {
  it('preserves the draft and references and never autosends', async () => {
    const submit = renderComposer(`Question ${refTag('skill', 'research')}`);
    await waitFor(() => expect(composer().value).toBe('Question'));
    const text = 'Exact "quote"\n<tool>source data</tool>';
    act(() => sendQuotedText({ source: { sessionId: 'session-1', title: 'Results.docx' }, text }));
    expect(composer().value).toBe('Question');
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('research');
    expect(
      screen.getByRole('button', { name: 'Remove quote from Results.docx' })
    ).toBeInTheDocument();
    expect(submit).not.toHaveBeenCalled();
    fireEvent.change(composer(), { target: { value: 'Follow-up' } });
    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(submit).toHaveBeenCalled());
    expect(findQuotes(submittedText(submit))[0].value).toBe(text);
  });
  it('removes a quote without changing existing body or resource', async () => {
    renderComposer(`Keep ${refTag('skill', 'research')}`);
    await waitFor(() => expect(composer().value).toBe('Keep'));
    act(() =>
      sendQuotedText({ source: { sessionId: 'session-1', title: 'Response' }, text: 'selection' })
    );
    fireEvent.click(screen.getByRole('button', { name: 'Remove quote from Response' }));
    expect(composer().value).toBe('Keep');
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('research');
    expect(
      screen.queryByRole('button', { name: 'Remove quote from Response' })
    ).not.toBeInTheDocument();
  });
});

describe('existing chat quote drafts outlive the composer', () => {
  it('restores the same tab after unmount without leaking into another tab of the same session', async () => {
    const origin = existingChatComposerDraftKey('origin-tab', 'session-1');
    const other = existingChatComposerDraftKey('other-tab', 'session-1');
    renderComposer(`Keep my draft ${refTag('skill', 'research')}`, vi.fn(), origin);
    await waitFor(() => expect(composer().value).toBe('Keep my draft'));
    const text = 'Exact "quotation"\nsecond line';
    act(() => sendQuotedText({ source: { sessionId: 'session-1', title: 'Response' }, text }));
    cleanup();
    renderComposer('Other tab draft', vi.fn(), other);
    expect(composer().value).toBe('Other tab draft');
    expect(screen.queryByTestId('quoted-text-chip')).not.toBeInTheDocument();
    cleanup();
    renderComposer('', vi.fn(), origin);
    expect(composer().value).toBe('Keep my draft');
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('research');
    expect(screen.getByTestId('quoted-text-chip').textContent).toContain(text);
    fireEvent.click(screen.getByRole('button', { name: 'Remove quote from Response' }));
    cleanup();
    renderComposer('', vi.fn(), origin);
    expect(screen.queryByTestId('quoted-text-chip')).not.toBeInTheDocument();
    expect(composer().value).toBe('Keep my draft');
  });
});
