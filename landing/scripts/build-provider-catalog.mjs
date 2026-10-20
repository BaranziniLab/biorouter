#!/usr/bin/env node
// Turns Baseten's `GET /v1/models` response into the keyless feed Biorouter
// reads at https://biorouter.ucsf.edu/provider-catalogs/baseten.json.
//
//   node landing/scripts/build-provider-catalog.mjs <raw.json> <out.json>
//
// Run by the `baseten-catalog` job in .github/workflows/deploy-landing.yml,
// which holds the API key; this script never sees it. It copies only the
// fields the app reads (crates/biorouter/src/providers/live_catalog.rs) and
// holds them to the parser's own bounds, because the parser refuses a whole
// feed for one entry out of bounds. Until 2026-10-20 this script was looser
// than the parser (it took any positive window and any price its pattern
// matched), so it could publish a feed every client refused, and every client
// would then sit on its cached or bundled catalog with the run showing green.
//
// One rule drops an entry rather than failing: a window under MIN_CONTEXT
// marks an embedder or a reranker, which the app could never offer as a chat
// model, so it leaves with a `::warning::` line. Every other problem fails the
// build. A failed build writes nothing (exit 1, one `::error::` line), and the
// deploy job then republishes the feed that is live now.
import { readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

// The parser's bounds. Each line mirrors the constant of the same name in
// live_catalog.rs, and `live_catalog::tests` reads this file to check that
// they still match, so change the two together.
export const MIN_CONTEXT = 4096;
export const MAX_CONTEXT = 16 * 1024 * 1024;
export const MAX_TOKEN_COST = 1e-3;
export const MAX_BYTES = 256 * 1024;
export const MAX_ID_BYTES = 128;
export const MAX_MODELS = 512;

// Baseten listed 18 models on 2026-10-18 and on 2026-10-20. Far fewer means a
// broken or filtered response, not a catalog change, so the last published
// feed stays. Counted after the MIN_CONTEXT drop, so a response padded with
// embedders cannot stand in for one that lost its chat models.
export const MIN_MODELS = 8;

// The bytes an id may hold, as the parser's `valid_id` allows them. All ASCII,
// so the id's length is its size in bytes.
const ID = /^[A-Za-z0-9._/-]+$/;
// A plain decimal, or one with a negative exponent ("1e-7"). The parser takes
// nothing else, so a positive exponent is refused here by name.
const PRICE = /^\d+(\.\d+)?([eE]-\d+)?$/;
const POSITIVE_EXPONENT = /[eE]\+?\d/;
// What `new Date().toISOString()` writes, and what the parser's RFC 3339 check
// accepts: a date, `T`, a time, optional fractional seconds, `Z` or an offset.
const RFC3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

/** The bytes the CLI writes, and the size MAX_BYTES is measured on. */
export function serializeCatalog(catalog) {
  return `${JSON.stringify(catalog, null, 2)}\n`;
}

/**
 * Build the feed from Baseten's raw response. Throws on anything the parser
 * would refuse. `warn` receives one line per dropped entry; by default it goes
 * to stderr as a GitHub Actions `::warning::`.
 */
export function buildCatalog(raw, fetchedAt, { warn = (line) => console.error(`::warning::${line}`) } = {}) {
  if (typeof fetchedAt !== 'string' || !RFC3339.test(fetchedAt) || Number.isNaN(Date.parse(fetchedAt))) {
    throw new Error(`fetched_at ${JSON.stringify(fetchedAt)} is not an RFC 3339 timestamp`);
  }
  const data = raw?.data;
  if (!Array.isArray(data)) throw new Error('response has no `data` array');

  const seen = new Set();
  const models = [];
  for (const entry of data) {
    if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) {
      throw new Error(`entry ${JSON.stringify(entry)} is not an object`);
    }
    const label = typeof entry.id === 'string' ? entry.id : JSON.stringify(entry.id);
    const context = entry.context_length;
    if (!Number.isInteger(context)) {
      throw new Error(`${label}: context_length ${JSON.stringify(context)} is not an integer`);
    }
    if (context < MIN_CONTEXT) {
      warn(`Baseten catalog: dropped ${label}: context_length ${context} is below MIN_CONTEXT (${MIN_CONTEXT})`);
      continue;
    }
    if (context > MAX_CONTEXT) {
      throw new Error(`${label}: context_length ${context} is above MAX_CONTEXT (${MAX_CONTEXT})`);
    }

    const id = entry.id;
    if (typeof id !== 'string' || !ID.test(id) || id.length > MAX_ID_BYTES) {
      throw new Error(
        `bad model id ${JSON.stringify(id)}: want 1 to ${MAX_ID_BYTES} (MAX_ID_BYTES) bytes of A-Z a-z 0-9 . _ / -`,
      );
    }
    if (seen.has(id)) throw new Error(`duplicate model id ${id}`);
    seen.add(id);

    const price = (field, required) => {
      const value = entry.pricing?.[field];
      if (value === undefined || value === null) {
        if (required) throw new Error(`${id}: missing pricing.${field}`);
        return null;
      }
      const text = typeof value === 'string' || typeof value === 'number' ? String(value) : '';
      const shown = `${id}: pricing.${field} ${JSON.stringify(value)}`;
      if (POSITIVE_EXPONENT.test(text)) throw new Error(`${shown} has a positive exponent`);
      if (!PRICE.test(text)) throw new Error(`${shown} is not a finite decimal number`);
      const cost = Number(text);
      if (!Number.isFinite(cost)) throw new Error(`${shown} is not a finite decimal number`);
      if (cost > MAX_TOKEN_COST) throw new Error(`${shown} is above MAX_TOKEN_COST (${MAX_TOKEN_COST})`);
      // The string Baseten sent, so the feed never reformats a float on the way.
      return text;
    };
    const strings = (field) => (Array.isArray(entry[field]) ? entry[field].filter((v) => typeof v === 'string') : []);
    models.push({
      id,
      context_length: context,
      input_modalities: strings('input_modalities'),
      supported_features: strings('supported_features'),
      pricing: {
        prompt: price('prompt', true),
        completion: price('completion', true),
        input_cache_read: price('input_cache_read', false),
      },
    });
  }

  if (models.length < MIN_MODELS) {
    throw new Error(
      `response lists ${data.length} models, ${models.length} of them at least MIN_CONTEXT; ` +
        `expected at least ${MIN_MODELS} (MIN_MODELS)`,
    );
  }
  if (models.length > MAX_MODELS) {
    throw new Error(`response lists ${models.length} usable models; MAX_MODELS is ${MAX_MODELS}`);
  }
  models.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  const catalog = {
    schema_version: 1,
    provider: 'baseten',
    source: 'https://inference.baseten.co/v1/models',
    fetched_at: fetchedAt,
    models,
  };
  const bytes = Buffer.byteLength(serializeCatalog(catalog));
  if (bytes > MAX_BYTES) throw new Error(`the feed is ${bytes} bytes; MAX_BYTES is ${MAX_BYTES}`);
  return catalog;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) {
    console.error('usage: build-provider-catalog.mjs <raw.json> <out.json>');
    process.exit(2);
  }
  // Written beside the output and renamed into place, so a run that fails at
  // any point, the write included, leaves no output file behind.
  const staged = join(dirname(output), `.build-provider-catalog-${process.pid}.tmp`);
  try {
    const catalog = buildCatalog(JSON.parse(readFileSync(input, 'utf8')), new Date().toISOString());
    writeFileSync(staged, serializeCatalog(catalog));
    renameSync(staged, output);
    console.log(`wrote ${catalog.models.length} models to ${output}`);
  } catch (error) {
    rmSync(staged, { force: true });
    console.error(`::error::Baseten catalog not built: ${error.message}`);
    process.exit(1);
  }
}
