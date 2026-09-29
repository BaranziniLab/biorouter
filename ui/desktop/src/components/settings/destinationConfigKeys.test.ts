import { describe, expect, it } from 'vitest';
import { isDestinationConfigKey } from './destinationConfigKeys';

/**
 * The key set itself is pinned to the daemon's by a Rust test
 * (`the_desktop_mirror_names_exactly_these_keys` in
 * `crates/biorouter/src/providers/destination_keys.rs`). This pins how a key is
 * matched, which that test cannot see.
 */
describe('isDestinationConfigKey', () => {
  it('names the settings that decide where requests and their key go', () => {
    for (const key of ['OPENAI_HOST', 'AZURE_OPENAI_ENDPOINT', 'AWS_REGION', 'CODEX_COMMAND']) {
      expect(isDestinationConfigKey(key)).toBe(true);
    }
  });

  it('covers every AWS endpoint override, one per service, as the daemon does', () => {
    expect(isDestinationConfigKey('AWS_ENDPOINT_URL')).toBe(true);
    expect(isDestinationConfigKey('AWS_ENDPOINT_URL_BEDROCK_RUNTIME')).toBe(true);
    expect(isDestinationConfigKey('AWS_ENDPOINT_URL_STS')).toBe(true);
  });

  it('matches in any case, as the daemon compares', () => {
    expect(isDestinationConfigKey('openai_host')).toBe(true);
  });

  it('leaves credentials and everything else alone', () => {
    for (const key of [
      'OPENAI_API_KEY',
      'AWS_BEARER_TOKEN_BEDROCK',
      'OPENAI_TIMEOUT',
      'BIOROUTER_MODEL',
      'OLLAMA_HOST',
    ]) {
      expect(isDestinationConfigKey(key)).toBe(false);
    }
  });
});
