/**
 * The composer's shape (owner, 2026-11-13): a ONE-LINE card and ONE muted
 * footer line under it.
 *
 *   card    [queue] [notices] [chips]
 *           (+) text…                                   (mic) (Send)
 *   footer  folder                      model · effort   ring   cost
 *
 * The `+` holds attach, mention, commands and the three tool lists (there is no
 * Tools chip), and the model chip moved out of the card onto the footer line.
 * The tool lists and the footer's insides belong to `bottom_menu/`
 * (WS-PICKERS); they are stubbed by path here, so these tests are about where
 * the composer puts things, not what the pickers draw. The footer stub renders
 * the `model` slot `ChatInput` hands it, so the model chip's place is pinned
 * here and its order on the line in `ComposerFooter.test.tsx`.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
vi.mock('./bottom_menu/toolsLists', () => ({
  useToolsLists: () => ({ shown: null, clearQueries: () => {} }),
  ToolsListPanel: () => null,
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
  ComposerFooter: ({
    sessionId,
    workingDirLocked,
    model,
  }: {
    sessionId: string | null;
    workingDirLocked?: boolean;
    model?: React.ReactNode;
  }) => (
    <div
      data-testid="composer-footer"
      data-session={sessionId ?? 'none'}
      data-locked={String(workingDirLocked)}
    >
      {model}
    </div>
  ),
}));
vi.mock('./MessageQueue', () => ({ default: () => null }));
// A decision chat's question editor reads the daemon through the generated client;
// these tests are about where the composer puts things, so it is a stub here.
vi.mock('./decisions/DecisionComposer', () => ({
  DecisionComposer: () => <div data-testid="decision-composer-stub" />,
  DECISION_CHAT_PLACEHOLDER: 'Ask the decision model',
}));
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

function composerElement(overrides: Partial<React.ComponentProps<typeof ChatInput>> = {}) {
  return (
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
      toolCount={0}
      onWorkingDirChange={vi.fn()}
      {...overrides}
    />
  );
}

function renderComposer(
  overrides: Partial<React.ComponentProps<typeof ChatInput>> = {}
): ReturnType<typeof render> {
  return render(composerElement(overrides));
}

const card = () => document.querySelector('.biorouter-composer-card') as HTMLElement;
const row = () => within(card()).getByTestId('chat-input-toolbar');
const sendButton = () => screen.getByRole('button', { name: COMPOSER_COPY.send.label });

describe('the composer is a one-line card and one footer line', () => {
  it('holds +, the text, the microphone and Send in ONE row inside the card, Send last', () => {
    renderComposer();
    const controls = row();
    expect(controls).toHaveClass('br-composer-row');
    expect(card().contains(controls)).toBe(true);

    const plus = within(controls).getByTestId('composer-plus');
    const field = within(controls).getByTestId('chat-input');
    // The microphone, immediately left of Send (DD-60).
    const mic = within(controls).getByTestId('dictation-button');
    const send = within(controls).getByRole('button', { name: COMPOSER_COPY.send.label });

    // Four children, one per slot, in this order: nothing else shares the line.
    const children = [...controls.children] as HTMLElement[];
    expect(children).toHaveLength(4);
    expect(children[0]).toHaveClass('br-composer-row__slot');
    expect(children[0].contains(plus)).toBe(true);
    expect(children[1].tagName).toBe('FORM');
    expect(children[1]).toHaveClass('br-composer-row__field');
    expect(children[1].contains(field)).toBe(true);
    expect(children[2]).toHaveClass('br-composer-row__slot');
    expect(children[2].contains(mic)).toBe(true);
    // Send stays the row's last child: the footer inset and the geometry tests
    // are derived from its edge.
    expect(children[3]).toHaveClass('br-composer-row__slot');
    expect(children[3].contains(send)).toBe(true);
    // The text is the form Send submits, wherever the composer is drawn.
    expect(send).toHaveAttribute('form', children[1].id);
  });

  it('has no second row of controls: no Tools chip, and no model in the card', () => {
    renderComposer();
    expect(card().querySelector('.br-composer-controls')).toBeNull();
    expect(screen.queryByTestId('composer-tools-chip')).toBeNull();
    expect(within(card()).queryByTestId('model-chip-stub')).toBeNull();
  });

  it('puts the footer line under the card, outside it', () => {
    renderComposer();
    const footer = screen.getByTestId('composer-footer');
    expect(card().contains(footer)).toBe(false);
    expect(card().compareDocumentPosition(footer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('hands the model chip to the footer line, in a chat and on Home', () => {
    renderComposer();
    const chip = within(screen.getByTestId('composer-footer')).getByTestId('model-chip-stub');
    // Wrapped in the element "Switch model" finds it by.
    expect(chip.parentElement).toHaveClass('br-footline__model');
    cleanup();

    renderComposer({ sessionId: null });
    expect(
      within(screen.getByTestId('composer-footer')).getByTestId('model-chip-stub')
    ).toBeInTheDocument();
    expect(within(card()).queryByTestId('model-chip-stub')).toBeNull();
  });

  it('has no rows above the card any more', () => {
    renderComposer();
    expect(screen.queryByTestId('composer-context-banner')).toBeNull();
    expect(card().previousElementSibling).toBeNull();
  });

  it('is the page ground with a resting hairline, not a filled box (R-32)', () => {
    renderComposer();
    const classes = card().classList;
    expect(classes.contains('bg-background-default')).toBe(true);
    // One 1px hairline at rest: the focus and working edges turn it the accent.
    expect(classes.contains('border')).toBe(true);
    expect(classes.contains('border-border-subtle/60')).toBe(true);
    expect(classes.contains('border-transparent')).toBe(false);
    // No grey fill, and not a filled box: what sits in it rests as on any page.
    expect(classes.contains('bg-background-fill')).toBe(false);
    expect(classes.contains('br-fill-surface')).toBe(false);
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

describe('a decision chat (B-3)', () => {
  it('offers no +: a decision model attaches nothing and calls no tools', () => {
    renderComposer({ effectiveModel: { provider: 'ollama_decision', model: 'tev1:0.8b' } });
    const controls = row();
    // The `+` is the only way to attach, mention, run a command or switch tools,
    // so without it none of those is in the composer.
    expect(screen.queryByTestId('composer-plus')).toBeNull();
    expect(screen.queryByTestId('composer-file-input')).toBeNull();
    // The row says so, and the text starts the line.
    expect(controls).toHaveAttribute('data-plus', 'none');
    expect(controls.firstElementChild).toHaveClass('br-composer-row__field');
    expect(
      within(controls.firstElementChild as HTMLElement).getByTestId('chat-input')
    ).toBeInTheDocument();
    // Send stays in the row, and the model chip on the footer line names the
    // decision model.
    expect(within(controls).getByRole('button', { name: COMPOSER_COPY.send.label })).toBeTruthy();
    expect(
      within(screen.getByTestId('composer-footer')).getByTestId('model-chip-stub')
    ).toBeInTheDocument();
  });

  it('keeps the + in an ordinary chat, first on the line', () => {
    renderComposer({ effectiveModel: { provider: 'versa_azure', model: 'gpt-5.6-sol' } });
    const controls = row();
    expect(controls).not.toHaveAttribute('data-plus');
    expect(controls.firstElementChild?.contains(screen.getByTestId('composer-plus'))).toBe(true);
  });

  // Owner decision, 2026-11-12: a decision chat shows the microphone. Dictation is
  // input, not a model feature, so it stays immediately left of Send there too.
  it('keeps the microphone beside Send', () => {
    renderComposer({ effectiveModel: { provider: 'baseten_decision', model: 'joey-1' } });
    expect(screen.getByTestId('decision-composer-stub')).toBeInTheDocument();
    const controls = row();
    const send = within(controls).getByRole('button', { name: COMPOSER_COPY.send.label });
    const mic = within(controls).getByTestId('dictation-button');
    expect(controls.lastElementChild?.contains(send)).toBe(true);
    expect(controls.lastElementChild?.previousElementSibling?.contains(mic)).toBe(true);
  });
});

describe('the text field', () => {
  it('has a name that does not change with the placeholder', () => {
    renderComposer({ messagesLength: 3 });
    expect(screen.getByRole('textbox', { name: COMPOSER_COPY.inputLabel })).toBe(
      screen.getByTestId('chat-input')
    );
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

describe('followUp: the placeholder for a message the caller already shows', () => {
  // A new chat's first send draws the person's message before the store holds it
  // (summary/useFirstSend.ts), so for about 0.7 s the transcript has a message
  // and `messagesLength` is still 0.
  it('invites a follow-up while the store is still empty', () => {
    renderComposer({ messagesLength: 0, followUp: true });
    expect(screen.getByPlaceholderText(COMPOSER_COPY.placeholderFollowUp)).toBeInTheDocument();
    expect(screen.queryByPlaceholderText(COMPOSER_COPY.placeholderEmpty)).toBeNull();
  });

  it('changes once, with the message on screen, and not again when the store catches up', () => {
    const { rerender } = renderComposer({ messagesLength: 0, followUp: false });
    const field = screen.getByTestId('chat-input') as HTMLTextAreaElement;
    const seen = [field.placeholder];
    // The send: the row is drawn, the store is not yet holding it.
    rerender(composerElement({ messagesLength: 0, followUp: true }));
    seen.push(field.placeholder);
    // The store's message lands under the same stated word.
    rerender(composerElement({ messagesLength: 1, followUp: true }));
    seen.push(field.placeholder);
    expect(seen).toEqual([
      COMPOSER_COPY.placeholderEmpty,
      COMPOSER_COPY.placeholderFollowUp,
      COMPOSER_COPY.placeholderFollowUp,
    ]);
  });

  it('wins over the length when it is stated, in both directions', () => {
    const { unmount } = renderComposer({ messagesLength: 3, followUp: false });
    expect(screen.getByPlaceholderText(COMPOSER_COPY.placeholderEmpty)).toBeInTheDocument();
    unmount();
    renderComposer({ messagesLength: 0, followUp: true });
    expect(screen.getByPlaceholderText(COMPOSER_COPY.placeholderFollowUp)).toBeInTheDocument();
  });

  it('is the placeholder and nothing else: the working folder stays unlocked', () => {
    // `workingDirIsLocked` falls back to `messagesLength > 0`; a drawn row that the
    // store does not hold must not lock the folder of a chat that has no message.
    renderComposer({ messagesLength: 0, followUp: true });
    expect(screen.getByTestId('composer-footer')).toHaveAttribute('data-locked', 'false');
    cleanup();
    renderComposer({ messagesLength: 2 });
    expect(screen.getByTestId('composer-footer')).toHaveAttribute('data-locked', 'true');
  });
});

describe('Send and Stop', () => {
  it('is quiet while empty and the accent once there is something to send', () => {
    renderComposer();
    // The filled primitive's ground (R-27): on the card's page ground it rests
    // on the fill, as on any page.
    expect(sendButton().className).toContain('br-fill-rest');
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

    // "Switch model" opens the model chip's menu, which lives on the footer line.
    fireEvent.click(within(notice).getByRole('button', { name: COMPOSER_COPY.vision.action }));
    expect(modelMenuOpened).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(
      within(screen.getByTestId('composer-footer')).getByTestId('model-chip-stub')
    );
  });
});
