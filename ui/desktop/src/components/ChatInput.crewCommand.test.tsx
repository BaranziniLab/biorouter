import React from 'react';
import { describe, expect, it, vi, beforeEach, type Mock } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

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

const model = vi.hoisted(() => ({
  currentProvider: 'versa_azure' as string | null,
  currentModel: 'gpt-5.5' as string | null,
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({
      model: model.currentModel,
      provider: model.currentProvider,
    })),
    currentModel: model.currentModel,
    currentProvider: model.currentProvider,
    modelConfigStatus: 'ready',
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

const mention = vi.hoisted(() => ({
  props: null as null | { isOpen: boolean; onSelect: (value: string) => void },
}));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(
    (props: { isOpen: boolean; onSelect: (value: string) => void }, _ref) => {
      mention.props = props;
      return props.isOpen ? (
        <button
          type="button"
          data-testid="slash-crew-option"
          onClick={() => props.onSelect('/crew')}
        >
          Crew
        </button>
      ) : null;
    }
  );
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
import type { DroppedFile } from '../hooks/useFileDrop';
import type { View, ViewOptions } from '../utils/navigationUtils';
import { refTag } from '../utils/resourceRefs';
import { resetComposerDraftsForTests } from '../utils/composerDrafts';
import { toastWarning } from '../toasts';

const droppedReport: DroppedFile = {
  id: 'drop-report',
  path: '/tmp/report.txt',
  sourcePath: '/tmp/report.txt',
  name: 'report.txt',
  type: 'text/plain',
  isImage: false,
  canUploadAsImage: false,
  isLoading: false,
};

beforeEach(() => {
  vi.clearAllMocks();
  resetComposerDraftsForTests();
  model.currentProvider = 'versa_azure';
  model.currentModel = 'gpt-5.5';
  mention.props = null;
  let imageNumber = 0;
  Object.assign(window, {
    appConfig: { get: () => '/tmp/biorouter-workdir' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
      saveDataUrlToTemp: vi.fn(async () => ({
        id: `pasted-${++imageNumber}`,
        filePath: `/tmp/biorouter-pasted-${imageNumber}.png`,
      })),
      readTempImageAsBase64: vi.fn(async () => ({ data: 'AAAA', mimeType: 'image/png' })),
      deleteTempFile: vi.fn(),
    },
  });
});

type SubmitFn = (event: React.FormEvent) => void | Promise<boolean | void>;
type SetViewFn = (view: View, options?: ViewOptions) => void;
type StopFn = (continuationPending?: boolean) => boolean | void | Promise<boolean | void>;
type SubmitMock = Mock<SubmitFn>;
type SetViewMock = Mock<SetViewFn>;
type StopMock = Mock<StopFn>;

type RenderOptions = {
  initialValue?: string;
  sessionId?: string | null;
  chatState?: ChatState;
  submissionBlocked?: boolean;
  droppedFiles?: DroppedFile[];
  draftKey?: string;
  handleSubmit?: SubmitMock;
  setView?: SetViewMock;
  onStop?: StopMock;
};

const renderComposer = (options: RenderOptions = {}) => {
  const handleSubmit = options.handleSubmit ?? vi.fn<SubmitFn>(async () => true);
  const setView = options.setView ?? vi.fn<SetViewFn>();
  const onStop = options.onStop ?? vi.fn<StopFn>();
  const view = render(
    <ChatInput
      sessionId={options.sessionId === undefined ? 'session-42' : options.sessionId}
      handleSubmit={handleSubmit}
      chatState={options.chatState ?? ChatState.Idle}
      submissionBlocked={options.submissionBlocked}
      onStop={onStop}
      initialValue={options.initialValue ?? ''}
      draftKey={options.draftKey}
      setView={setView}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={options.droppedFiles ?? []}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      disableAnimation
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
  return { handleSubmit, setView, onStop, view };
};

const composer = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;
const sendButton = () => screen.getByRole('button', { name: 'Send message' });
const submitValue = (handleSubmit: SubmitMock) =>
  (handleSubmit.mock.calls[0][0] as unknown as CustomEvent).detail.value as string;

describe('the /crew composer navigation command', () => {
  it('opens Crew from Enter and passes the current session without submitting', () => {
    const { handleSubmit, setView } = renderComposer({ initialValue: '/crew' });

    fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter' });

    expect(setView).toHaveBeenCalledWith('crew', { resumeSessionId: 'session-42' });
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(composer().value).toBe('');
  });

  it('opens Crew from Send with no session and bypasses provider, queue, and submission gates', () => {
    model.currentProvider = null;
    model.currentModel = null;
    const { handleSubmit, onStop, setView } = renderComposer({
      initialValue: '/crew',
      sessionId: null,
      chatState: ChatState.Streaming,
      submissionBlocked: true,
    });

    fireEvent.click(sendButton());

    expect(setView).toHaveBeenCalledWith('crew', undefined);
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(onStop).not.toHaveBeenCalled();
    expect(screen.queryByTestId('message-queue')).toBeNull();
  });

  it('selects Crew from the slash menu without submitting to the active model', async () => {
    const { handleSubmit, setView } = renderComposer();

    fireEvent.change(composer(), { target: { value: '/', selectionStart: 1 } });
    await waitFor(() => expect(screen.getByTestId('slash-crew-option')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('slash-crew-option'));

    expect(setView).toHaveBeenCalledWith('crew', { resumeSessionId: 'session-42' });
    expect(handleSubmit).not.toHaveBeenCalled();
  });
});

describe('the /crew command refuses local attachment extras', () => {
  it('keeps a reference chip and draft, with no dispatch', async () => {
    const { handleSubmit, setView } = renderComposer({
      initialValue: `/crew ${refTag('skill', 'attached skill')}`,
    });

    await waitFor(() => expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument());
    fireEvent.click(sendButton());

    expect(toastWarning).toHaveBeenCalledWith(expect.objectContaining({ title: 'Draft kept' }));
    expect(setView).not.toHaveBeenCalled();
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(composer().value).toBe('/crew');
    expect(screen.getByTestId('resource-ref-chip')).toBeInTheDocument();
  });

  it('keeps a dropped file and draft, with no dispatch or queueing', () => {
    const { handleSubmit, setView } = renderComposer({
      initialValue: '/crew',
      droppedFiles: [droppedReport],
      chatState: ChatState.Streaming,
    });

    expect(screen.getByTitle('report.txt')).toBeInTheDocument();
    fireEvent.click(sendButton());

    expect(toastWarning).toHaveBeenCalledWith(expect.objectContaining({ title: 'Draft kept' }));
    expect(setView).not.toHaveBeenCalled();
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(composer().value).toBe('/crew');
    expect(screen.getByTitle('report.txt')).toBeInTheDocument();
  });

  it('keeps a pasted image and draft, with no dispatch', async () => {
    const { handleSubmit, setView } = renderComposer({ initialValue: '/crew' });
    const file = new File([new Uint8Array([137, 80, 78, 71])], 'shot.png', { type: 'image/png' });

    fireEvent.paste(composer(), {
      clipboardData: { files: [file], items: [], getData: () => '' },
    });
    await waitFor(() => expect(screen.getByAltText(/^Pasted image/)).toBeInTheDocument());
    fireEvent.click(sendButton());

    expect(toastWarning).toHaveBeenCalledWith(expect.objectContaining({ title: 'Draft kept' }));
    expect(setView).not.toHaveBeenCalled();
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(composer().value).toBe('/crew');
    expect(screen.getByAltText(/^Pasted image/)).toBeInTheDocument();
  });
});

describe('non-exact /crew text keeps ordinary submission behavior', () => {
  it('submits /crew prose to the configured model', () => {
    const { handleSubmit, setView } = renderComposer({ initialValue: '/crew please help' });

    fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter' });

    expect(setView).not.toHaveBeenCalled();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
    expect(submitValue(handleSubmit)).toBe('/crew please help');
  });

  it('retains the existing no-provider gate for /crew-like prose', () => {
    model.currentProvider = null;
    model.currentModel = null;
    const { handleSubmit, setView } = renderComposer({ initialValue: '/crew-like prose' });

    fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter' });

    expect(setView).not.toHaveBeenCalled();
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(sendButton()).toBeDisabled();
  });
});

describe('consumed /crew drafts do not resurrect after remount', () => {
  it('clears the command synchronously before the tab composer remounts', () => {
    const first = renderComposer({ initialValue: '/crew', draftKey: 'tab:crew-command' });
    const setView = first.setView;

    fireEvent.click(sendButton());
    expect(setView).toHaveBeenCalledWith('crew', { resumeSessionId: 'session-42' });
    expect(composer().value).toBe('');

    first.view.unmount();
    renderComposer({ initialValue: '', draftKey: 'tab:crew-command' });
    expect(composer().value).toBe('');
  });
});
