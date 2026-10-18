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

  it('never offers Claude Code or Codex, which the daemon refuses for every Crew task', () => {
    expect(crewTaskModelUsable(versa)).toBe(true);
    expect(crewTaskModelUsable(claudeCode)).toBe(false);
    expect(crewTaskModelUsable(codex)).toBe(false);
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

  it('falls back to the app’s default when the Crew setting names an unconfigured or unusable provider', () => {
    for (const crew of [
      { provider: 'openai', model: 'gpt-6-sol' },
      { provider: 'claude_code', model: 'opus' },
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
        configured: [claudeCode, versa],
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
