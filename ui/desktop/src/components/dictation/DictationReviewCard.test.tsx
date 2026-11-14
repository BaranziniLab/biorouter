import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DICTATION_ACTIONS, dictationMessage } from './copy';
import DictationReviewCard from './DictationReviewCard';

type Review = {
  text?: string;
  message?: string;
  confirmPrivate?: boolean;
  retry?: boolean;
  expiresAt: number | null;
  revision: number;
};

const FAILED = 'Biorouter could not finish dictation. Try again.';
const KEPT = 'The clipboard did not keep the text. Copy it from this card.';

let push: (review: Review) => void;
let electron: {
  platform: string;
  onDictationAnywhereReview: (callback: (review: Review) => void) => () => void;
  dictationAnywhereReviewGet: ReturnType<typeof vi.fn>;
  dictationAnywhereReviewRetry: ReturnType<typeof vi.fn>;
  dictationAnywhereReviewCopy: ReturnType<typeof vi.fn>;
  dictationAnywherePrivateChoice: ReturnType<typeof vi.fn>;
};

const settle = async () => {
  for (let i = 0; i < 4; i++) await act(async () => {});
};

beforeEach(() => {
  electron = {
    platform: 'darwin',
    onDictationAnywhereReview: (callback) => {
      push = callback;
      return () => undefined;
    },
    dictationAnywhereReviewGet: vi.fn(async () => null),
    dictationAnywhereReviewRetry: vi.fn(async () => true),
    dictationAnywhereReviewCopy: vi.fn(async () => true),
    dictationAnywherePrivateChoice: vi.fn(async () => true),
  };
  Object.assign(window, { electron });
});

afterEach(() => cleanup());

describe('the Dictate to clipboard review card', () => {
  it('offers Retry only while main holds the failed recording', async () => {
    render(<DictationReviewCard />);
    await settle();
    act(() => push({ message: FAILED, retry: true, expiresAt: null, revision: 1 }));
    expect(screen.getByRole('status')).toHaveTextContent(FAILED);
    fireEvent.click(screen.getByRole('button', { name: DICTATION_ACTIONS.retry }));
    expect(electron.dictationAnywhereReviewRetry).toHaveBeenCalledTimes(1);

    // The hold ended: the same sentence, without Retry.
    act(() => push({ message: FAILED, expiresAt: null, revision: 2 }));
    expect(screen.getByRole('status')).toHaveTextContent(FAILED);
    expect(screen.queryByRole('button', { name: DICTATION_ACTIONS.retry })).toBeNull();
  });

  it('says so when its Copy does not reach the clipboard either', async () => {
    electron.dictationAnywhereReviewCopy.mockResolvedValue(false);
    render(<DictationReviewCard />);
    await settle();
    const expiresAt = Date.now() + 60_000;
    act(() => push({ message: KEPT, text: 'EPAS1 variant', expiresAt, revision: 1 }));
    const sentence = dictationMessage('MSG-CLIPBOARD-COPY-FAILED', { key: '⌘C' });
    expect(screen.queryByText(sentence)).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await settle();
    expect(electron.dictationAnywhereReviewCopy).toHaveBeenCalledTimes(1);
    expect(screen.getByText(sentence)).toBeInTheDocument();

    // A copy that lands takes the sentence away, and so does a new review.
    electron.dictationAnywhereReviewCopy.mockResolvedValue(true);
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await settle();
    expect(screen.queryByText(sentence)).toBeNull();
    electron.dictationAnywhereReviewCopy.mockResolvedValue(false);
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await settle();
    expect(screen.getByText(sentence)).toBeInTheDocument();
    act(() => push({ message: KEPT, text: 'EPAS1 variant', expiresAt, revision: 2 }));
    expect(screen.queryByText(sentence)).toBeNull();
  });

  it('names the copy key for Windows and Linux', async () => {
    electron.platform = 'win32';
    electron.dictationAnywhereReviewCopy.mockRejectedValue(new Error('no answer'));
    render(<DictationReviewCard />);
    await settle();
    act(() => push({ message: KEPT, text: 'EPAS1', expiresAt: Date.now() + 60_000, revision: 1 }));
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await settle();
    expect(
      screen.getByText(dictationMessage('MSG-CLIPBOARD-COPY-FAILED', { key: 'Ctrl+C' }))
    ).toBeInTheDocument();
  });
});
