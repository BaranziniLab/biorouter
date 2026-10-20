// Regression suite for build-provider-catalog.mjs.
//
//     node --test landing/scripts/build-provider-catalog.test.mjs
//
// The script writes the Baseten feed every Biorouter client reads, and the
// parser on the other end (crates/biorouter/src/providers/live_catalog.rs)
// refuses a whole feed for one entry out of bounds. A review on 2026-10-20
// found the script looser than the parser, so it could publish a feed every
// client refused, and found nothing tying the committed output fixture to the
// script: it was built once and could drift from it unnoticed. This file is
// what holds both. The Rust half reads the same output fixture, so the round
// trip below is the one place the script, the fixture and the parser meet.
//
// Regenerate the output fixture with the script, never by hand. From the repo
// root, after replacing fixtures/baseten-models.raw.json with a new response
// (docs/providers/baseten.md has the fetch):
//
//     node landing/scripts/build-provider-catalog.mjs \
//       landing/scripts/fixtures/baseten-models.raw.json \
//       landing/scripts/fixtures/baseten-catalog.json
//
// The round trip compares against the fixture's own `fetched_at`, so the time
// the script stamps does not matter.
//
// Every CLI refusal asserts what build-registry.test.mjs asserts, for its
// reasons: exit status EXACTLY 1, stderr naming the rule that fired, and no
// file at the path the run was asked to write.
//
// No npm dependency: node:test ships with the runtime, so this runs anywhere
// the script does, including the publishing job before it builds the feed.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  buildCatalog,
  MAX_BYTES,
  MAX_CONTEXT,
  MAX_ID_BYTES,
  MAX_MODELS,
  MAX_TOKEN_COST,
  MIN_CONTEXT,
  MIN_MODELS,
  serializeCatalog,
} from './build-provider-catalog.mjs';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(SCRIPTS, 'build-provider-catalog.mjs');
const RAW_FIXTURE = join(SCRIPTS, 'fixtures', 'baseten-models.raw.json');
const FEED_FIXTURE = join(SCRIPTS, 'fixtures', 'baseten-catalog.json');
const AT = '2026-10-20T07:58:00.000Z';

const scratch = mkdtempSync(join(tmpdir(), 'provider-catalog-'));
process.on('exit', () => rmSync(scratch, { recursive: true, force: true }));
let seq = 0;

// One entry shaped like Baseten's /v1/models on 2026-10-20, including fields
// the feed must drop (`name`, `max_completion_tokens`, `quantization`, ...).
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
const priced = (id, prompt) => entry(id, { pricing: { prompt, completion: '0.0000044' } });
const quiet = { warn: () => {} };
const build = (raw, at = AT) => buildCatalog(raw, at, quiet);

/** Run the script the way the publishing job does. */
function run(raw) {
  seq += 1;
  const input = join(scratch, `raw-${seq}.json`);
  const out = join(scratch, `out-${seq}`, 'baseten.json');
  writeFileSync(input, typeof raw === 'string' ? raw : JSON.stringify(raw));
  mkdirSync(dirname(out));
  const r = spawnSync(process.execPath, [SCRIPT, input, out], { encoding: 'utf8' });
  if (r.error) throw r.error;
  return { code: r.status, stdout: r.stdout, stderr: r.stderr, out };
}

test("the committed feed fixture is exactly what the script builds from Baseten's real response", () => {
  const raw = JSON.parse(readFileSync(RAW_FIXTURE, 'utf8'));
  const committed = JSON.parse(readFileSync(FEED_FIXTURE, 'utf8'));
  assert.deepEqual(buildCatalog(raw, committed.fetched_at, quiet), committed);
  // The bytes too, so a hand reformat cannot pass as the script's output.
  assert.equal(serializeCatalog(committed), readFileSync(FEED_FIXTURE, 'utf8'));
});

test('keeps only the fields the app reads, sorted by id, with prices as Baseten wrote them', () => {
  const catalog = build(response(MIN_MODELS, [entry('aaa/first')]));
  assert.deepEqual(Object.keys(catalog), ['schema_version', 'provider', 'source', 'fetched_at', 'models']);
  assert.equal(catalog.schema_version, 1);
  assert.equal(catalog.provider, 'baseten');
  assert.equal(catalog.fetched_at, AT);
  assert.equal(catalog.models[0].id, 'aaa/first');
  for (const model of catalog.models) {
    assert.deepEqual(Object.keys(model), ['id', 'context_length', 'input_modalities', 'supported_features', 'pricing']);
    assert.equal('name' in model, false);
    assert.equal('max_completion_tokens' in model, false);
  }
  assert.deepEqual(catalog.models[0].pricing, {
    prompt: '0.0000014',
    completion: '0.0000044',
    input_cache_read: '0.00000014',
  });
});

test('a missing cache rate is null, not a failure', () => {
  const catalog = build(response(MIN_MODELS, [priced('openai/gpt-oss-120b', '0.0000001')]));
  assert.equal(catalog.models.find((m) => m.id === 'openai/gpt-oss-120b').pricing.input_cache_read, null);
});

test('a window under MIN_CONTEXT is dropped with a warning naming the model', () => {
  const lines = [];
  const catalog = buildCatalog(
    response(MIN_MODELS, [
      entry('baai/bge-embed', { context_length: 512 }),
      entry('vendor/edge', { context_length: MIN_CONTEXT }),
    ]),
    AT,
    { warn: (line) => lines.push(line) },
  );
  const ids = catalog.models.map((m) => m.id);
  assert.equal(ids.includes('baai/bge-embed'), false);
  assert.equal(ids.includes('vendor/edge'), true, 'a window of exactly MIN_CONTEXT stays');
  assert.equal(lines.length, 1);
  assert.match(lines[0], /baai\/bge-embed.*512.*MIN_CONTEXT/);
});

test('MIN_MODELS counts what is left after the drop, so embedders cannot pad a shrunk response', () => {
  const embedders = Array.from({ length: 20 }, (_, i) => entry(`baai/embed-${i}`, { context_length: 512 }));
  assert.throws(() => build(response(MIN_MODELS - 1, embedders)), /MIN_MODELS/);
  assert.equal(build(response(MIN_MODELS, embedders)).models.length, MIN_MODELS);
  assert.throws(() => build({ error: 'unauthorized' }), /no `data` array/);
});

test('a window above MAX_CONTEXT, or one that is not an integer, fails the build', () => {
  const atCeiling = build(response(MIN_MODELS, [entry('vendor/max', { context_length: MAX_CONTEXT })]));
  assert.equal(atCeiling.models.length, MIN_MODELS + 1, 'a window of exactly MAX_CONTEXT stays');
  assert.throws(
    () => build(response(MIN_MODELS, [entry('vendor/huge', { context_length: MAX_CONTEXT + 1 })])),
    /vendor\/huge.*MAX_CONTEXT/,
  );
  for (const context_length of ['1048576', 1048576.5, null]) {
    assert.throws(() => build(response(MIN_MODELS, [entry('vendor/x', { context_length })])), /not an integer/);
  }
});

test('a price above MAX_TOKEN_COST fails the build; MAX_TOKEN_COST itself and zero pass', () => {
  assert.throws(() => build(response(MIN_MODELS, [priced('vendor/dear', '0.0011')])), /MAX_TOKEN_COST/);
  const catalog = build(
    response(MIN_MODELS, [priced('vendor/cap', String(MAX_TOKEN_COST)), priced('vendor/free', '0')]),
  );
  assert.equal(catalog.models.find((m) => m.id === 'vendor/cap').pricing.prompt, '0.001');
  // The app reads a zero price as unknown, not free, so it is published as sent.
  assert.equal(catalog.models.find((m) => m.id === 'vendor/free').pricing.prompt, '0');
});

test('a price with a positive exponent fails the build; a negative exponent passes', () => {
  for (const prompt of ['1e5', '1e+2', '1E3', '0.1e1']) {
    assert.throws(() => build(response(MIN_MODELS, [priced('vendor/x', prompt)])), /positive exponent/, prompt);
  }
  const catalog = build(response(MIN_MODELS, [priced('vendor/small', '1.4e-6')]));
  assert.equal(catalog.models.find((m) => m.id === 'vendor/small').pricing.prompt, '1.4e-6');
});

test('a price that is not a finite decimal number fails the build', () => {
  for (const prompt of ['-1', 'abc', 'NaN', 'Infinity', '', '$0.1', true, {}, '9'.repeat(400)]) {
    assert.throws(
      () => build(response(MIN_MODELS, [priced('vendor/x', prompt)])),
      /not a finite decimal number/,
      JSON.stringify(prompt),
    );
  }
  assert.throws(() => build(response(MIN_MODELS, [entry('vendor/x', { pricing: { completion: '0.1' } })])), /missing/);
});

test('a bad or duplicate model id fails the build', () => {
  const longest = `v/${'a'.repeat(MAX_ID_BYTES - 2)}`;
  assert.equal(build(response(MIN_MODELS, [entry(longest)])).models.length, MIN_MODELS + 1);
  for (const id of ['has space', '', `${longest}a`, 'vendor/ü', 'vendor:x', 42, undefined]) {
    assert.throws(() => build(response(MIN_MODELS, [entry(id)])), /bad model id/, JSON.stringify(id));
  }
  assert.throws(() => build(response(MIN_MODELS, [entry('dup/x'), entry('dup/x')])), /duplicate model id dup\/x/);
});

test('more than MAX_MODELS models fails the build', () => {
  const slim = (i) => entry(`m/${i}`, { input_modalities: [], supported_features: [] });
  const at = (count) => ({ data: Array.from({ length: count }, (_, i) => slim(i)) });
  assert.equal(build(at(MAX_MODELS)).models.length, MAX_MODELS);
  assert.throws(() => build(at(MAX_MODELS + 1)), /MAX_MODELS/);
});

test('a feed over MAX_BYTES fails the build', () => {
  const features = Array.from({ length: 40 }, (_, i) => `feature-${i}`);
  const heavy = { data: Array.from({ length: 300 }, (_, i) => entry(`m/${i}`, { supported_features: features })) };
  assert.ok(heavy.data.length <= MAX_MODELS, 'the size rule, not the count rule, must be what fires');
  assert.throws(() => build(heavy), /MAX_BYTES/);
});

test('a fetched_at that is not an RFC 3339 timestamp fails the build', () => {
  for (const at of ['now', '2026-10-20', '2026-13-45T07:58:00Z', '', undefined]) {
    assert.throws(() => buildCatalog(response(), at, quiet), /RFC 3339/, JSON.stringify(at));
  }
  assert.equal(build(response(), '2026-10-20T07:58:00+02:00').fetched_at, '2026-10-20T07:58:00+02:00');
});

test('the CLI builds the real response, stamped with the time it ran', () => {
  const r = run(readFileSync(RAW_FIXTURE, 'utf8'));
  assert.equal(r.code, 0, r.stderr);
  const written = JSON.parse(readFileSync(r.out, 'utf8'));
  const committed = JSON.parse(readFileSync(FEED_FIXTURE, 'utf8'));
  assert.ok(!Number.isNaN(Date.parse(written.fetched_at)));
  assert.deepEqual({ ...written, fetched_at: committed.fetched_at }, committed);
  assert.deepEqual(readdirSync(dirname(r.out)), ['baseten.json'], 'no staging file is left beside it');
});

test('the CLI warns on a dropped model and still writes the feed', () => {
  const r = run(response(MIN_MODELS, [entry('baai/bge-rerank', { context_length: 1024 })]));
  assert.equal(r.code, 0, r.stderr);
  assert.match(r.stderr, /^::warning::.*baai\/bge-rerank.*MIN_CONTEXT/m);
  assert.equal(existsSync(r.out), true);
});

for (const [name, raw, rule] of [
  ['a shrunk response', response(MIN_MODELS - 1), /MIN_MODELS/],
  ['a price over the ceiling', response(MIN_MODELS, [priced('vendor/dear', '0.5')]), /MAX_TOKEN_COST/],
  ['a positive exponent', response(MIN_MODELS, [priced('vendor/x', '1e3')]), /positive exponent/],
  ['a window over the ceiling', response(MIN_MODELS, [entry('v/x', { context_length: MAX_CONTEXT * 2 })]), /MAX_CONTEXT/],
  ['a duplicate id', response(MIN_MODELS, [entry('dup/x'), entry('dup/x')]), /duplicate model id/],
  ['input that is not JSON', '{"data": [', /JSON/],
]) {
  test(`the CLI refuses ${name} by its own rule and writes nothing`, () => {
    const r = run(raw);
    assert.equal(r.code, 1, `expected exit 1, got ${r.code}\n${r.stderr}`);
    const errors = r.stderr.split('\n').filter((line) => line.startsWith('::error::'));
    assert.equal(errors.length, 1, r.stderr);
    assert.match(errors[0], rule);
    assert.equal(existsSync(r.out), false, 'a refused run must leave the file it was asked to write absent');
    assert.deepEqual(readdirSync(dirname(r.out)), [], 'nor a staging file beside it');
  });
}

test('the CLI exits 2 on a usage error', () => {
  const r = spawnSync(process.execPath, [SCRIPT, RAW_FIXTURE], { encoding: 'utf8' });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /^usage:/);
});
