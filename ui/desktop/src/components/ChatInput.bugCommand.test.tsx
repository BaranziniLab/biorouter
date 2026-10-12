import React from 'react';
import { describe, expect, it, vi, beforeEach, type Mock } from 'vitest';
import { fireEvent, render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// `/bug` is drawn as a chip ("pellet") in the composer: the rail shows it, the
// textarea holds only the prose after it, and the message that leaves the
// composer still starts with `/bug `, where the daemon reads a command.

vi.mock('../toasts', () => ({
  toastWarning: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
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
    getCurrentModelAndProvider: vi.fn(async () => ({ model: 'gpt-5.5', provider: 'versa_azure' })),
    currentModel: 'gpt-5.5',
    currentProvider: 'versa_azure',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
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
vi.mock('./MessageQueue', () => ({
  default: ({
    queuedMessages = [],
  }: {
    queuedMessages?: Array<{ id: string; content: string }>;
  }) => (
    <ul data-testid="message-queue">
      {queuedMessages.map((message) => (
        <li key={message.id}>{message.content}</li>
      ))}
    </ul>
  ),
  canSteerMessage: () => false,
}));

// The popover is replaced by one row, the daemon's `bug` built-in, and what it
// inserts comes from the REAL `getMentionInsertText`: the contract under test is
// the popover's insert and the composer's reading of it together.
vi.mock('./MentionPopover', async () => {
  const actual = await vi.importActual<typeof import('./MentionPopover')>('./MentionPopover');
  const MentionPopoverMock = React.forwardRef(
    (props: { isOpen: boolean; onSelect: (value: string) => void }, _ref) =>
      props.isOpen ? (
        <button
          type="button"
          data-testid="slash-bug-option"
          onClick={() =>
            props.onSelect(
              actual.getMentionInsertText({
                name: 'bug',
                extra: 'Report a Biorouter bug',
                itemType: 'Builtin',
                relativePath: 'bug',
                builtIn: true,
              })
            )
          }
        >
          bug
        </button>
      ) : null
  );
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { ...actual, default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { refTag } from '../utils/resourceRefs';
import { resetComposerDraftsForTests } from '../utils/composerDrafts';
import { restoreComposerText } from '../utils/composerRestore';
import { resetAnnotationChannelForTests, sendArtifactAnnotation } from '../utils/annotationChannel';

beforeEach(() => {
  vi.clearAllMocks();
  resetComposerDraftsForTests();
  Object.assign(window, {
    appConfig: { get: () => '/tmp/biorouter-workdir' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
      deleteTempFile: vi.fn(),
    },
  });
});

type SubmitFn = (event: React.FormEvent) => void | Promise<boolean | void>;
type SubmitMock = Mock<SubmitFn>;

const renderComposer = (
  options: {
    initialValue?: string;
    chatState?: ChatState;
    messagesLength?: number;
    commandHistory?: string[];
  } = {}
) => {
  const handleSubmit = vi.fn<SubmitFn>(async () => true);
  render(
    <ChatInput
      sessionId="session-42"
      handleSubmit={handleSubmit}
      chatState={options.chatState ?? ChatState.Idle}
      onStop={vi.fn()}
      initialValue={options.initialValue ?? ''}
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={options.messagesLength ?? 0}
      commandHistory={options.commandHistory}
      disableAnimation
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
  return handleSubmit;
};

const composer = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;
const chip = () => screen.queryByTestId('command-chip');
const submitted = (handleSubmit: SubmitMock) => {
  const calls = handleSubmit.mock.calls;
  return (calls[calls.length - 1][0] as unknown as CustomEvent).detail.value as string;
};

/** Type into the textarea the way the browser reports it: value and caret. */
const typeProse = (value: string, caret = value.length) => {
  fireEvent.change(composer(), { target: { value, selectionStart: caret, selectionEnd: caret } });
};

const pickBugFromPopover = async () => {
  fireEvent.click(await screen.findByTestId('slash-bug-option'));
};

describe('picking /bug from the slash menu', () => {
  it('turns it into a chip and leaves the textarea empty', async () => {
    renderComposer();
    typeProse('/bu');
    await pickBugFromPopover();

    expect(chip()).toBeInTheDocument();
    expect(chip()).toHaveAttribute('data-command', 'bug');
    expect(screen.getByTestId('command-chip-name')).toHaveTextContent('Report a bug');
    expect(composer().value).toBe('');
    expect(composer().placeholder).toBe('Describe what went wrong (optional)');
  });

  it('takes the typed query out of the prose wherever it was', async () => {
    renderComposer();
    typeProse('the chart is blank /bu');
    await pickBugFromPopover();

    expect(chip()).toBeInTheDocument();
    expect(composer().value).toBe('the chart is blank ');
  });

  // The query sits mid-prose, with text after it: jsdom puts the caret at the
  // END of a value set programmatically, so only text after the query shows
  // whether the caret went back to where the user was typing.
  it('puts the caret back where the query was', async () => {
    renderComposer();
    typeProse('the chart /bu is blank', 'the chart /bu'.length);
    await pickBugFromPopover();

    expect(chip()).toBeInTheDocument();
    expect(composer().value).toBe('the chart  is blank');
    await waitFor(() => expect(composer().selectionStart).toBe('the chart '.length));
  });

  it('sends /bug and the prose, with the command at the start', async () => {
    const handleSubmit = renderComposer();
    typeProse('/bu');
    await pickBugFromPopover();
    typeProse('the chart panel is blank');
    fireEvent.submit(composer().closest('form')!);

    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe('/bug the chart panel is blank');
  });

  it('can send the chip on its own', async () => {
    const handleSubmit = renderComposer();
    typeProse('/bu');
    await pickBugFromPopover();
    expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();

    fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter' });

    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe('/bug');
    await waitFor(() => expect(chip()).not.toBeInTheDocument());
  });
});

describe('typing /bug', () => {
  it('converts "/bug " at the start into the chip', async () => {
    renderComposer();
    typeProse('/bug ');

    expect(chip()).toBeInTheDocument();
    expect(composer().value).toBe('');
  });

  it('keeps what follows when "/bug " is typed in front of existing prose', async () => {
    const handleSubmit = renderComposer();
    typeProse('/bug the export button does nothing', 5);

    expect(chip()).toBeInTheDocument();
    expect(composer().value).toBe('the export button does nothing');
    // The caret goes back to where the user was typing, not to the end.
    await waitFor(() => expect(composer().selectionStart).toBe(0));

    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe('/bug the export button does nothing');
  });

  it('leaves a bare /bug, /bugs and /bug in the middle as text', async () => {
    renderComposer();
    for (const text of ['/bug', '/bugs ', 'x /bug y', '/Bug x']) {
      typeProse(text);
      expect(chip(), text).not.toBeInTheDocument();
      expect(composer().value).toBe(text);
    }
  });

  it('adds no phantom character while typing after the chip', async () => {
    renderComposer();
    typeProse('/bug ');
    await userEvent.type(composer(), 'hi');

    expect(composer().value).toBe('hi');
    expect(chip()).toBeInTheDocument();
  });
});

describe('removing the chip', () => {
  it('drops the command and keeps the prose when its × is clicked', async () => {
    const handleSubmit = renderComposer({ initialValue: '/bug keep these words' });
    await waitFor(() => expect(chip()).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Remove command /bug' }));

    expect(chip()).not.toBeInTheDocument();
    expect(composer().value).toBe('keep these words');
    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe('keep these words');
  });

  it('drops it on Backspace at the start of the prose', async () => {
    renderComposer({ initialValue: '/bug words' });
    await waitFor(() => expect(chip()).toBeInTheDocument());

    composer().setSelectionRange(0, 0);
    fireEvent.keyDown(composer(), { key: 'Backspace', code: 'Backspace' });

    expect(chip()).not.toBeInTheDocument();
    expect(composer().value).toBe('words');
  });

  it('keeps it on Backspace anywhere else, or over a selection', async () => {
    renderComposer({ initialValue: '/bug words' });
    await waitFor(() => expect(chip()).toBeInTheDocument());

    composer().setSelectionRange(3, 3);
    fireEvent.keyDown(composer(), { key: 'Backspace', code: 'Backspace' });
    expect(chip()).toBeInTheDocument();

    composer().setSelectionRange(0, 3);
    fireEvent.keyDown(composer(), { key: 'Backspace', code: 'Backspace' });
    expect(chip()).toBeInTheDocument();
  });

  it('drops it on Backspace in an empty box', async () => {
    renderComposer();
    typeProse('/bug ');
    expect(chip()).toBeInTheDocument();

    fireEvent.keyDown(composer(), { key: 'Backspace', code: 'Backspace' });
    expect(chip()).not.toBeInTheDocument();
    expect(composer().value).toBe('');
  });
});

describe('the chip alongside the rest of the composer', () => {
  it('rides first in the rail with references, and both reach the message', async () => {
    const handleSubmit = renderComposer({
      initialValue: `/bug the panel is blank ${refTag('skill', 'my skill')}`,
    });
    await waitFor(() => expect(chip()).toBeInTheDocument());

    const rail = screen.getByTestId('composer-reference-rail');
    expect(rail.firstElementChild).toBe(chip());
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('my skill');
    expect(composer().value).toBe('the panel is blank');

    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe(`/bug the panel is blank ${refTag('skill', 'my skill')}`);
  });

  it('restores a draft or deep link that starts with /bug as the chip', async () => {
    renderComposer({ initialValue: '/bug from a draft' });

    await waitFor(() => expect(chip()).toBeInTheDocument());
    expect(composer().value).toBe('from a draft');
  });

  it('queues /bug with its prose while a turn runs', async () => {
    const handleSubmit = renderComposer({ chatState: ChatState.Streaming });
    typeProse('/bug ');
    typeProse('the table never loads');

    act(() => {
      fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter' });
    });

    expect(handleSubmit).not.toHaveBeenCalled();
    expect(screen.getByTestId('message-queue')).toHaveTextContent('/bug the table never loads');
    expect(chip()).not.toBeInTheDocument();
  });

  // A region selected in the preview panel appends its context after the
  // prose. Trimming the whole text there ate the space that makes `/bug ` a
  // chip, and the command fell into the textarea as text.
  it('keeps the chip when a preview region is attached to a chip-only composer', async () => {
    resetAnnotationChannelForTests();
    Object.assign(window.electron, {
      readTempImageAsBase64: vi.fn(async () => ({ data: 'AAAA', mimeType: 'image/png' })),
    });
    renderComposer();
    typeProse('/bug ');
    expect(chip()).toBeInTheDocument();

    act(() => {
      sendArtifactAnnotation({
        sessionId: 'session-42',
        imagePath: '/tmp/x.png',
        sourceTitle: 'Chart',
        region: { x: 0, y: 0, width: 1, height: 1, surfaceWidth: 1, surfaceHeight: 1 },
        width: 1,
        height: 1,
      });
    });

    await waitFor(() => expect(composer().value).not.toBe(''));
    expect(chip()).toBeInTheDocument();
    expect(composer().value.startsWith('/bug')).toBe(false);
  });

  it('treats /crew typed behind the chip as part of the report', async () => {
    const handleSubmit = renderComposer();
    typeProse('/bug ');
    typeProse('/crew');
    fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter' });

    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe('/bug /crew');
  });
});

// A chip sent on its own leaves as a bare `/bug` (the send trims), and that is
// what the chat's history, a handed-back message and the queue keep. Text the
// person did not type must come back as the chip it was sent as.
describe('a sent chip-only /bug coming back to the composer', () => {
  it('is recalled from the chat history as the chip', async () => {
    renderComposer({ commandHistory: ['/bug'] });

    fireEvent.keyDown(composer(), { key: 'ArrowUp', code: 'ArrowUp', metaKey: true });

    await waitFor(() => expect(chip()).toBeInTheDocument());
    expect(composer().value).toBe('');
  });

  it('is restored to its chat as the chip', async () => {
    renderComposer();

    act(() => {
      restoreComposerText({ sessionId: 'session-42', value: '/bug' });
    });

    await waitFor(() => expect(chip()).toBeInTheDocument());
    expect(composer().value).toBe('');
  });
});
