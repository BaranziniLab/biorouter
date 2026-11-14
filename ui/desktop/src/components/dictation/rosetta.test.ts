import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { resolveDictationPhase, type DictationStatusSnapshot } from './dictationStore';
import type { DictationStatus } from './dictationApi';
import { ROSETTA_BEFORE_MACOS15_KEY, rosettaBeforeMacOS15 } from './rosetta';

describe('rosettaBeforeMacOS15', () => {
  it.each([
    ['darwin', true, '14.6.1', true],
    ['darwin', true, '12.7.0', true],
    ['darwin', true, '15.0.0', false],
    ['darwin', true, '26.1', false],
    ['darwin', false, '13.0.0', false],
    ['win32', true, '10.0.22631', false],
    ['darwin', true, '', false],
    ['darwin', true, 'unknown', false],
  ] as const)('%s, translated %s, macOS %s: %s', (platform, translated, version, expected) => {
    expect(rosettaBeforeMacOS15(platform, translated, version)).toBe(expected);
  });
});

describe('the phase reads what main computed', () => {
  const unsupported = {
    load: 'loaded',
    micStatus: 'unknown',
    status: {
      engine: { compiled: true, state: 'unsupported_cpu', missing_cpu_features: ['avx2', 'fma'] },
      policy: { enabled: { allowed: true }, capture: { allowed: true } },
      models: [],
      active_model: null,
      install: { allowed: true },
    } as unknown as DictationStatus,
  } as DictationStatusSnapshot;

  afterEach(() => {
    delete (window as unknown as { appConfig?: unknown }).appConfig;
  });

  const withAppConfig = (values: Record<string, unknown>) => {
    (window as unknown as { appConfig: unknown }).appConfig = { get: (key: string) => values[key] };
  };

  it('says MSG-CPU-ROSETTA when main set the flag', () => {
    withAppConfig({ [ROSETTA_BEFORE_MACOS15_KEY]: true });
    expect(resolveDictationPhase(unsupported, { secure: true }).reasonId).toBe('MSG-CPU-ROSETTA');
  });

  it('says MSG-CPU without it, and an explicit environment wins', () => {
    withAppConfig({});
    expect(resolveDictationPhase(unsupported, { secure: true }).reasonId).toBe('MSG-CPU');
    withAppConfig({ [ROSETTA_BEFORE_MACOS15_KEY]: true });
    expect(
      resolveDictationPhase(unsupported, { secure: true, rosettaBeforeMacOS15: false }).reasonId
    ).toBe('MSG-CPU');
  });
});

/**
 * The flag reaches the window through its start up configuration, not through a
 * preload method: main serialises it into `additionalArguments`, preload hands
 * every key of that JSON to `window.appConfig.get`, and the store reads it there.
 * jsdom runs none of the three, so these pin each hop in the source.
 */
describe('main hands the flag to every chat window', () => {
  const src = resolve(__dirname, '../..');
  const read = (path: string) => readFileSync(join(src, path), 'utf8');

  it('main sets the shared key from the shared function, in the chat window’s configuration', () => {
    const main = read('main.ts');
    expect(main).toMatch(
      /import \{ ROSETTA_BEFORE_MACOS15_KEY, rosettaBeforeMacOS15 \} from '\.\/components\/dictation\/rosetta';/
    );
    // The `additionalArguments` JSON of the window that carries the API host, which every
    // chat window gets.
    const windowConfig = main.match(
      /additionalArguments: \[\s*JSON\.stringify\(\{\s*\.\.\.appConfig,\s*BIOROUTER_API_HOST: baseUrl,[\s\S]*?\}\),\s*\]/
    );
    expect(windowConfig, 'the chat window configuration').not.toBeNull();
    expect(windowConfig![0].replace(/\s+/g, ' ')).toContain(
      '[ROSETTA_BEFORE_MACOS15_KEY]: rosettaBeforeMacOS15( process.platform, app.runningUnderARM64Translation === true, process.getSystemVersion() )'
    );
    // Spelled once: main never writes the key by its string.
    expect(main).not.toContain(`'${ROSETTA_BEFORE_MACOS15_KEY}'`);
  });

  it('preload passes every key of that JSON through appConfig.get', () => {
    const preload = read('preload.ts');
    expect(preload).toContain(
      "const config = JSON.parse(process.argv.find((arg) => arg.startsWith('{')) || '{}');"
    );
    expect(preload).toMatch(/get: \(key: string\) => config\[key\],/);
  });

  it('the store reads it there, as a boolean', () => {
    const store = read('components/dictation/dictationStore.ts');
    expect(store).toContain("import { ROSETTA_BEFORE_MACOS15_KEY } from './rosetta';");
    expect(store).toContain('window.appConfig?.get(ROSETTA_BEFORE_MACOS15_KEY) === true');
  });
});
