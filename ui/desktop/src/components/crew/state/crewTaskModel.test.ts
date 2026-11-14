import { describe, expect, it } from 'vitest';
import type { ProviderDetails } from '../../../api';
import {
  CREW_TASK_MODEL_KEY,
  crewTaskModelUsable,
  parseTaskModel,
  resolveTaskModel,
} from './crewTaskModel';

const provider = (name: string, configured = true) =>
  ({ name, is_configured: configured, metadata: { display_name: name } }) as ProviderDetails;

const versa = provider('versa_azure');
const ollama = provider('ollama');
const claudeCode = provider('claude_code');
const codex = provider('codex');
const unconfigured = provider('openai', false);

describe('the model Crew tasks start on (OW-11)', () => {
  it('is the config key the CLI reads too', () => {
    expect(CREW_TASK_MODEL_KEY).toBe('CREW_TASK_MODEL');
  });

  it('reads a stored choice only when both fields are non-blank strings', () => {
    expect(parseTaskModel({ provider: 'ollama', model: ' qwen3.8:27b ' })).toEqual({
      provider: 'ollama',
      model: 'qwen3.8:27b',
    });
    for (const value of [
      null,
      undefined,
      '',
      'ollama',
      [],
      { provider: 'ollama' },
      { model: 'qwen3.8:27b' },
      { provider: ' ', model: 'qwen3.8:27b' },
      { provider: 'ollama', model: 7 },
    ])
      expect(parseTaskModel(value)).toBeNull();
  });

  it('offers every configured provider, Claude Code and Codex included (coding-agent parity)', () => {
    expect(crewTaskModelUsable(versa)).toBe(true);
    expect(crewTaskModelUsable(claudeCode)).toBe(true);
    expect(crewTaskModelUsable(codex)).toBe(true);
    expect(crewTaskModelUsable(provider('claude_code', false))).toBe(false);
    expect(crewTaskModelUsable(provider('codex', false))).toBe(false);
    expect(crewTaskModelUsable(unconfigured)).toBe(false);
    expect(crewTaskModelUsable(undefined)).toBe(false);
  });

  it('resolves the Crew setting first, then the app’s default, and says which', () => {
    const configured = [versa, ollama];
    expect(
      resolveTaskModel({
        crew: { provider: 'ollama', model: 'qwen3.8:27b' },
        appProvider: 'versa_azure',
        appModel: 'gpt-5.5',
        configured,
      })
    ).toEqual({ choice: { provider: 'ollama', model: 'qwen3.8:27b' }, source: 'crew' });
    expect(
      resolveTaskModel({ crew: null, appProvider: 'versa_azure', appModel: 'gpt-5.5', configured })
    ).toEqual({ choice: { provider: 'versa_azure', model: 'gpt-5.5' }, source: 'app' });
  });

  it('resolves Claude Code or Codex like any configured provider, from either source', () => {
    expect(
      resolveTaskModel({
        crew: { provider: 'claude_code', model: 'opus' },
        appProvider: 'versa_azure',
        appModel: 'gpt-5.5',
        configured: [versa, claudeCode],
      })
    ).toEqual({ choice: { provider: 'claude_code', model: 'opus' }, source: 'crew' });
    expect(
      resolveTaskModel({
        crew: null,
        appProvider: 'codex',
        appModel: 'gpt-5.5-codex',
        configured: [codex],
      })
    ).toEqual({ choice: { provider: 'codex', model: 'gpt-5.5-codex' }, source: 'app' });
  });

  it('falls back to the app’s default when the Crew setting names an unconfigured provider or no model', () => {
    for (const crew of [
      { provider: 'openai', model: 'gpt-6-sol' },
      { provider: 'codex', model: 'gpt-5.5-codex' },
      { provider: 'versa_azure' },
    ])
      expect(
        resolveTaskModel({
          crew,
          appProvider: 'versa_azure',
          appModel: 'gpt-5.5',
          configured: [versa, claudeCode, unconfigured],
        })?.source
      ).toBe('app');
  });

  it('resolves nothing when neither names a model a Crew task could start on', () => {
    expect(
      resolveTaskModel({
        crew: '',
        appProvider: 'claude_code',
        appModel: 'opus',
        configured: [provider('claude_code', false), versa],
      })
    ).toBeNull();
    expect(
      resolveTaskModel({
        crew: null,
        appProvider: 'versa_azure',
        appModel: ' ',
        configured: [versa],
      })
    ).toBeNull();
    expect(
      resolveTaskModel({ crew: null, appProvider: null, appModel: null, configured: [] })
    ).toBeNull();
  });
});
