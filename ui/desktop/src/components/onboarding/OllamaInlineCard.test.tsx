import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import OllamaInlineCard from './OllamaInlineCard';
import { ollamaCopy } from './copy';

const mocks = vi.hoisted(() => ({
  checkOllamaStatus: vi.fn(),
  hasModel: vi.fn(),
  upsert: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

// Never the real Ollama on 11434: what comes back would depend on the developer's machine.
vi.mock('../../utils/ollamaDetection', () => ({
  checkOllamaStatus: mocks.checkOllamaStatus,
  hasModel: mocks.hasModel,
  getOllamaDownloadUrl: () => 'https://ollama.com/download',
  pollForOllama: vi.fn(() => () => {}),
  pullOllamaModel: vi.fn(async () => false),
  getPreferredModel: () => 'gpt-oss:20b',
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ upsert: mocks.upsert }),
}));

vi.mock('../../toasts', () => ({
  toastService: { success: mocks.toastSuccess, error: mocks.toastError },
}));

/**
 * Ollama's panel on the first-run standard (R-34; vision FR-S-8, FR-S-11): sentence-case toasts
 * with no exclamation mark, buttons that say what they do on the 32px rung and are never stretched,
 * and no machine id in a button's label.
 */
describe('OllamaInlineCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.upsert.mockResolvedValue(undefined);
  });

  it('offers to install Ollama when it is not running, as an ordinary button', async () => {
    mocks.checkOllamaStatus.mockResolvedValue({ isRunning: false });
    render(<OllamaInlineCard onSuccess={vi.fn()} chrome="bare" />);
    const install = await screen.findByRole('link', { name: ollamaCopy.install });
    expect(install.className).not.toContain('w-full');
    expect(install.className).not.toContain('h-control-lg');
    expect(screen.getByText(ollamaCopy.notDetected)).toBeInTheDocument();
  });

  it('says what a download is without naming the model id on the button', async () => {
    mocks.checkOllamaStatus.mockResolvedValue({ isRunning: true });
    mocks.hasModel.mockResolvedValue(false);
    render(<OllamaInlineCard onSuccess={vi.fn()} chrome="bare" />);
    const download = await screen.findByRole('button', { name: ollamaCopy.downloadModel });
    expect(download).not.toHaveTextContent('gpt-oss');
    // The model is named once, on the status line.
    expect(
      screen.getByText(ollamaCopy.running('gpt-oss:20b', 'not-available'))
    ).toBeInTheDocument();
  });

  it('connects with a plain, sentence-case toast', async () => {
    mocks.checkOllamaStatus.mockResolvedValue({ isRunning: true });
    mocks.hasModel.mockResolvedValue(true);
    const onSuccess = vi.fn();
    render(<OllamaInlineCard onSuccess={onSuccess} chrome="bare" />);
    fireEvent.click(await screen.findByRole('button', { name: ollamaCopy.use }));
    await waitFor(() => expect(onSuccess).toHaveBeenCalled());
    const [toast] = mocks.toastSuccess.mock.calls[0] as [{ title: string }];
    expect(toast.title).toBe(ollamaCopy.connectedToast);
    for (const title of [
      ollamaCopy.connectedToast,
      ollamaCopy.detectedToast,
      ollamaCopy.downloadedToast,
    ]) {
      expect(title).not.toMatch(/!/);
    }
  });
});
