import { llamacppStatus, ollamaContextWindow } from '../api';

/**
 * The context window a LOCAL provider is really running `model` with, or
 * `null` when it cannot say — the caller then falls back to the static
 * catalog and name-pattern tables.
 *
 * Local windows are a property of the server, not of the model's name:
 *
 * - **Llama Server** allocates its window at launch (64k or 128k, or
 *   `LLAMACPP_CONTEXT_SIZE`) and reports it back from `/props`, which the
 *   sidecar status carries as `context_size`.
 * - **Ollama** gets a `num_ctx` from Biorouter on every request (64k or 128k
 *   by memory, `OLLAMA_NUM_CTX`, capped at the model's trained window).
 *   `/ollama/context_window` answers with that same number, from the provider
 *   itself. Before it, the gauge matched the model's NAME against a pattern
 *   table — "qwen" read as 128k — which is the same guess that let the agent
 *   believe it had 128k while Ollama, sent no `num_ctx`, served its 2-4k
 *   default and cut the prompt.
 */
export async function liveContextWindow(provider: string, model: string): Promise<number | null> {
  const positive = (value: unknown) =>
    typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : null;
  try {
    if (provider === 'llamacpp') {
      const status = await llamacppStatus();
      return positive(status.data?.sidecar?.context_size);
    }
    if (provider === 'ollama') {
      const response = await ollamaContextWindow({ query: { model } });
      return positive(response.data?.context_window);
    }
  } catch (error) {
    console.warn(`Failed to read the ${provider} context window, using fallback:`, error);
  }
  return null;
}
