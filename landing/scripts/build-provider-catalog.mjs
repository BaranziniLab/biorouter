#!/usr/bin/env node
// Turns Baseten's `GET /v1/models` response into the keyless feed Biorouter
// reads at https://biorouter.ucsf.edu/provider-catalogs/baseten.json.
//
//   node landing/scripts/build-provider-catalog.mjs <raw.json> <out.json>
//
// Run by the `baseten-catalog` job in .github/workflows/deploy-landing.yml,
// which holds the API key; this script never sees it. It copies only the
// fields the app parses (crates/biorouter/src/providers/live_catalog.rs, which
// refuses unknown fields), and it fails rather than publish a feed the app
// would refuse or one that has quietly shrunk.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Baseten listed 18 models on 2026-10-18. Far fewer means a broken or
// filtered response, not a catalog change, so the last published feed stays.
export const MIN_MODELS = 8;
const MAX_MODELS = 512;
const ID = /^[A-Za-z0-9._/-]{1,128}$/;
const PRICE = /^\d+(\.\d+)?([eE]-?\d+)?$/;

export function buildCatalog(raw, fetchedAt) {
  const data = raw?.data;
  if (!Array.isArray(data)) throw new Error('response has no `data` array');
  if (data.length < MIN_MODELS) {
    throw new Error(`response lists ${data.length} models; expected at least ${MIN_MODELS}`);
  }
  if (data.length > MAX_MODELS) throw new Error(`response lists ${data.length} models`);

  const seen = new Set();
  const models = data.map((entry) => {
    const id = entry?.id;
    if (typeof id !== 'string' || !ID.test(id)) throw new Error(`bad model id ${JSON.stringify(id)}`);
    if (seen.has(id)) throw new Error(`duplicate model id ${id}`);
    seen.add(id);
    if (!Number.isInteger(entry.context_length) || entry.context_length <= 0) {
      throw new Error(`${id}: bad context_length`);
    }
    const price = (field, required) => {
      const value = entry.pricing?.[field];
      if (value === undefined || value === null) {
        if (required) throw new Error(`${id}: missing pricing.${field}`);
        return null;
      }
      const text = String(value);
      if (!PRICE.test(text)) throw new Error(`${id}: bad pricing.${field} ${JSON.stringify(value)}`);
      return text;
    };
    const strings = (field) => (Array.isArray(entry[field]) ? entry[field].filter((v) => typeof v === 'string') : []);
    return {
      id,
      name: typeof entry.name === 'string' ? entry.name : null,
      context_length: entry.context_length,
      max_completion_tokens: Number.isInteger(entry.max_completion_tokens) ? entry.max_completion_tokens : null,
      input_modalities: strings('input_modalities'),
      supported_features: strings('supported_features'),
      pricing: {
        prompt: price('prompt', true),
        completion: price('completion', true),
        input_cache_read: price('input_cache_read', false),
      },
    };
  });
  models.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  return {
    schema_version: 1,
    provider: 'baseten',
    source: 'https://inference.baseten.co/v1/models',
    fetched_at: fetchedAt,
    models,
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) {
    console.error('usage: build-provider-catalog.mjs <raw.json> <out.json>');
    process.exit(2);
  }
  try {
    const catalog = buildCatalog(JSON.parse(readFileSync(input, 'utf8')), new Date().toISOString());
    writeFileSync(output, `${JSON.stringify(catalog, null, 2)}\n`);
    console.log(`wrote ${catalog.models.length} models to ${output}`);
  } catch (error) {
    console.error(`::error::Baseten catalog not built: ${error.message}`);
    process.exit(1);
  }
}
