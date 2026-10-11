/* eslint-disable @typescript-eslint/no-explicit-any */
/* global EventListener */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  checkOllamaStatus,
  getOllamaModels,
  hasModel,
  pullOllamaModel,
  deleteOllamaModel,
  pollForOllama,
  getOllamaDownloadUrl,
  getOllamaSetup,
  rendererOllamaUrl,
  isRendererReachable,
  sameOllamaTag,
  ollamaTagsForModelFile,
} from './ollamaDetection';

const mockOllamaRecommended = vi.fn();
vi.mock('../api', () => ({
  ollamaRecommended: (...args: unknown[]) => mockOllamaRecommended(...args),
}));

// Mock fetch globally
globalThis.fetch = vi.fn();

/** A fetch Response whose body streams `chunks` (strings), one per read. */
const streamResponse = (chunks: string[]) => {
  const read = vi.fn();
  for (const chunk of chunks) {
    read.mockResolvedValueOnce({ done: false, value: new TextEncoder().encode(chunk) });
  }
  read.mockResolvedValue({ done: true });
  return { ok: true, body: { getReader: () => ({ read, cancel: vi.fn() }) } };
};

/** `/api/tags` listing `names`. */
const tagsResponse = (names: string[]) => ({
  ok: true,
  json: async () => ({
    models: names.map((name) => ({ name, model: name, size: 1, digest: 'd', modified_at: '' })),
  }),
});

const line = (value: Record<string, unknown>) => JSON.stringify(value) + '\n';

// Define global objects for testing environment if they don't exist
if (typeof globalThis.AbortSignal === 'undefined') {
  globalThis.AbortSignal = class AbortSignal {
    aborted = false;
    reason: any = undefined;
    onabort: ((this: AbortSignal, ev: Event) => any) | null = null;

    addEventListener(_type: string, _listener: EventListener): void {
      // Mock implementation
    }

    removeEventListener(_type: string, _listener: EventListener): void {
      // Mock implementation
    }

    dispatchEvent(_event: Event): boolean {
      return true;
    }
  } as any;
}

if (typeof globalThis.TextEncoder === 'undefined') {
  globalThis.TextEncoder = class TextEncoder {
    encode(str: string): Uint8Array {
      return new Uint8Array(str.split('').map((c) => c.charCodeAt(0)));
    }
  } as any;
}

describe('ollamaDetection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('checkOllamaStatus', () => {
    it('should return isRunning: true when Ollama is accessible', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: true,
        json: async () => ({ models: [] }),
      });

      const result = await checkOllamaStatus();

      expect(result).toEqual({
        isRunning: true,
        host: 'http://127.0.0.1:11434',
      });
      expect(globalThis.fetch).toHaveBeenCalledWith('http://127.0.0.1:11434/api/tags', {
        method: 'GET',
        signal: expect.any(globalThis.AbortSignal),
      });
    });

    it('should return isRunning: false when Ollama is not accessible', async () => {
      (globalThis.fetch as any).mockRejectedValueOnce(new Error('Connection refused'));

      const result = await checkOllamaStatus();

      expect(result).toEqual({
        isRunning: false,
        host: 'http://127.0.0.1:11434',
        error: 'Connection refused',
      });
    });

    it('should timeout after 2 seconds', async () => {
      let abortSignal: AbortSignal | undefined;

      (globalThis.fetch as any).mockImplementationOnce((_url: string, options: any) => {
        abortSignal = options.signal;
        return new Promise((_, reject) => {
          // Listen for abort signal
          options.signal.addEventListener('abort', () => {
            reject(new Error('The operation was aborted'));
          });
        });
      });

      const checkPromise = checkOllamaStatus();

      // Fast-forward 2 seconds
      vi.advanceTimersByTime(2000);

      // The abort signal should be triggered
      expect(abortSignal?.aborted).toBe(true);

      const result = await checkPromise;
      expect(result.isRunning).toBe(false);
      expect(result.error).toBe('The operation was aborted');
    });
  });

  describe('getOllamaModels', () => {
    it('should return models when API call is successful', async () => {
      const mockModels = [
        { name: 'llama2:latest', size: 4733363377, digest: 'abc123', modified_at: '2023-10-01' },
        { name: 'qwen3.8:27b', size: 13780173839, digest: 'def456', modified_at: '2023-10-02' },
      ];

      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: true,
        json: async () => ({ models: mockModels }),
      });

      const result = await getOllamaModels();

      expect(result).toEqual(mockModels);
    });

    it('should return empty array when API call fails', async () => {
      (globalThis.fetch as any).mockRejectedValueOnce(new Error('Network error'));

      const result = await getOllamaModels();

      expect(result).toEqual([]);
    });

    it('should handle non-ok responses', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: false,
        statusText: 'Not Found',
      });

      const result = await getOllamaModels();

      expect(result).toEqual([]);
    });
  });

  describe('hasModel', () => {
    it('should return true when model exists', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          models: [
            {
              name: 'llama2:latest',
              size: 4733363377,
              digest: 'abc123',
              modified_at: '2023-10-01',
            },
            { name: 'qwen3.8:27b', size: 13780173839, digest: 'def456', modified_at: '2023-10-02' },
          ],
        }),
      });

      const result = await hasModel('qwen3.8:27b');

      expect(result).toBe(true);
    });

    it('should return false when model does not exist', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          models: [
            {
              name: 'llama2:latest',
              size: 4733363377,
              digest: 'abc123',
              modified_at: '2023-10-01',
            },
          ],
        }),
      });

      const result = await hasModel('qwen3.8:27b');

      expect(result).toBe(false);
    });
  });

  describe('pullOllamaModel', () => {
    it('should successfully pull a model and report progress', async () => {
      const progressUpdates: any[] = [];
      const onProgress = vi.fn((progress) => progressUpdates.push(progress));

      (globalThis.fetch as any)
        .mockResolvedValueOnce(
          streamResponse([
            line({ status: 'downloading', completed: 100, total: 1000 }),
            line({ status: 'downloading', completed: 500, total: 1000 }),
            line({ status: 'success' }),
          ])
        )
        .mockResolvedValueOnce(tagsResponse(['qwen3.8:27b']));

      const result = await pullOllamaModel('qwen3.8:27b', onProgress);

      expect(result).toEqual({ ok: true });
      expect(globalThis.fetch).toHaveBeenCalledWith(
        'http://127.0.0.1:11434/api/pull',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({ model: 'qwen3.8:27b', stream: true }),
        })
      );
      // Verified against the model list afterwards, not taken on trust.
      expect(globalThis.fetch).toHaveBeenLastCalledWith(
        'http://127.0.0.1:11434/api/tags',
        expect.anything()
      );
      expect(onProgress).toHaveBeenCalledTimes(3);
      expect(progressUpdates).toContainEqual({
        status: 'downloading',
        completed: 100,
        total: 1000,
      });
      expect(progressUpdates).toContainEqual({
        status: 'downloading',
        completed: 500,
        total: 1000,
      });
      expect(progressUpdates).toContainEqual({ status: 'success' });
    });

    /**
     * The reported bug: Ollama answers 200, streams progress, then writes an
     * `error` line and closes. That used to read as "Model Downloaded!".
     */
    it("fails on an error line in a 200 stream, with Ollama's words", async () => {
      (globalThis.fetch as any).mockResolvedValueOnce(
        streamResponse([
          line({ status: 'pulling manifest' }),
          line({ status: 'downloading', completed: 5, total: 10 }),
          line({ error: 'max retries exceeded: unexpected EOF' }),
        ])
      );

      const result = await pullOllamaModel('qwen3.8:27b');

      expect(result).toEqual({ ok: false, error: 'max retries exceeded: unexpected EOF' });
      // No verification round trip after a failure the stream already named.
      expect(globalThis.fetch).toHaveBeenCalledTimes(1);
    });

    it('fails when the stream ends without a final success', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce(
        streamResponse([line({ status: 'downloading', completed: 5, total: 10 })])
      );

      const result = await pullOllamaModel('qwen3.8:27b');

      expect(result.ok).toBe(false);
      expect(!result.ok && result.error).toMatch(/last status: downloading/);
    });

    it('fails when Ollama says success but the model is not listed', async () => {
      (globalThis.fetch as any)
        .mockResolvedValueOnce(streamResponse([line({ status: 'success' })]))
        .mockResolvedValueOnce(tagsResponse(['llama3.1:8b']));

      const result = await pullOllamaModel('qwen3.8:27b');

      expect(result.ok).toBe(false);
      expect(!result.ok && result.error).toMatch(/not in its model list/);
    });

    /**
     * A read can end mid-line. Parsing chunk by chunk threw a split line away
     * whole — and when the split landed in the last line, the `success`.
     */
    it('reassembles NDJSON lines split across reads', async () => {
      const onProgress = vi.fn();
      const first = line({ status: 'downloading', completed: 1, total: 2 });
      const last = line({ status: 'success' });
      (globalThis.fetch as any)
        .mockResolvedValueOnce(
          streamResponse([first.slice(0, 7), first.slice(7) + last.slice(0, 5), last.slice(5)])
        )
        .mockResolvedValueOnce(tagsResponse(['qwen3.8:27b']));

      const result = await pullOllamaModel('qwen3.8:27b', onProgress);

      expect(result).toEqual({ ok: true });
      expect(onProgress).toHaveBeenCalledWith({ status: 'downloading', completed: 1, total: 2 });
      expect(onProgress).toHaveBeenCalledWith({ status: 'success' });
    });

    it('accepts a final success line with no trailing newline', async () => {
      (globalThis.fetch as any)
        .mockResolvedValueOnce(streamResponse([JSON.stringify({ status: 'success' })]))
        .mockResolvedValueOnce(tagsResponse(['qwen3.8:27b']));

      expect(await pullOllamaModel('qwen3.8:27b')).toEqual({ ok: true });
    });

    it('reports the error body of a refused pull', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: false,
        status: 412,
        statusText: 'Precondition Failed',
        text: async () =>
          JSON.stringify({ error: 'this model requires a newer version of Ollama' }),
      });

      const result = await pullOllamaModel('qwen3.8:27b');

      expect(result).toEqual({
        ok: false,
        error: 'this model requires a newer version of Ollama',
      });
    });

    it('should fail on API error', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: false,
        status: 404,
        statusText: 'Model not found',
        text: async () => '',
      });

      const result = await pullOllamaModel('invalid-model');

      expect(result).toEqual({ ok: false, error: 'HTTP 404 Model not found' });
    });

    it('pulls from and verifies against the host it is given', async () => {
      (globalThis.fetch as any)
        .mockResolvedValueOnce(streamResponse([line({ status: 'success' })]))
        .mockResolvedValueOnce(tagsResponse(['gemma4:31b']));

      const result = await pullOllamaModel('gemma4:31b', undefined, 'http://127.0.0.1:11500');

      expect(result).toEqual({ ok: true });
      expect(globalThis.fetch).toHaveBeenNthCalledWith(
        1,
        'http://127.0.0.1:11500/api/pull',
        expect.anything()
      );
      expect(globalThis.fetch).toHaveBeenNthCalledWith(
        2,
        'http://127.0.0.1:11500/api/tags',
        expect.anything()
      );
    });
  });

  describe('deleteOllamaModel', () => {
    it('should delete a model through the Ollama API', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: true,
      });

      const result = await deleteOllamaModel('gemma4:31b');

      expect(result).toBe(true);
      expect(globalThis.fetch).toHaveBeenCalledWith(
        'http://127.0.0.1:11434/api/delete',
        expect.objectContaining({
          method: 'DELETE',
          body: JSON.stringify({ model: 'gemma4:31b' }),
        })
      );
    });

    it('should return false when delete fails', async () => {
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: false,
        statusText: 'Model not found',
      });

      const result = await deleteOllamaModel('missing-model');

      expect(result).toBe(false);
    });
  });

  describe('pollForOllama', () => {
    it('should poll until Ollama is detected', async () => {
      const onDetected = vi.fn();

      // First call: Ollama not running
      (globalThis.fetch as any).mockRejectedValueOnce(new Error('Connection refused'));

      const stopPolling = pollForOllama(onDetected, 100);

      // Verify initial call was made
      expect(globalThis.fetch).toHaveBeenCalledTimes(1);

      // Second call: Still not running
      (globalThis.fetch as any).mockRejectedValueOnce(new Error('Connection refused'));
      vi.advanceTimersByTime(100);

      // Third call: Ollama is running
      (globalThis.fetch as any).mockResolvedValueOnce({
        ok: true,
        json: async () => ({ models: [] }),
      });
      vi.advanceTimersByTime(100);

      // Wait for async operations
      await vi.runAllTimersAsync();

      expect(onDetected).toHaveBeenCalledWith({
        isRunning: true,
        host: 'http://127.0.0.1:11434',
      });

      stopPolling();
    });

    /**
     * The probe in flight when polling stops (the card unmounted) used to
     * report detection anyway, and the card set state after it was gone.
     */
    it('does not report a probe that settles after polling stopped', async () => {
      const onDetected = vi.fn();
      let answer!: (value: unknown) => void;
      (globalThis.fetch as any).mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            answer = resolve;
          })
      );

      const stopPolling = pollForOllama(onDetected, 100, 'http://127.0.0.1:11500');
      expect(globalThis.fetch).toHaveBeenCalledWith(
        'http://127.0.0.1:11500/api/tags',
        expect.anything()
      );
      stopPolling();
      answer({ ok: true, json: async () => ({ models: [] }) });
      await vi.runAllTimersAsync();

      expect(onDetected).not.toHaveBeenCalled();
    });

    it('should stop polling when stop function is called', () => {
      const onDetected = vi.fn();

      (globalThis.fetch as any).mockRejectedValue(new Error('Connection refused'));

      const stopPolling = pollForOllama(onDetected, 100);

      // Should make initial call
      expect(globalThis.fetch).toHaveBeenCalledTimes(1);

      // Stop polling
      stopPolling();

      // Advance time and verify no more calls are made
      vi.advanceTimersByTime(500);

      // Only the initial call should have been made
      expect(globalThis.fetch).toHaveBeenCalledTimes(1);
      expect(onDetected).not.toHaveBeenCalled();
    });
  });

  describe('utility functions', () => {
    it('should return correct download URL', () => {
      expect(getOllamaDownloadUrl()).toBe('https://ollama.com/download');
    });

    it('treats a tag without a version as :latest', () => {
      expect(sameOllamaTag('qwen3', 'qwen3:latest')).toBe(true);
      expect(sameOllamaTag('qwen3.8:27b', 'qwen3.8:27b')).toBe(true);
      expect(sameOllamaTag('qwen3.8:27b', 'qwen3.8:27b-q4_K_M')).toBe(false);
    });
  });

  describe('rendererOllamaUrl', () => {
    it('rewrites localhost to the loopback address the CSP admits', () => {
      expect(rendererOllamaUrl('http://localhost:11434/')).toBe('http://127.0.0.1:11434');
    });

    it('keeps a configured host, port and path prefix', () => {
      expect(rendererOllamaUrl('http://127.0.0.1:11500/')).toBe('http://127.0.0.1:11500');
      expect(rendererOllamaUrl('https://gpu-box.example.org/ollama/')).toBe(
        'https://gpu-box.example.org/ollama'
      );
    });

    it('falls back to the default for something that is not a URL', () => {
      expect(rendererOllamaUrl('not a url')).toBe('http://127.0.0.1:11434');
    });

    /** `OLLAMA_HOST=0.0.0.0` is how `ollama serve` is told to listen everywhere. */
    it('rewrites the listen-everywhere addresses to loopback', () => {
      expect(rendererOllamaUrl('http://0.0.0.0:11434/')).toBe('http://127.0.0.1:11434');
      expect(rendererOllamaUrl('http://[::]:11434/')).toBe('http://127.0.0.1:11434');
    });
  });

  describe('isRendererReachable', () => {
    it('admits only the loopback address the CSP admits', () => {
      expect(isRendererReachable('http://127.0.0.1:11434')).toBe(true);
      expect(isRendererReachable('http://127.0.0.1:11500')).toBe(true);
      expect(isRendererReachable('http://gpu-box.lab.example:11434')).toBe(false);
      expect(isRendererReachable('http://192.168.1.10:11434')).toBe(false);
      expect(isRendererReachable('not a url')).toBe(false);
    });
  });

  describe('getOllamaSetup', () => {
    it('takes the models and the host from the daemon', async () => {
      const recommendations = {
        models: [
          {
            tag: 'qwen3.8:27b',
            display_name: 'Qwen3.8 27B',
            approx_download_gb: 18,
            is_default: true,
          },
        ],
        default_model: 'qwen3.8:27b',
        min_memory_gib: 32,
        memory_gib: 64,
        memory_kind: 'apple_unified',
        suitability: 'suitable',
        suitability_message: 'ok',
        base_url: 'http://localhost:11434/',
        host_configured: false,
        tier: 'private',
      };
      mockOllamaRecommended.mockResolvedValueOnce({ data: recommendations });

      const setup = await getOllamaSetup();

      expect(setup.recommendations).toBe(recommendations);
      expect(setup.host).toBe('http://127.0.0.1:11434');
      expect(setup.reachable).toBe(true);
      expect(setup.private).toBe(true);
    });

    it('marks a server on another machine as one the renderer cannot probe', async () => {
      mockOllamaRecommended.mockResolvedValueOnce({
        data: {
          base_url: 'http://gpu-box.lab.example:11434/',
          host_configured: true,
          tier: 'public',
        },
      });

      const setup = await getOllamaSetup();

      expect(setup.host).toBe('http://gpu-box.lab.example:11434');
      expect(setup.reachable).toBe(false);
      expect(setup.private).toBe(false);
    });

    /**
     * "This computer" is the provider's tier, not a list of host names here.
     * The list called `0.0.0.0` this computer, under a card that said "Free,
     * private, offline", while the tier made every chat on it Public; and it
     * called `::1` another computer while the tier said Private.
     */
    it('takes privacy from the daemon’s tier, separately from reachability', async () => {
      mockOllamaRecommended.mockResolvedValueOnce({
        data: { base_url: 'http://0.0.0.0:11434/', host_configured: true, tier: 'public' },
      });
      const listenEverywhere = await getOllamaSetup();
      // Probed through loopback, where the same server answers…
      expect(listenEverywhere.host).toBe('http://127.0.0.1:11434');
      expect(listenEverywhere.reachable).toBe(true);
      // …and still not private.
      expect(listenEverywhere.private).toBe(false);

      mockOllamaRecommended.mockResolvedValueOnce({
        data: { base_url: 'http://[::1]:11434/', host_configured: true, tier: 'private' },
      });
      const ipv6Loopback = await getOllamaSetup();
      // Not rewritten: a server bound to ::1 does not answer on 127.0.0.1.
      expect(ipv6Loopback.host).toBe('http://[::1]:11434');
      expect(ipv6Loopback.reachable).toBe(false);
      expect(ipv6Loopback.private).toBe(true);
    });

    it('treats a missing tier as public', async () => {
      mockOllamaRecommended.mockResolvedValueOnce({
        data: { base_url: 'http://localhost:11434/', host_configured: false },
      });
      expect((await getOllamaSetup()).private).toBe(false);
    });

    it("throws the daemon's message when it cannot answer", async () => {
      mockOllamaRecommended.mockResolvedValueOnce({
        data: undefined,
        error: "Invalid Ollama host 'http://': no host name",
      });

      await expect(getOllamaSetup()).rejects.toThrow(/Invalid Ollama host/);
    });
  });

  describe('ollamaTagsForModelFile', () => {
    const BLOB = 'sha256-f5f1dd8920d4';
    /** `/api/show` for a GGUF tag, as Ollama 0.40 writes its modelfile. */
    const showGguf = (blob: string) => ({
      ok: true,
      json: async () => ({
        modelfile: `# Modelfile generated by "ollama show"\nFROM /Users/me/.ollama/models/blobs/${blob}\nFROM /Users/me/.ollama/models/blobs/sha256-projector\nTEMPLATE {{ .Prompt }}\n`,
      }),
    });
    /** An MLX model names a tag in its FROM line, never a blob. */
    const showMlx = { ok: true, json: async () => ({ modelfile: 'FROM qwen3.8:27b\n' }) };

    /**
     * Measured on Ollama 0.40.2 on Apple Silicon: `qwen3.8:27b` is the MLX
     * build and `qwen3.8:27b-q4_K_M` the GGUF that Llama Server reuses.
     */
    it('finds the GGUF tag beside an MLX bare tag, and only it', async () => {
      const fetchMock = globalThis.fetch as any;
      fetchMock.mockResolvedValueOnce(
        tagsResponse(['qwen3.8:27b', 'qwen3.8:27b-q4_K_M', 'qwen3:8b', 'gemma4:31b'])
      );
      fetchMock.mockResolvedValueOnce(showMlx);
      fetchMock.mockResolvedValueOnce(showGguf(BLOB));

      const tags = await ollamaTagsForModelFile(
        `/Users/me/.ollama/models/blobs/${BLOB}`,
        'qwen3.8:27b'
      );

      expect(tags).toEqual(['qwen3.8:27b-q4_K_M']);
      // Only the two tags of the same repository were described.
      const shown = fetchMock.mock.calls
        .filter(([url]: [string]) => url.endsWith('/api/show'))
        .map(([, init]: [string, RequestInit]) => JSON.parse(String(init.body)).model);
      expect(shown).toEqual(['qwen3.8:27b', 'qwen3.8:27b-q4_K_M']);
    });

    it('matches by blob name, whatever directory the daemon spelled', async () => {
      const fetchMock = globalThis.fetch as any;
      fetchMock.mockResolvedValueOnce(tagsResponse(['qwen3.8:27b']));
      fetchMock.mockResolvedValueOnce(showGguf(BLOB));

      expect(
        await ollamaTagsForModelFile(`/private/var/ollama/blobs/${BLOB}`, 'qwen3.8:27b')
      ).toEqual(['qwen3.8:27b']);
    });

    it('finds nothing when no tag names the file, or Ollama will not say', async () => {
      const fetchMock = globalThis.fetch as any;
      fetchMock.mockResolvedValueOnce(tagsResponse(['qwen3.8:27b', 'qwen3.8:27b-q4_K_M']));
      fetchMock.mockResolvedValueOnce(showMlx);
      fetchMock.mockResolvedValueOnce({ ok: false, status: 404, json: async () => ({}) });

      expect(
        await ollamaTagsForModelFile(`/Users/me/.ollama/models/blobs/${BLOB}`, 'qwen3.8:27b')
      ).toEqual([]);
    });

    it('asks nothing for a path that is not an Ollama blob', async () => {
      expect(await ollamaTagsForModelFile('/tmp/model.gguf', 'qwen3.8:27b')).toEqual([]);
      expect(globalThis.fetch).not.toHaveBeenCalled();
    });
  });
});
