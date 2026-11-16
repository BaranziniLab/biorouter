import { getModelDisplayName } from '../settings/models/predefinedModelsUtils';

/**
 * The model's name as a person says it (spec 3.7): the release-date stamp a
 * provider appends comes off, so `gpt-5.6-sol-2026-07-09` reads `gpt-5.6-sol`
 * and `claude-3-5-sonnet-20241022` reads `claude-3-5-sonnet`. The full id stays
 * in the chip's tooltip, its accessible name and the menu.
 *
 * Only a trailing date is removed, and only when something is left in front of
 * it: a name that IS a date, or has none, comes back unchanged.
 *
 * A Bedrock id carries more than a date: an inference profile's region
 * (`us.`, `eu.`, `apac.`, `global.`), the vendor (`anthropic.`) and the API
 * version (`-v1:0`, after which a date is no longer trailing). Those come off
 * too, so `us.anthropic.claude-sonnet-4-6` reads `claude-sonnet-4-6` and
 * `anthropic.claude-3-5-sonnet-20240620-v1:0` reads `claude-3-5-sonnet`. A
 * name left as one word keeps its vendor (`deepseek.r1-v1:0` reads
 * `deepseek-r1`, not `r1`).
 *
 * The version rule is Bedrock's only: elsewhere `-v3` is a generation
 * (`deepseek-v3`), and the vendor segment is recognised by name unless a
 * region prefix already says the id is Bedrock's, so `qwen2.5` or
 * `llama3.1:8b` are never read as one.
 */
const TRAILING_DATE = /-(?:\d{4}-\d{2}-\d{2}|\d{8})$/;

const BEDROCK_REGION = '(?:us|us-gov|eu|apac|jp|au|ca|global)';
const BEDROCK_VENDOR =
  '(?:amazon|anthropic|ai21|cohere|deepseek|google|luma|meta|minimax|mistral|moonshot|nvidia|openai|qwen|stability|twelvelabs|writer)';
const BEDROCK_ID = new RegExp(
  `^(?:${BEDROCK_REGION}\\.([a-z][a-z0-9-]*)|(${BEDROCK_VENDOR}))\\.(.+)$`
);
const BEDROCK_VERSION = /-(?:v\d+(?::\d+)?|\d+:\d+)$/;

function withoutDate(name: string): string {
  const stripped = name.replace(TRAILING_DATE, '');
  return stripped.length > 0 ? stripped : name;
}

export function friendlyModelName(id: string): string {
  const trimmed = id.trim();
  const bedrock = BEDROCK_ID.exec(trimmed);
  if (!bedrock) return withoutDate(trimmed);

  const vendor = bedrock[1] ?? bedrock[2];
  const model = bedrock[3];
  const unversioned = model.replace(BEDROCK_VERSION, '');
  const name = withoutDate(unversioned.length > 0 ? unversioned : model);
  return name.includes('-') ? name : `${vendor}-${name}`;
}

/**
 * The model's name as a person reads it, the one name every surface shows (Phase C, T1 F2).
 * The same Bedrock model read `us.anthropic.claude-haiku-5-5` in Settings and the model picker,
 * `claude-haiku-5-5` on the composer chip and "Claude Haiku 5.5" in the provider documentation.
 * Now each of them reads "Claude Haiku 5.5", and the raw id a person may need (to type into
 * `biorouter configure`, or to quote) sits beside it as secondary text or in its tooltip.
 *
 * The name is, in order:
 * 1. the alias a deployment gives the model (`BIOROUTER_PREDEFINED_MODELS`);
 * 2. the family's own written name, for the families whose naming is regular: Claude
 *    (`Claude Opus 5.5`, `Claude 3.5 Sonnet`), GPT (`GPT-5.6 Sol`, `GPT-5 mini`, `GPT-4o`,
 *    `GPT-5.3-Codex`), Gemini (`Gemini 3 Pro Preview`), and the two local models Biorouter
 *    recommends, Qwen (`Qwen3.8 27B`) and Gemma (`Gemma 4 31B`), spelled as Llama Server's
 *    catalog and Ollama's recommendation spell them;
 * 3. otherwise the short id ({@link shortModelId}).
 * Both steps read the short id, so a Bedrock profile, an API version, a date stamp, a
 * `vendor/` path or Databricks' `databricks-` endpoint prefix never decides the name: one model
 * reads the same whichever provider serves it.
 */
export function modelDisplayName(id: string): string {
  const trimmed = id.trim();
  if (!trimmed) return id;
  // No model id has a space in it, and every name does ("Claude Haiku 5.5", "Qwen3.8 27B", an
  // alias), so a name handed back in comes out as it went in.
  if (/\s/.test(trimmed)) return trimmed;
  // The predefined list is read through `window.appConfig`, which only a host provides (Electron's
  // preload, the web shell's shim); without one there is no list, and asking would log a parse
  // failure on every call (as `crew/pane/presentation.ts` `modelDisplay` notes).
  const hosted = typeof window !== 'undefined' && typeof window.appConfig?.get === 'function';
  const alias = hosted ? getModelDisplayName(trimmed) : trimmed;
  if (alias && alias !== trimmed) return alias;
  const short = shortModelId(trimmed);
  return familyName(short) ?? short;
}

/** One row of a list of models, as {@link distinguishModels} reads it. */
export interface ModelRowName {
  /** What the row reads by: its display name, or the surface's own name for the row. */
  name: string;
  /** The provider that served it, written after the name when the name alone is not enough. */
  provider?: string | null;
  /** The raw id, written last when the name and the provider are still not enough. */
  id?: string | null;
}

/** For each entry, whether another entry in the list reads the same. */
function repeated(labels: readonly string[]): boolean[] {
  const counts = new Map<string, number>();
  for (const label of labels) counts.set(label, (counts.get(label) ?? 0) + 1);
  return labels.map((label) => (counts.get(label) ?? 0) > 1);
}

/** What a row of a list needs written after its name to read apart from the others. */
export interface ModelRowParts {
  provider: boolean;
  id: boolean;
}

/**
 * What tells a list's models apart. A display name drops what only says where a model is served
 * (a date stamp, a Bedrock region profile, a `vendor/` path), so two rows of one list can read the
 * same: one model through two providers, or two snapshots of one model from one provider
 * (`gpt-5.5-2026-04-24` and `gpt-5.5-2026-07-09` both read "GPT-5.5"), each priced on its own.
 * A row reads by its name alone; where another row reads the same, its provider follows; where
 * that still reads the same, its raw id follows too. Rows keyed by provider and id end apart.
 */
export function distinguishModels(rows: readonly ModelRowName[]): ModelRowParts[] {
  const needProvider = repeated(rows.map((row) => row.name));
  const withProvider = rows.map((row, index) =>
    needProvider[index] && row.provider ? `${row.name} · ${row.provider}` : row.name
  );
  const needId = repeated(withProvider);
  return rows.map((row, index) => ({
    provider: needProvider[index] && !!row.provider,
    id: needId[index] && !!row.id,
  }));
}

/**
 * {@link distinguishModels}' answer as one line per row: "GPT-5.5", "GPT-5.5 · openai", or
 * "GPT-5.5 · openai · gpt-5.5-2026-04-24".
 */
export function distinctModelLabels(rows: readonly ModelRowName[]): string[] {
  return distinguishModels(rows).map((parts, index) => {
    const row = rows[index];
    return [row.name, parts.provider && row.provider, parts.id && row.id]
      .filter(Boolean)
      .join(' · ');
  });
}

const VERTEX_DATE = /@(?:\d{8}|\d{4}-\d{2}-\d{2})$/;

/**
 * The id without what only says where it is served: {@link friendlyModelName}'s Bedrock and
 * date rules, then the last `vendor/` path segment (`zai-org/GLM-5.3` reads `GLM-5.3`,
 * `accounts/fireworks/models/kimi-k2-instruct` reads `kimi-k2-instruct`) and Vertex AI's
 * `@20250514` date. An Ollama tag keeps all of itself, `:latest` included: it is the name the
 * person pulled, and the decision surfaces name it so.
 */
function shortModelId(id: string): string {
  const friendly = friendlyModelName(id);
  const last = friendly.slice(friendly.lastIndexOf('/') + 1) || friendly;
  const short = last.replace(VERTEX_DATE, '');
  return short || last;
}

const capitalise = (word: string) => word.charAt(0).toUpperCase() + word.slice(1);

const CLAUDE_TIER = '(opus|sonnet|haiku|fable|mythos)';
const CLAUDE_NAMED = new RegExp(`^claude-${CLAUDE_TIER}-(\\d+)(?:[-.](\\d+))?$`);
const CLAUDE_NUMBERED = new RegExp(`^claude-(\\d+)(?:[-.](\\d+))?-${CLAUDE_TIER}$`);
const GPT = /^gpt-(\d+(?:\.\d+)?[a-z]?)((?:-[a-z]+)*)$/;
const GEMINI = /^gemini-(\d+(?:\.\d+)?)((?:-[a-z]+)*)$/;
/** A local tag with nothing after its size: `qwen3.8:27b`, `qwen3.8-27b`, `gemma4:31b`. */
const QWEN = /^qwen(\d+(?:\.\d+)?)[:-](\d+(?:\.\d+)?)b$/;
const GEMMA = /^gemma(\d+(?:\.\d+)?)[:-](\d+(?:\.\d+)?)b$/;

/**
 * Databricks names an endpoint `databricks-<model>`, with a dash where the vendor writes a dot
 * (`databricks-gpt-5-6-sol`, `databricks-gemini-3-1-pro`). Only there: OpenAI's own `gpt-4-0613`
 * is a dated GPT-4, not a GPT-4.0613.
 */
function withoutDatabricksPrefix(id: string): string {
  if (!id.startsWith('databricks-')) return id;
  return id.slice('databricks-'.length).replace(/^(gpt|gemini)-(\d+)-(\d+)(?=-|$)/, '$1-$2.$3');
}

/** Words OpenAI writes in lower case after the version (`GPT-5 mini`). */
const GPT_LOWER = new Set(['mini', 'nano']);

function familyName(short: string): string | null {
  const id = withoutDatabricksPrefix(short.toLowerCase());
  const named = CLAUDE_NAMED.exec(id);
  if (named) {
    const [, tier, major, minor] = named;
    return `Claude ${capitalise(tier)} ${minor ? `${major}.${minor}` : major}`;
  }
  const numbered = CLAUDE_NUMBERED.exec(id);
  if (numbered) {
    const [, major, minor, tier] = numbered;
    return `Claude ${minor ? `${major}.${minor}` : major} ${capitalise(tier)}`;
  }
  const gpt = GPT.exec(id);
  if (gpt) {
    const [, version, rest] = gpt;
    const words = rest.split('-').filter(Boolean);
    // OpenAI joins the Codex line with hyphens: `GPT-5.3-Codex`, `GPT-5.1-Codex-Max`.
    if (words[0] === 'codex') {
      return `GPT-${version}-${words.map(capitalise).join('-')}`;
    }
    const tail = words.map((word) => (GPT_LOWER.has(word) ? word : capitalise(word)));
    return [`GPT-${version}`, ...tail].join(' ');
  }
  const gemini = GEMINI.exec(id);
  if (gemini) {
    const [, version, rest] = gemini;
    const words = rest.split('-').filter(Boolean).map(capitalise);
    return [`Gemini ${version}`, ...words].join(' ');
  }
  const qwen = QWEN.exec(id);
  if (qwen) return `Qwen${qwen[1]} ${qwen[2]}B`;
  const gemma = GEMMA.exec(id);
  if (gemma) return `Gemma ${gemma[1]} ${gemma[2]}B`;
  return null;
}
