import { describe, it, expect, vi, beforeEach } from 'vitest';
import { liveContextWindow } from './liveContextWindow';

const mocks = vi.hoisted(() => ({
  llamacppStatus: vi.fn(),
  ollamaContextWindow: vi.fn(),
}));

vi.mock('../api', () => ({
  llamacppStatus: mocks.llamacppStatus,
  ollamaContextWindow: mocks.ollamaContextWindow,
}));

describe('liveContextWindow', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  /**
   * The gauge used to take an Ollama model's window from its NAME ("qwen" read
   * as 128k). It now asks the daemon for the `num_ctx` the provider sends.
   */
  it('asks the daemon for the window Ollama is sent for this model', async () => {
    mocks.ollamaContextWindow.mockResolvedValue({
      data: { model: 'qwen3.8:27b', context_window: 65536 },
    });

    expect(await liveContextWindow('ollama', 'qwen3.8:27b')).toBe(65536);
    expect(mocks.ollamaContextWindow).toHaveBeenCalledWith({ query: { model: 'qwen3.8:27b' } });
    expect(mocks.llamacppStatus).not.toHaveBeenCalled();
  });

  it('falls back when Ollama could not describe the model', async () => {
    mocks.ollamaContextWindow.mockResolvedValue({
      data: { model: 'not-pulled:7b', context_window: null },
    });
    expect(await liveContextWindow('ollama', 'not-pulled:7b')).toBeNull();
  });

  it('falls back when the daemon errors', async () => {
    mocks.ollamaContextWindow.mockResolvedValue({ data: undefined, error: 'model is required' });
    expect(await liveContextWindow('ollama', 'qwen3.8:27b')).toBeNull();

    mocks.ollamaContextWindow.mockRejectedValue(new Error('offline'));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(await liveContextWindow('ollama', 'qwen3.8:27b')).toBeNull();
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('reads the window the Llama Server sidecar allocated', async () => {
    mocks.llamacppStatus.mockResolvedValue({ data: { sidecar: { context_size: 131072 } } });
    expect(await liveContextWindow('llamacpp', 'qwen3.8-27b')).toBe(131072);

    mocks.llamacppStatus.mockResolvedValue({ data: { sidecar: { context_size: null } } });
    expect(await liveContextWindow('llamacpp', 'qwen3.8-27b')).toBeNull();
  });

  it('asks nothing for a hosted provider', async () => {
    expect(await liveContextWindow('anthropic', 'claude-opus-5')).toBeNull();
    expect(mocks.llamacppStatus).not.toHaveBeenCalled();
    expect(mocks.ollamaContextWindow).not.toHaveBeenCalled();
  });
});
