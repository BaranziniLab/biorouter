import assert from 'node:assert/strict';
import { test } from 'node:test';
import { buildCatalog, MIN_MODELS } from './build-provider-catalog.mjs';

// One entry shaped exactly like Baseten's /v1/models on 2026-10-18, including
// fields the feed must drop (`created`, `owned_by`, `quantization`, ...).
const entry = (id, overrides = {}) => ({
  id,
  created: 1754410981,
  object: 'model',
  owned_by: 'baseten',
  name: id,
  description: 'a model',
  context_length: 1048576,
  max_completion_tokens: 262144,
  input_modalities: ['text', 'image'],
  output_modalities: ['text'],
  quantization: 'fp8',
  supported_features: ['tools', 'json_mode', 'reasoning'],
  supported_sampling_parameters: ['temperature'],
  pricing: { prompt: '0.0000014', completion: '0.0000044', image: '0', request: '0', input_cache_read: '0.00000014' },
  ...overrides,
});
const response = (count = MIN_MODELS, extra = []) => ({
  data: [...Array.from({ length: count }, (_, i) => entry(`vendor/model-${i}`)), ...extra],
});

test('keeps only the fields the app parses, sorted by id', () => {
  const catalog = buildCatalog(response(MIN_MODELS, [entry('aaa/first')]), '2026-10-19T00:00:00.000Z');
  assert.equal(catalog.schema_version, 1);
  assert.equal(catalog.provider, 'baseten');
  assert.equal(catalog.models[0].id, 'aaa/first');
  assert.deepEqual(Object.keys(catalog.models[0]).sort(), [
    'context_length',
    'id',
    'input_modalities',
    'max_completion_tokens',
    'name',
    'pricing',
    'supported_features',
  ]);
  assert.deepEqual(catalog.models[0].pricing, {
    prompt: '0.0000014',
    completion: '0.0000044',
    input_cache_read: '0.00000014',
  });
});

test('a missing cache rate is null, not a failure', () => {
  const withoutCache = entry('openai/gpt-oss-120b', { pricing: { prompt: '0.0000001', completion: '0.0000005' } });
  const catalog = buildCatalog(response(MIN_MODELS, [withoutCache]), 'now');
  const model = catalog.models.find((m) => m.id === 'openai/gpt-oss-120b');
  assert.equal(model.pricing.input_cache_read, null);
});

test('refuses a response that has shrunk, so the last feed stays published', () => {
  assert.throws(() => buildCatalog(response(MIN_MODELS - 1), 'now'), /expected at least/);
  assert.throws(() => buildCatalog({ error: 'unauthorized' }, 'now'), /no `data` array/);
});

test('refuses entries the app would refuse', () => {
  const cases = [
    entry('has space'),
    entry('vendor/x', { context_length: '1048576' }),
    entry('vendor/x', { pricing: { prompt: '-1', completion: '0.1' } }),
    entry('vendor/x', { pricing: { completion: '0.1' } }),
  ];
  for (const bad of cases) {
    assert.throws(() => buildCatalog(response(MIN_MODELS, [bad]), 'now'));
  }
  assert.throws(() => buildCatalog(response(MIN_MODELS, [entry('dup/x'), entry('dup/x')]), 'now'), /duplicate/);
});
