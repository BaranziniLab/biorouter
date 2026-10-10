/**
 * The composer's shape (spec 3.7, owner message 8): ONE card holding the text
 * and its controls, and ONE muted footer line under it.
 *
 *   card    [queue] [notices] [chips] text
 *           [+] [Tools]                 [model · effort] (Send)
 *   footer  folder                                  ring   cost
 *
 * The pickers inside the controls row and the footer's insides belong to
 * `bottom_menu/` (WS-PICKERS); they are stubbed by path here, so these tests
 * are about where the composer puts things, not what the pickers draw.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
const model = vi.hoisted(() => ({ supportsVision: true }));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: 'gpt-5.6-sol',
    currentProvider: 'versa_azure',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: model.supportsVision,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
const modelMenuOpened = vi.hoisted(() => vi.fn());
vi.mock('./bottom_menu/ToolsChip', () => ({
  ToolsChip: () => <button data-testid="tools-chip-stub">Tools</button>,
}));
vi.mock('./bottom_menu/ModelEffortChip', () => ({
  ModelEffortChip: () => (
    <button
      aria-haspopup="menu"
      data-testid="model-chip-stub"
      onKeyDown={(event) => {
        if (event.key === 'Enter') modelMenuOpened();
      }}
    >
      gpt-5.6-sol
    </button>
  ),
}));
vi.mock('./bottom_menu/ComposerFooter', () => ({
  ComposerFooter: ({ sessionId }: { sessionId: string | null }) => (
    <div data-testid="composer-footer" data-session={sessionId ?? 'none'} />
  ),
}));
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
import { COMPOSER_COPY } from './composer/copy';
import { resetAnnotationChannelForTests, sendArtifactAnnotation } from '../utils/annotationChannel';

beforeEach(() => {
  vi.clearAllMocks();
  resetAnnotationChannelForTests();
  model.supportsVision = true;
  Object.assign(window, {
    appConfig: { get: () => '/Users/wgu/Desktop' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn((file: File) => `/data/${file.name}`),
      on: vi.fn(),
      off: vi.fn(),
      saveDataUrlToTemp: vi.fn(async (_dataUrl: string, id: string) => ({
        id,
        filePath: `/tmp/biorouter-test/${id}.png`,
      })),
      readTempImageAsBase64: vi.fn(async () => ({ data: 'AAAA', mimeType: 'image/png' })),
      deleteTempFile: vi.fn(),
    },
  });
});

function renderComposer(
  overrides: Partial<React.ComponentProps<typeof ChatInput>> = {}
): ReturnType<typeof render> {
  return render(
    <ChatInput
      sessionId="chat-1"
      handleSubmit={vi.fn()}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      disableAnimation
      toolCount={0}
      onWorkingDirChange={vi.fn()}
      {...overrides}
    />
  );
}

const card = () => document.querySelector('.biorouter-composer-card') as HTMLElement;
const sendButton = () => screen.getByRole('button', { name: COMPOSER_COPY.send.label });

describe('the composer is one card and one footer line', () => {
  it('holds the text and every control inside the card, Send last', () => {
    renderComposer();
    const controls = within(card()).getByTestId('chat-input-toolbar');
    expect(within(card()).getByTestId('chat-input')).toBeInTheDocument();

    const order = [
      within(controls).getByTestId('composer-plus'),
      within(controls).getByTestId('tools-chip-stub'),
      within(controls).getByTestId('model-chip-stub'),
      within(controls).getByRole('button', { name: COMPOSER_COPY.send.label }),
    ];
    for (let i = 1; i < order.length; i++) {
      expect(
        order[i - 1].compareDocumentPosition(order[i]) & Node.DOCUMENT_POSITION_FOLLOWING
      ).toBeTruthy();
    }
  });

  it('puts the footer line under the card, outside it', () => {
    renderComposer();
    const footer = screen.getByTestId('composer-footer');
    expect(card().contains(footer)).toBe(false);
    expect(card().compareDocumentPosition(footer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('has no rows above the card any more', () => {
    renderComposer();
    expect(screen.queryByTestId('composer-context-banner')).toBeNull();
    expect(card().previousElementSibling).toBeNull();
  });

  it('uses no monospace anywhere in the composer', () => {
    const { container } = renderComposer();
    expect(container.querySelector('.font-mono')).toBeNull();
  });

  it('tells the footer whether there is a chat, so Home shows no ring or cost', () => {
    renderComposer({ sessionId: null });
    expect(screen.getByTestId('composer-footer')).toHaveAttribute('data-session', 'none');
  });
});

describe('placeholders', () => {
  it('asks the open question in an empty chat', () => {
    renderComposer({ messagesLength: 0 });
    expect(screen.getByPlaceholderText(COMPOSER_COPY.placeholderEmpty)).toBeInTheDocument();
  });

  it('invites a follow-up in a chat with messages, never a shortcut hint', () => {
    renderComposer({ messagesLength: 3 });
    expect(screen.getByPlaceholderText(COMPOSER_COPY.placeholderFollowUp)).toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/navigate messages/)).toBeNull();
  });
});

describe('Send and Stop', () => {
  it('is quiet while empty and the accent once there is something to send', () => {
    renderComposer();
    expect(sendButton().className).toContain('bg-background-medium');
    fireEvent.change(screen.getByTestId('chat-input'), { target: { value: 'hello' } });
    expect(sendButton().className).toContain('bg-background-accent');
  });

  it('swaps to a Stop with the same geometry and no native title or ping', () => {
    renderComposer({ chatState: ChatState.Streaming });
    const stop = screen.getByTestId('chat-stop-button');
    expect(stop).toHaveAccessibleName(COMPOSER_COPY.stop.label);
    expect(stop).not.toHaveAttribute('title');
    expect(stop.className).toContain('h-control-md');
    expect(stop.querySelector('.animate-ping')).toBeNull();
  });

  it('shows a spinner in Send while a chat is being started, not a Stop', () => {
    renderComposer({ sessionId: null, chatState: ChatState.LoadingConversation });
    expect(screen.queryByTestId('chat-stop-button')).toBeNull();
    expect(sendButton()).toHaveAttribute('aria-busy', 'true');
    expect(sendButton().querySelector('.br-spinner')).not.toBeNull();
  });
});

describe('the + menu', () => {
  it('offers attach, mention and commands', async () => {
    const user = userEvent.setup();
    renderComposer();
    await user.click(screen.getByRole('button', { name: COMPOSER_COPY.plus.label }));
    expect(
      await screen.findByRole('menuitem', { name: COMPOSER_COPY.plus.attachFile })
    ).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /^Mention/ })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /^Commands/ })).toBeInTheDocument();
  });

  it('starts a mention at the caret', async () => {
    const user = userEvent.setup();
    renderComposer();
    const box = screen.getByTestId('chat-input') as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: 'compare' } });
    await user.click(screen.getByRole('button', { name: COMPOSER_COPY.plus.label }));
    await user.click(await screen.findByRole('menuitem', { name: /^Mention/ }));
    await waitFor(() => expect(box.value).toBe('compare @'));
  });

  it('starts a command at the start of a word', async () => {
    const user = userEvent.setup();
    renderComposer();
    const box = screen.getByTestId('chat-input') as HTMLTextAreaElement;
    await user.click(screen.getByRole('button', { name: COMPOSER_COPY.plus.label }));
    await user.click(await screen.findByRole('menuitem', { name: /^Commands/ }));
    await waitFor(() => expect(box.value).toBe('/'));
  });

  it('attaches a picked file the way a drop does', async () => {
    renderComposer();
    const input = screen.getByTestId('composer-file-input') as HTMLInputElement;
    const file = new File(['a,b'], 'cohort.csv', { type: 'text/csv' });
    await act(async () => {
      fireEvent.change(input, { target: { files: [file] } });
    });
    expect(await screen.findByTestId('composer-file-chip')).toHaveTextContent('cohort.csv');
  });
});

describe('attachments', () => {
  it('sit in one chips row above the text', async () => {
    renderComposer({
      droppedFiles: [
        {
          id: 'f1',
          path: '/data/results.csv',
          sourcePath: '/data/results.csv',
          name: 'results.csv',
          type: 'text/csv',
          isImage: false,
          canUploadAsImage: false,
          isLoading: false,
        },
      ],
    });
    const chips = await screen.findByTestId('composer-chips');
    const text = screen.getByTestId('chat-input');
    expect(chips.compareDocumentPosition(text) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(chips).getByTestId('composer-file-chip')).toHaveTextContent('results.csv');
  });

  it('draws a loading image under a translucent scrim, never solid black', async () => {
    renderComposer({
      droppedFiles: [
        {
          id: 'img1',
          path: '/data/plot.png',
          sourcePath: '/data/plot.png',
          name: 'plot.png',
          type: 'image/png',
          isImage: true,
          canUploadAsImage: true,
          dataUrl: 'data:image/png;base64,AAAA',
          isLoading: true,
        },
      ],
    });
    const thumb = await screen.findByTestId('composer-image-thumb');
    expect(thumb.querySelector('.br-composer-thumb__scrim')).not.toBeNull();
    expect(thumb.querySelector('.br-spinner')).not.toBeNull();
    expect(thumb.innerHTML).not.toMatch(/bg-black|bg-opacity/);
  });
});

describe('notices inside the card', () => {
  it('says the model cannot read images in one line, with a way to switch model', async () => {
    model.supportsVision = false;
    renderComposer();
    // A region from the preview panel stages an image whatever the model can
    // read, which is how an image meets a model that cannot read it.
    await act(async () => {
      sendArtifactAnnotation({
        sessionId: 'chat-1',
        imagePath: '/tmp/biorouter-test/region.png',
        sourceTitle: 'figure.html',
        region: { x: 0, y: 0, width: 10, height: 10, surfaceWidth: 100, surfaceHeight: 100 },
        width: 10,
        height: 10,
      });
    });

    const notice = await screen.findByTestId('composer-vision-notice');
    expect(card().contains(notice)).toBe(true);
    expect(notice).toHaveTextContent(COMPOSER_COPY.vision.banner);
    // The explanation lives in the help, not in a paragraph.
    expect(
      within(notice).getByRole('button', { name: `About ${COMPOSER_COPY.vision.banner}` })
    ).toHaveAccessibleDescription(COMPOSER_COPY.vision.help);
    expect(sendButton()).toBeDisabled();

    fireEvent.click(within(notice).getByRole('button', { name: COMPOSER_COPY.vision.action }));
    expect(modelMenuOpened).toHaveBeenCalledTimes(1);
  });
});
