import { ollamaRecommended, type OllamaRecommendationsResponse } from '../api';

/**
 * Where Ollama listens when nothing says otherwise, written the one way the
 * renderer may reach it: the window's CSP admits `http://127.0.0.1:*` and not
 * `http://localhost:*` (`buildConnectSrc` in `main.ts`).
 */
export const DEFAULT_OLLAMA_HOST = 'http://127.0.0.1:11434';
const OLLAMA_DOWNLOAD_URL = 'https://ollama.com/download';

export interface OllamaStatus {
  isRunning: boolean;
  host: string;
  error?: string;
}

export interface OllamaModel {
  name: string;
  /** Ollama >= 0.1.x sends both `name` and `model`; they agree. */
  model?: string;
  size: number;
  digest: string;
  modified_at: string;
}

/** One NDJSON line of Ollama's `POST /api/pull` stream. */
export interface PullProgress {
  status: string;
  digest?: string;
  total?: number;
  completed?: number;
  /** Set on the line that reports a failure, in place of `status`. */
  error?: string;
}

/** The outcome of a pull, with Ollama's own words when it failed. */
export type PullResult = { ok: true } | { ok: false; error: string };

/**
 * Host names the renderer may probe as `127.0.0.1`. `0.0.0.0` and `::` are what
 * people set `OLLAMA_HOST` to so that `ollama serve` listens on every
 * interface; the Ollama CLI then connects to them, which the OS routes to
 * loopback, so a probe of `127.0.0.1` reaches the same server. The renderer's
 * CSP names neither, so they are rewritten.
 *
 * ⚠ This answers "can the renderer ASK that server?" and nothing else. Whether
 * the server counts as this computer is the privacy tier's question, and its
 * loopback rule is deliberately narrower: `0.0.0.0` is Public there. That
 * answer comes from the daemon (`OllamaSetup.private`); this list used to
 * stand in for it, and put "Free, private, offline" over a Public server.
 * `::1` is not here: an `ollama serve` bound to it listens on IPv6 only, where
 * a probe of `127.0.0.1` would find nothing and report the server missing.
 */
const PROBE_AS_LOOPBACK = new Set(['localhost', '0.0.0.0', '[::]']);

/**
 * Turn the daemon's resolved `OLLAMA_HOST` (`/ollama/recommended`'s
 * `base_url`, e.g. `http://localhost:11434/`) into a URL the renderer can
 * fetch: a name for this computer becomes `127.0.0.1` for the CSP above, and
 * the trailing slash goes so paths can be appended. A path prefix (Ollama
 * behind a proxy at `https://host/ollama/`) is kept.
 */
export function rendererOllamaUrl(baseUrl: string): string {
  try {
    const url = new URL(baseUrl);
    if (PROBE_AS_LOOPBACK.has(url.hostname)) url.hostname = '127.0.0.1';
    return `${url.origin}${url.pathname}`.replace(/\/+$/, '');
  } catch {
    return DEFAULT_OLLAMA_HOST;
  }
}

/**
 * Whether the renderer may fetch `host` at all. The CSP's `connect-src` admits
 * `http://127.0.0.1:*` and no other address (`buildConnectSrc` in `main.ts`),
 * so a probe of any other address is refused before it leaves the window —
 * and reads exactly like "Ollama is not running", which sent a user with a
 * working remote server to the Install Ollama link.
 */
export function isRendererReachable(host: string): boolean {
  try {
    return new URL(host).hostname === '127.0.0.1';
  } catch {
    return false;
  }
}

/** What the Ollama onboarding needs from the daemon, in one place. */
export interface OllamaSetup {
  recommendations: OllamaRecommendationsResponse;
  /** The configured Ollama server, in the form the renderer fetches. */
  host: string;
  /**
   * Whether the renderer can probe that server (see `isRendererReachable`).
   * False for another machine's Ollama, and for one at `::1`: a chat, which
   * reaches the server from the daemon, can use either.
   */
  reachable: boolean;
  /**
   * Whether a chat on this server is Private: the provider's own tier, from
   * the daemon. This — not `reachable`, and not a list of host names here —
   * is what decides whether the card may call the server "this computer" and
   * the model private.
   */
  private: boolean;
}

/**
 * The recommended models, this machine's memory verdict and the configured
 * Ollama server, from the daemon (`GET /ollama/recommended`).
 *
 * The daemon is the only source: the recommended tag used to be a constant
 * here (`gpt-oss:20b`) that had drifted from the provider's own default, and
 * the host was always 127.0.0.1:11434 whatever `OLLAMA_HOST` said. Throws with
 * the daemon's message when it cannot answer (a malformed `OLLAMA_HOST` is a
 * 400 that names the value).
 */
export async function getOllamaSetup(): Promise<OllamaSetup> {
  const response = await ollamaRecommended();
  if (!response.data) {
    const detail = typeof response.error === 'string' ? response.error : '';
    throw new Error(detail || 'Biorouter could not read its recommended Ollama models.');
  }
  const host = rendererOllamaUrl(response.data.base_url);
  return {
    recommendations: response.data,
    host,
    reachable: isRendererReachable(host),
    // Fail-safe: anything but an explicit `private` is treated as Public.
    private: response.data.tier === 'private',
  };
}

/**
 * Check whether Ollama answers at `host`.
 */
export async function checkOllamaStatus(host: string = DEFAULT_OLLAMA_HOST): Promise<OllamaStatus> {
  try {
    // Create an AbortController for timeout
    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), 2000);

    try {
      // Ollama exposes a health endpoint at /api/tags
      const response = await fetch(`${host}/api/tags`, {
        method: 'GET',
        signal: controller.signal,
      });

      window.clearTimeout(timeoutId);

      return {
        isRunning: response.ok,
        host,
      };
    } catch (err) {
      window.clearTimeout(timeoutId);
      throw err;
    }
  } catch (error) {
    return {
      isRunning: false,
      host,
      error: error instanceof Error ? error.message : 'Unknown error',
    };
  }
}

/**
 * Get the Ollama download URL
 */
export function getOllamaDownloadUrl(): string {
  return OLLAMA_DOWNLOAD_URL;
}

/**
 * Check which models are available in Ollama
 */
export async function getOllamaModels(host: string = DEFAULT_OLLAMA_HOST): Promise<OllamaModel[]> {
  try {
    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), 5000);

    try {
      const response = await fetch(`${host}/api/tags`, {
        method: 'GET',
        signal: controller.signal,
      });

      window.clearTimeout(timeoutId);

      if (!response.ok) {
        throw new Error(`Failed to get models: ${response.statusText}`);
      }

      const data = await response.json();
      return data.models || [];
    } catch (err) {
      window.clearTimeout(timeoutId);
      throw err;
    }
  } catch (error) {
    console.error('Failed to get Ollama models:', error);
    return [];
  }
}

/**
 * Ollama lists a tag pulled without one as `name:latest`, so `qwen3` and
 * `qwen3:latest` are the same model.
 */
export function sameOllamaTag(a: string, b: string): boolean {
  const normalise = (tag: string) => (tag.includes(':') ? tag : `${tag}:latest`);
  return normalise(a.trim()) === normalise(b.trim());
}

/**
 * Check if a specific model is available
 */
export async function hasModel(
  modelName: string,
  host: string = DEFAULT_OLLAMA_HOST
): Promise<boolean> {
  const models = await getOllamaModels(host);
  return models.some(
    (model) => sameOllamaTag(model.name, modelName) || sameOllamaTag(model.model ?? '', modelName)
  );
}

/** Ollama's `{"error": "..."}` body, or the status text when there is none. */
async function errorFromResponse(response: Response): Promise<string> {
  try {
    const text = await response.text();
    try {
      const parsed = JSON.parse(text) as { error?: unknown };
      if (typeof parsed.error === 'string' && parsed.error) return parsed.error;
    } catch {
      // Not JSON: the text itself is the best description there is.
    }
    if (text.trim()) return text.trim();
  } catch {
    // Body unreadable: fall through to the status line.
  }
  return `HTTP ${response.status} ${response.statusText}`.trim();
}

/**
 * Pull a model into Ollama, reporting each progress line.
 *
 * A pull counts as done only when (1) the stream carried no `error` line,
 * (2) its last word was `status: "success"`, and (3) the model is then in
 * `/api/tags`. This used to return `true` whenever the HTTP stream ended,
 * which is also how a FAILED pull ends: Ollama answers 200, streams progress,
 * then writes `{"error": "..."}` (a dropped connection, a full disk, a tag that
 * needs a newer Ollama) and closes. The card reported "Model Downloaded!" over
 * a model that was not there.
 *
 * Lines are buffered across chunks: a network read can end mid-line, and
 * parsing chunk by chunk threw the line away whole — including, when it fell
 * that way, the final `success`.
 */
export async function pullOllamaModel(
  modelName: string,
  onProgress?: (progress: PullProgress) => void,
  host: string = DEFAULT_OLLAMA_HOST
): Promise<PullResult> {
  try {
    const response = await fetch(`${host}/api/pull`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        model: modelName,
        stream: true,
      }),
    });

    if (!response.ok) {
      return { ok: false, error: await errorFromResponse(response) };
    }

    const reader = response.body?.getReader();
    if (!reader) {
      return { ok: false, error: 'Ollama sent no progress stream.' };
    }

    const decoder = new window.TextDecoder();
    // An object, not two `let`s: they are written inside `handleLine`, and
    // TypeScript keeps a closure-assigned `let` narrowed to its initial `null`.
    const seen: { lastStatus: string | null; error: string | null } = {
      lastStatus: null,
      error: null,
    };
    let buffered = '';

    const handleLine = (line: string) => {
      if (!line.trim()) return;
      let progress: PullProgress;
      try {
        progress = JSON.parse(line) as PullProgress;
      } catch {
        // A line that is not JSON is not progress; a COMPLETE one cannot be a
        // half-received line any more, so it is safe to skip.
        return;
      }
      if (typeof progress.error === 'string' && progress.error) {
        seen.error = progress.error;
        return;
      }
      seen.lastStatus = progress.status ?? seen.lastStatus;
      onProgress?.(progress);
    };

    let done = false;
    while (!done && !seen.error) {
      const { value, done: readerDone } = await reader.read();
      done = readerDone;
      buffered += value ? decoder.decode(value, { stream: true }) : decoder.decode();
      const lines = buffered.split('\n');
      // The last piece is either empty (the chunk ended on a newline) or a
      // partial line still waiting for the rest of its bytes.
      buffered = lines.pop() ?? '';
      for (const line of lines) {
        handleLine(line);
        if (seen.error) break;
      }
    }
    if (!seen.error) handleLine(buffered);

    if (seen.error) {
      try {
        await reader.cancel();
      } catch {
        // Already closed.
      }
      return { ok: false, error: seen.error };
    }
    if (seen.lastStatus !== 'success') {
      return {
        ok: false,
        error: `Ollama stopped before the download finished (last status: ${seen.lastStatus ?? 'none'}).`,
      };
    }
    if (!(await hasModel(modelName, host))) {
      return {
        ok: false,
        error: `Ollama reported success, but ${modelName} is not in its model list.`,
      };
    }
    return { ok: true };
  } catch (error) {
    console.error('Failed to pull model:', error);
    return { ok: false, error: error instanceof Error ? error.message : String(error) };
  }
}

/** The repository part of a tag: `qwen3.8` for `qwen3.8:27b-q4_K_M`. */
const repositoryOf = (tag: string) => {
  const trimmed = tag.trim();
  const colon = trimmed.lastIndexOf(':');
  // A colon before the last `/` is a registry port, not a tag separator.
  return colon > trimmed.lastIndexOf('/') ? trimmed.slice(0, colon) : trimmed;
};

/** The blob file names (`sha256-…`) a `/api/show` modelfile's `FROM` lines name. */
const fromBlobNames = (modelfile: string): string[] =>
  modelfile
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => /^FROM\s/i.test(line))
    .map((line) => line.replace(/^FROM\s+/i, '').trim())
    .map((path) => path.split(/[\\/]/).pop() ?? '')
    .filter((name) => name.startsWith('sha256-'));

/**
 * The pulled tags in `relatedTo`'s repository whose model is the file at
 * `modelPath` (a blob under Ollama's `models/blobs/`), so that deleting them
 * deletes that file and nothing else.
 *
 * Llama Server reuses a GGUF that Ollama pulled, and that GGUF need not be the
 * bare tag's: on Apple Silicon `qwen3.8:27b` is the MLX build, which
 * llama-server cannot load, and the file Llama Server lists comes from
 * `qwen3.8:27b-q4_K_M`. Deleting the row deleted `qwen3.8:27b` — the MLX model
 * the Ollama provider chats with — left the GGUF in place, and the row still
 * read installed. Ollama names each model's blob in `/api/show`'s modelfile
 * (`FROM /…/blobs/sha256-…`); an MLX model names a tag there instead, so it
 * can never match. Compared by blob name, not full path: the daemon and Ollama
 * may spell the models directory differently.
 *
 * An empty list means no tag could be matched (or Ollama did not answer);
 * the caller must then delete nothing rather than fall back to a guess.
 */
export async function ollamaTagsForModelFile(
  modelPath: string,
  relatedTo: string,
  host: string = DEFAULT_OLLAMA_HOST
): Promise<string[]> {
  const blob = modelPath.split(/[\\/]/).pop() ?? '';
  if (!blob.startsWith('sha256-')) return [];
  const repository = repositoryOf(relatedTo);
  const candidates = (await getOllamaModels(host))
    .map((model) => model.name)
    .filter((name) => repositoryOf(name) === repository);
  const matches: string[] = [];
  for (const tag of candidates) {
    try {
      const response = await fetch(`${host}/api/show`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ model: tag }),
      });
      if (!response.ok) continue;
      const shown = (await response.json()) as { modelfile?: unknown };
      if (typeof shown.modelfile === 'string' && fromBlobNames(shown.modelfile).includes(blob)) {
        matches.push(tag);
      }
    } catch (error) {
      console.error(`Failed to describe Ollama model ${tag}:`, error);
    }
  }
  return matches;
}

export async function deleteOllamaModel(
  modelName: string,
  host: string = DEFAULT_OLLAMA_HOST
): Promise<boolean> {
  try {
    const response = await fetch(`${host}/api/delete`, {
      method: 'DELETE',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        model: modelName,
      }),
    });

    if (!response.ok) {
      throw new Error(`Failed to delete model: ${response.statusText}`);
    }

    return true;
  } catch (error) {
    console.error('Failed to delete Ollama model:', error);
    return false;
  }
}

/**
 * Poll for Ollama availability
 * @param onDetected Callback when Ollama is detected
 * @param intervalMs Polling interval in milliseconds
 * @param host The Ollama server to poll
 * @returns A function to stop polling
 */
export function pollForOllama(
  onDetected: (status: OllamaStatus) => void,
  intervalMs: number = 5000,
  host: string = DEFAULT_OLLAMA_HOST
): () => void {
  let intervalId: number | null = null;
  let isPolling = true;

  const poll = async () => {
    if (!isPolling) return;

    const status = await checkOllamaStatus(host);
    // Checked again AFTER the probe: a probe in flight when polling stopped
    // (the card unmounted) used to report detection anyway, and the card's
    // callback then set state on an unmounted component.
    if (!isPolling) return;
    if (status.isRunning) {
      onDetected(status);
      stopPolling();
    }
  };

  const stopPolling = () => {
    isPolling = false;
    if (intervalId) {
      window.clearInterval(intervalId);
      intervalId = null;
    }
  };

  // Start polling immediately
  poll();

  // Then poll at intervals
  intervalId = window.setInterval(poll, intervalMs);

  return stopPolling;
}
