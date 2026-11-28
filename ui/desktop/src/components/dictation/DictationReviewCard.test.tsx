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

type Appearance = { family: string; mode: 'light' | 'dark' };

let push: (review: Review) => void;
let pushAppearance: ((appearance: Appearance) => void) | null;
let electron: {
  platform: string;
  onDictationAnywhereReview: (callback: (review: Review) => void) => () => void;
  dictationAnywhereReviewGet: ReturnType<typeof vi.fn>;
  dictationAnywhereReviewRetry: ReturnType<typeof vi.fn>;
  dictationAnywhereReviewCopy: ReturnType<typeof vi.fn>;
  dictationAnywherePrivateChoice: ReturnType<typeof vi.fn>;
  getThemeAppearance?: ReturnType<typeof vi.fn>;
  onThemeAppearance?: (callback: (appearance: Appearance) => void) => () => void;
  setThemeAppearance?: ReturnType<typeof vi.fn>;
};

const settle = async () => {
  for (let i = 0; i < 4; i++) await act(async () => {});
};

beforeEach(() => {
  pushAppearance = null;
  document.documentElement.classList.remove('dark', 'light');
  document.documentElement.removeAttribute('data-theme');
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

afterEach(() => {
  cleanup();
  document.documentElement.classList.remove('dark', 'light');
  document.documentElement.removeAttribute('data-theme');
});

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

// The card's window may run on a session whose storage holds no theme (no chat window open), so
// index.html stamps Parchment and the system mode. It wears main's appearance instead (DD-99).
describe('the review card wears the person’s theme', () => {
  const root = () => document.documentElement;

  beforeEach(() => {
    root().classList.add('light');
    root().setAttribute('data-theme', 'parchment');
    electron.getThemeAppearance = vi.fn(async () => ({ family: 'alma-mater', mode: 'dark' }));
    electron.onThemeAppearance = (callback) => {
      pushAppearance = callback;
      return () => {
        pushAppearance = null;
      };
    };
    electron.setThemeAppearance = vi.fn();
  });

  it('takes main’s appearance when it opens, and follows a change while it is open', async () => {
    render(<DictationReviewCard />);
    await settle();
    expect(electron.getThemeAppearance).toHaveBeenCalledTimes(1);
    expect(root().getAttribute('data-theme')).toBe('alma-mater');
    expect(root().classList.contains('dark')).toBe(true);
    expect(root().classList.contains('light')).toBe(false);

    act(() => pushAppearance?.({ family: 'roche-limit', mode: 'light' }));
    expect(root().getAttribute('data-theme')).toBe('roche-limit');
    expect(root().classList.contains('light')).toBe(true);
    expect(root().classList.contains('dark')).toBe(false);

    // The card shows main's choice and never makes one of its own.
    expect(electron.setThemeAppearance).not.toHaveBeenCalled();
  });

  it('keeps the review working when main has no appearance yet', async () => {
    electron.getThemeAppearance = vi.fn(async () => null);
    render(<DictationReviewCard />);
    await settle();
    expect(root().getAttribute('data-theme')).toBe('parchment');
    expect(root().classList.contains('light')).toBe(true);
    act(() => push({ message: KEPT, text: 'EPAS1', expiresAt: Date.now() + 60_000, revision: 1 }));
    expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument();
  });

  it('stops following main once it closes', async () => {
    const view = render(<DictationReviewCard />);
    await settle();
    expect(pushAppearance).not.toBeNull();
    view.unmount();
    expect(pushAppearance).toBeNull();
  });
});
