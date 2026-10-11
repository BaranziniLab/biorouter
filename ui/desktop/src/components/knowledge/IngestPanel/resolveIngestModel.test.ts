import { describe, expect, it } from 'vitest';
import { resolveIngestModel } from './resolveIngestModel';

describe('resolveIngestModel', () => {
  it("uses the app's configured provider and model when the base has no default", () => {
    expect(resolveIngestModel(null, 'versa_azure', 'gpt-5.5-2026-04-24')).toEqual({
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
    });
  });

  it("prefers the knowledge base's own default model over the app configuration", () => {
    expect(
      resolveIngestModel(
        { provider: 'ollama', model: 'qwen3.8:27b' },
        'versa_azure',
        'gpt-5.5-2026-04-24'
      )
    ).toEqual({ provider: 'ollama', model: 'qwen3.8:27b' });
  });

  it('returns null rather than inventing a vendor when nothing resolves', () => {
    expect(resolveIngestModel(null, null, null)).toBeNull();
    expect(resolveIngestModel(undefined, undefined, undefined)).toBeNull();
  });

  it('ignores a half-populated source instead of emitting a partial ModelRef', () => {
    expect(resolveIngestModel(null, 'versa_azure', '')).toBeNull();
    expect(resolveIngestModel(null, '', 'gpt-5.5-2026-04-24')).toBeNull();
    expect(resolveIngestModel({ provider: 'ollama', model: '' }, 'versa_azure', 'gpt-5.5')).toEqual(
      {
        provider: 'versa_azure',
        model: 'gpt-5.5',
      }
    );
  });
});

describe('the coding-agent providers are no longer excluded (#109)', () => {
  it('preselects the chat model even when it is a coding agent', () => {
    // The exclusion existed because `claude_code` / `codex` accepted `tools` on
    // `complete_with_model` and dropped them, so a macro run narrated its calls
    // as prose and wrote nothing. That is fixed at the source: a macro turn goes
    // through `ProviderToolTurnContext`, which issues the MCP bridge grant those
    // providers need. Refusing here would now hide a provider that works.
    expect(resolveIngestModel(null, 'claude_code', 'opus-5')).toEqual({
      provider: 'claude_code',
      model: 'opus-5',
    });
    expect(resolveIngestModel({ provider: 'codex', model: 'gpt-5.5-codex' }, null, null)).toEqual({
      provider: 'codex',
      model: 'gpt-5.5-codex',
    });
  });

  it('still returns null when there is genuinely nothing configured', () => {
    // The one verdict this function is allowed to reach on its own. Whether a
    // provider can carry tools is now the daemon's answer, given before a model
    // run is spent — not a name a renderer recognises.
    expect(resolveIngestModel(null, null, null)).toBeNull();
  });
});
