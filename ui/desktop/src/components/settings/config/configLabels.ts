import { configLabels } from '../../../utils/configUtils';

/**
 * The names the Configuration editor puts on its fields.
 *
 * The key itself (`BIOROUTER_AUTO_COMPACT_THRESHOLD`) is a machine string and lives in the label's
 * tooltip, in the mono face; the label is a plain name in sentence case, so a column of them reads
 * as one list. The provider setup form names a key through `providerFieldName`, which runs the
 * curated Title Case label through the exported `sentenceCase` below; `utils/configUtils.getUiNames`
 * is not used by either. This file is the editor's own naming: the same known names and a better
 * fallback. Keep `sentenceCase` exported as `(label: string) => string`: the provider dialog
 * imports it, and its name cases (OpenAI, MiMo, Llama Server, API, CLI, GCP, ID) depend on it.
 *
 * - The brand prefix goes: every key in this file belongs to Biorouter.
 * - Acronyms and product names keep their own spelling (AWS, URL, OpenAI, GitHub).
 * - Every other word is lower case, and the first is capitalised.
 */

/** Words written in capitals wherever they stand. */
const ACRONYMS: ReadonlySet<string> = new Set([
  'aws',
  'api',
  'cli',
  'cpu',
  'csv',
  'dns',
  'gcp',
  'gpu',
  'http',
  'https',
  'iam',
  'id',
  'ip',
  'json',
  'kb',
  'llm',
  'mcp',
  'ocr',
  'pii',
  'sdk',
  'sql',
  'ssl',
  'tls',
  'ttl',
  'ucsf',
  'ui',
  'uri',
  'url',
  'vpc',
  'yaml',
]);

/** Words with a spelling of their own (a product, a brand, or a short name for an internal one). */
const SPELLINGS: Readonly<Record<string, string>> = {
  anthropic: 'Anthropic',
  azure: 'Azure',
  bedrock: 'Bedrock',
  biorouter: 'Biorouter',
  brsdk: 'App SDK',
  claude: 'Claude',
  codex: 'Codex',
  databricks: 'Databricks',
  github: 'GitHub',
  google: 'Google',
  groq: 'Groq',
  llamacpp: 'Llama Server',
  mimo: 'MiMo',
  ollama: 'Ollama',
  openai: 'OpenAI',
  openrouter: 'OpenRouter',
  snowflake: 'Snowflake',
  versa: 'Versa',
  vertex: 'Vertex',
  xiaomi: 'Xiaomi',
};

/** Capitalised words that stay capitalised when they are not the first word of a label. */
const PROPER_WORDS: ReadonlySet<string> = new Set([
  'Anthropic',
  'Azure',
  'Biorouter',
  'Claude',
  'Codex',
  'Databricks',
  'Google',
  'Groq',
  'Llama',
  'Ollama',
  'Snowflake',
  'Versa',
  'Xiaomi',
]);

/** Pairs whose second word is part of a name: "Llama Server port", not "Llama server port". */
const PROPER_PAIRS: ReadonlySet<string> = new Set(['Llama Server']);

/**
 * Sentence case for a Title Case name: the first word keeps its capital, a later word that is a
 * plain capitalised word ("Provider", "Threshold") goes to lower case, and anything with another
 * capital or a digit in it (OpenAI, MiMo, GCP, ID, z.ai) or on the proper-name lists stays.
 */
export function sentenceCase(label: string): string {
  const words = label.split(' ');
  return words
    .map((word, index) => {
      if (index === 0) return word;
      const bare = word.replace(/^[^A-Za-z]+|[^A-Za-z]+$/g, '');
      if (!/^[A-Z][a-z]+$/.test(bare)) return word;
      if (PROPER_WORDS.has(bare)) return word;
      if (PROPER_PAIRS.has(`${words[index - 1]} ${bare}`)) return word;
      return word.replace(bare, bare.toLowerCase());
    })
    .join(' ');
}

/** `computer_use_capabilities_version` as the sentence it abbreviates. */
function humaniseKey(key: string): string {
  const words = key.split(/[_\s-]+/).filter(Boolean);
  // Everything in this file is Biorouter's, so a leading brand word says nothing.
  const named = words.length > 1 && words[0].toLowerCase() === 'biorouter' ? words.slice(1) : words;
  const text = named
    .map((word) => {
      const lower = word.toLowerCase();
      if (SPELLINGS[lower]) return SPELLINGS[lower];
      if (ACRONYMS.has(lower)) return lower.toUpperCase();
      return lower;
    })
    .join(' ');
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** The editor's name for a config key: a known name in sentence case, else the key made readable. */
export function configFieldLabel(key: string): string {
  const known = configLabels[key];
  return known ? sentenceCase(known) : humaniseKey(key);
}
