import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

// A dropped file that could not be located must SAY SO in the composer.
//
// Only the image branch of the attachment chip rendered `error`; a document
// with one rendered as an ordinary chip showing its MIME type, so the user
// attached a file that would never be read and had no way to tell. That is the
// display half of the browser-mode path defect -- the data half lives in
// `useFileDrop`.

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
// The pickers and the footer are WS-PICKERS' modules, mocked by path; their
// insides are tested where they live.
vi.mock('./bottom_menu/ToolsChip', () => ({ ToolsChip: () => null }));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
vi.mock('./bottom_menu/ComposerFooter', () => ({ ComposerFooter: () => null }));
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
import type { DroppedFile } from '../hooks/useFileDrop';
import { ChatState } from '../types/chatState';

beforeEach(() => {
  vi.clearAllMocks();
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

const erroredFile: DroppedFile = {
  id: 'dropped-nopath-1',
  path: '',
  name: 'results.csv',
  type: 'text/csv',
  isImage: false,
  canUploadAsImage: false,
  isLoading: false,
  error: 'This file cannot be read from a browser tab. Biorouter runs on another machine here.',
};

const okFile: DroppedFile = {
  id: 'dropped-ok-1',
  path: '/data/results.csv',
  sourcePath: '/data/results.csv',
  name: 'results.csv',
  type: 'text/csv',
  isImage: false,
  canUploadAsImage: false,
  isLoading: false,
};

const renderWithFiles = (droppedFiles: DroppedFile[]) => {
  render(
    <ChatInput
      sessionId="session-1"
      handleSubmit={vi.fn()}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={droppedFiles}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
};

describe('an attachment that could not be located', () => {
  /// Fails against the previous component, which rendered the file's MIME type
  /// here and showed `error` only for images. Measured, not assumed: reverting
  /// the chip fails this and the next, and leaves the control below passing.
  it('shows the reason on the chip instead of looking ordinary', async () => {
    renderWithFiles([erroredFile]);
    await waitFor(() => {
      expect(screen.getByText(/cannot be read from a browser tab/i)).toBeTruthy();
    });
  });

  it('does not show the harmless-looking type line in its place', async () => {
    renderWithFiles([erroredFile]);
    await waitFor(() => {
      expect(screen.getByText(/cannot be read from a browser tab/i)).toBeTruthy();
    });
    expect(screen.queryByText('text/csv')).toBeNull();
  });

  /// The control: a fix that marked every attachment broken would satisfy both
  /// assertions above and fail this one.
  it('leaves a file that does have a path alone', async () => {
    renderWithFiles([okFile]);
    await waitFor(() => {
      expect(screen.getByTestId('composer-file-chip')).toHaveTextContent('results.csv');
    });
    expect(screen.queryByText(/cannot be read/i)).toBeNull();
    expect(screen.queryByTestId('composer-failed-chip')).toBeNull();
  });

  it('marks the failed file in danger ink and explains it in its help', async () => {
    renderWithFiles([erroredFile]);
    const chip = await screen.findByTestId('composer-failed-chip');
    expect(chip).toHaveTextContent('results.csv');
    expect(chip.className).toContain('text-text-danger');
    expect(screen.getByRole('button', { name: 'About results.csv' })).toHaveAccessibleDescription(
      /cannot be read from a browser tab/i
    );
  });
});
