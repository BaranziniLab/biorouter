import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Mock } from 'vitest';

vi.mock('../../../../../../api', () => ({
  checkProvider: vi.fn(async () => ({ data: {} })),
}));

import { providerConfigSubmitHandler } from './DefaultSubmitHandler';

/**
 * What the daemon must end up writing into `config.yaml`.
 *
 * The form renders every key as an `<input type="text">`, so the handler is
 * always handed strings. `/config/upsert` writes what it is given verbatim, and
 * the backend reads these keys back with a *typed* `Config::get_param::<T>()`
 * (`crates/biorouter/src/config/base.rs`), which bottoms out in
 * `serde_yaml::from_value::<T>()`. serde_yaml does not coerce a YAML string into
 * a `usize`/`u16`/`bool`, so `LLAMACPP_PORT: '11543'` reads as `Err` and every
 * call site swallows it with `.ok()` / `.unwrap_or(DEFAULT)`: the setting saves,
 * reloads into the form, and does nothing.
 *
 * These assertions are on the JS *type* of the upserted value, not on its text.
 * `toBe('11543')` and `toBe(11543)` are both satisfied by a loose matcher, and
 * the string one is the whole bug — so each case pins `typeof` as well.
 */

/** The real llamacpp config keys, as `crates/biorouter/src/providers/llamacpp.rs` declares them. */
const LLAMACPP_KEYS = [
  { name: 'LLAMACPP_PORT', required: true, secret: false, default: '11543' },
  { name: 'LLAMACPP_CONTEXT_SIZE', required: false, secret: false, default: '0' },
  { name: 'LLAMACPP_TIMEOUT', required: false, secret: false, default: '600' },
  { name: 'LLAMACPP_ENABLE_THINKING', required: false, secret: false, default: 'false' },
  { name: 'LLAMACPP_EXTERNAL_HOST', required: false, secret: false, default: undefined },
];

function providerWith(config_keys: Array<Record<string, unknown>>) {
  return { name: 'llamacpp', metadata: { config_keys } } as Parameters<
    typeof providerConfigSubmitHandler
  >[1];
}

type UpsertFn = (key: string, value: unknown, isSecret: boolean) => Promise<void>;
type UpsertMock = Mock<UpsertFn>;

/** Every `(key, value, isSecret)` triple the handler pushed, as a lookup by key. */
function written(upsert: UpsertMock): Record<string, unknown> {
  return Object.fromEntries(upsert.mock.calls.map((call) => [call[0], call[1]]));
}

describe('providerConfigSubmitHandler value types', () => {
  let upsert: UpsertMock;

  beforeEach(() => {
    upsert = vi.fn<UpsertFn>(async () => undefined);
  });

  it('writes a numeric key as a JS number, not a quoted string', async () => {
    await providerConfigSubmitHandler(upsert, providerWith(LLAMACPP_KEYS), {
      LLAMACPP_PORT: '11543',
      LLAMACPP_CONTEXT_SIZE: '8192',
      LLAMACPP_TIMEOUT: '600',
    });

    const values = written(upsert);
    expect(values.LLAMACPP_PORT).toBe(11543);
    expect(typeof values.LLAMACPP_PORT).toBe('number');
    expect(values.LLAMACPP_CONTEXT_SIZE).toBe(8192);
    expect(typeof values.LLAMACPP_CONTEXT_SIZE).toBe('number');
    expect(values.LLAMACPP_TIMEOUT).toBe(600);
    expect(typeof values.LLAMACPP_TIMEOUT).toBe('number');
  });

  it('writes a boolean key as a JS boolean, not the text "true"', async () => {
    await providerConfigSubmitHandler(upsert, providerWith(LLAMACPP_KEYS), {
      LLAMACPP_PORT: '11543',
      LLAMACPP_ENABLE_THINKING: 'true',
    });

    const values = written(upsert);
    expect(values.LLAMACPP_ENABLE_THINKING).toBe(true);
    expect(typeof values.LLAMACPP_ENABLE_THINKING).toBe('boolean');
  });

  it('writes "false" as boolean false — the value the user actually has on disk', async () => {
    await providerConfigSubmitHandler(upsert, providerWith(LLAMACPP_KEYS), {
      LLAMACPP_PORT: '11543',
      LLAMACPP_ENABLE_THINKING: 'false',
    });

    expect(written(upsert).LLAMACPP_ENABLE_THINKING).toBe(false);
  });

  it('leaves a string key a string, including one whose value looks numeric', async () => {
    await providerConfigSubmitHandler(
      upsert,
      providerWith([
        { name: 'OPENAI_HOST', required: true, secret: false, default: 'https://api.openai.com' },
        { name: 'OPENAI_BASE_PATH', required: true, secret: false, default: 'v1/chat/completions' },
        { name: 'AZURE_OPENAI_API_VERSION', required: true, secret: false, default: '2025-01-01' },
        { name: 'AWS_PROFILE', required: true, secret: false, default: 'default' },
      ]),
      {
        OPENAI_HOST: 'https://api.openai.com',
        OPENAI_BASE_PATH: 'v1/chat/completions',
        // A string key whose value is a bare number must NOT become one: the
        // backend reads it with `get_param::<String>()`, which a YAML number
        // fails just as surely as a YAML string fails `get_param::<usize>()`.
        AZURE_OPENAI_API_VERSION: '2024',
        AWS_PROFILE: '12345',
      }
    );

    const values = written(upsert);
    expect(values.OPENAI_HOST).toBe('https://api.openai.com');
    expect(values.OPENAI_BASE_PATH).toBe('v1/chat/completions');
    expect(values.AZURE_OPENAI_API_VERSION).toBe('2024');
    expect(typeof values.AZURE_OPENAI_API_VERSION).toBe('string');
    expect(values.AWS_PROFILE).toBe('12345');
    expect(typeof values.AWS_PROFILE).toBe('string');
  });

  it('never coerces a secret, however numeric it looks', async () => {
    await providerConfigSubmitHandler(
      upsert,
      providerWith([{ name: 'SOME_API_KEY', required: true, secret: true, default: '0' }]),
      { SOME_API_KEY: '1234567890' }
    );

    const call = upsert.mock.calls[0];
    expect(call[1]).toBe('1234567890');
    expect(typeof call[1]).toBe('string');
    expect(call[2]).toBe(true);
  });

  it('does not turn an empty required value into 0', async () => {
    await providerConfigSubmitHandler(
      upsert,
      providerWith([{ name: 'LLAMACPP_PORT', required: true, secret: false, default: '11543' }]),
      { LLAMACPP_PORT: '' }
    );

    expect(written(upsert).LLAMACPP_PORT).toBe('');
  });

  it('writes unparseable text through unchanged rather than as NaN', async () => {
    await providerConfigSubmitHandler(upsert, providerWith(LLAMACPP_KEYS), {
      LLAMACPP_PORT: 'not-a-port',
      LLAMACPP_ENABLE_THINKING: 'yes',
    });

    const values = written(upsert);
    expect(values.LLAMACPP_PORT).toBe('not-a-port');
    expect(values.LLAMACPP_ENABLE_THINKING).toBe('yes');
  });

  it('coerces on the all-optional-with-defaults path too', async () => {
    // No required key and every key defaulted takes the early-return branch,
    // which has its own upsert call site.
    const keys = [
      { name: 'GCP_MAX_RETRIES', required: false, secret: false, default: '4' },
      { name: 'GCP_BACKOFF_MULTIPLIER', required: false, secret: false, default: '2' },
      { name: 'GCP_LOCATION', required: false, secret: false, default: 'us-central1' },
    ];

    await providerConfigSubmitHandler(upsert, providerWith(keys), {
      GCP_MAX_RETRIES: '7',
      GCP_BACKOFF_MULTIPLIER: '1.5',
    });

    const values = written(upsert);
    expect(values.GCP_MAX_RETRIES).toBe(7);
    expect(values.GCP_BACKOFF_MULTIPLIER).toBe(1.5);
    // Untouched by the user, so its declared default is what gets written — and
    // a string default stays a string.
    expect(values.GCP_LOCATION).toBe('us-central1');
  });
});

/**
 * W2-PRV-2. A key was saved over the working one and only then "checked" by
 * building the provider, which makes no network call, so a typo saved as
 * Configured. The values are now checked live, as candidates, BEFORE anything
 * is written, and a rejection writes nothing.
 */
describe('providerConfigSubmitHandler checks a credential before saving it', () => {
  const anthropic = {
    name: 'anthropic',
    metadata: {
      config_keys: [
        { name: 'ANTHROPIC_API_KEY', required: true, secret: true },
        {
          name: 'ANTHROPIC_HOST',
          required: true,
          secret: false,
          default: 'https://api.anthropic.com',
        },
      ],
    },
  } as Parameters<typeof providerConfigSubmitHandler>[1];

  let upsert: Mock<(key: string, value: unknown, isSecret: boolean) => Promise<void>>;
  let check: Mock;

  beforeEach(async () => {
    upsert = vi.fn(async () => undefined);
    check = (await import('../../../../../../api')).checkProvider as unknown as Mock;
    check.mockReset();
    check.mockResolvedValue({ data: {} });
  });

  it('checks the exact values live, as candidates, before the first write', async () => {
    await providerConfigSubmitHandler(upsert, anthropic, { ANTHROPIC_API_KEY: 'sk-ant-new' });

    expect(check.mock.calls[0][0]).toMatchObject({
      body: {
        provider: 'anthropic',
        live: true,
        candidate: { ANTHROPIC_API_KEY: 'sk-ant-new', ANTHROPIC_HOST: 'https://api.anthropic.com' },
      },
      throwOnError: true,
    });
    expect(check.mock.invocationCallOrder[0]).toBeLessThan(upsert.mock.invocationCallOrder[0]);
    expect(upsert).toHaveBeenCalledWith('ANTHROPIC_API_KEY', 'sk-ant-new', true);
  });

  it('writes nothing when the provider rejects the key', async () => {
    check.mockRejectedValueOnce('Anthropic rejected these credentials: invalid x-api-key');
    await expect(
      providerConfigSubmitHandler(upsert, anthropic, { ANTHROPIC_API_KEY: 'sk-ant-typo' })
    ).rejects.toBe('Anthropic rejected these credentials: invalid x-api-key');
    expect(upsert).not.toHaveBeenCalled();
  });

  it('does not check live a save that carries no credential', async () => {
    await providerConfigSubmitHandler(
      upsert,
      {
        name: 'databricks',
        metadata: { config_keys: [{ name: 'DATABRICKS_HOST', required: true, secret: false }] },
      },
      { DATABRICKS_HOST: 'https://x.cloud.databricks.com' }
    );
    expect(check).toHaveBeenCalledTimes(1);
    expect(check.mock.calls[0][0].body).toEqual({ provider: 'databricks' });
  });

  it('writes every setting before any credential', async () => {
    await providerConfigSubmitHandler(upsert, anthropic, {
      ANTHROPIC_API_KEY: 'sk-ant-new',
      ANTHROPIC_HOST: 'https://gateway.example',
    });
    const order = (key: string) =>
      upsert.mock.invocationCallOrder[upsert.mock.calls.findIndex((call) => call[0] === key)];
    expect(order('ANTHROPIC_HOST')).toBeLessThan(order('ANTHROPIC_API_KEY'));
  });

  it('saves no key when the daemon refuses the host beside it', async () => {
    // What a daemon that cannot prove a person answers for a moved host.
    const refusal = "'ANTHROPIC_HOST' decides where a provider sends its requests";
    upsert.mockImplementation(async (key) => {
      if (key === 'ANTHROPIC_HOST') throw refusal;
    });
    await expect(
      providerConfigSubmitHandler(upsert, anthropic, {
        ANTHROPIC_API_KEY: 'sk-ant-new',
        ANTHROPIC_HOST: 'https://gateway.example',
      })
    ).rejects.toBe(refusal);
    expect(upsert).not.toHaveBeenCalledWith('ANTHROPIC_API_KEY', 'sk-ant-new', true);
  });

  it('checks a new host against the saved key before saving it', async () => {
    await providerConfigSubmitHandler(upsert, anthropic, {
      ANTHROPIC_HOST: 'https://gateway.example',
    });
    expect(check.mock.calls[0][0].body).toEqual({
      provider: 'anthropic',
      live: true,
      candidate: { ANTHROPIC_HOST: 'https://gateway.example' },
    });
    expect(check.mock.invocationCallOrder[0]).toBeLessThan(upsert.mock.invocationCallOrder[0]);
  });
});
