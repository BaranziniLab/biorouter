// Run: node --test landing/scripts/provider-defaults.test.mjs
//
// The providers page says its defaults are what the published release picks
// (`v{{LATEST_VERSION}}`), but the page deploys on every push to main that
// touches it, so it can name a default the published app does not pick yet.
// #393 moved the Llama Server default to Qwen3.8 27B and Ollama's to
// `qwen3.8:27b` on main while 1.92.1 was the published app, and the page named
// both as plain defaults: every 1.92.1 user read a default their app does not
// pick. The Versa API Azure row already said which release picks which ("after
// v1.92.1", "in v1.92.1 and earlier"); this holds every provider to that rule.
//
// The published defaults come from the release's git tag, and the tree's from
// the source. Without the tag (a shallow clone) the page test skips; the rule's
// own cases below need no git.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import assert from 'node:assert/strict';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const read = (path) => readFileSync(join(repo, path), 'utf8');
const PAGE = 'docs/website/pages/providers.html';

// Every provider whose default the page states, by the constant that holds it.
// Llama Server's `DEFAULT_MODEL` is new after v1.92.1; before it, the model was
// picked by memory in `recommended_model_for_memory_gib` (see `defaultsIn`).
const PROVIDERS = [
  ['Llama Server', 'llamacpp.rs', 'DEFAULT_MODEL'],
  ['Ollama', 'ollama.rs', 'OLLAMA_DEFAULT_MODEL'],
  ['Versa API Azure', 'versa_azure.rs', 'VERSA_AZURE_DEFAULT_MODEL'],
  ['Versa API Bedrock', 'versa_bedrock.rs', 'VERSA_BEDROCK_DEFAULT_MODEL'],
  ['Anthropic', 'anthropic.rs', 'ANTHROPIC_DEFAULT_MODEL'],
  ['OpenAI', 'openai.rs', 'OPEN_AI_DEFAULT_MODEL'],
  ['Google Gemini', 'google.rs', 'GOOGLE_DEFAULT_MODEL'],
  ['Azure OpenAI', 'azure.rs', 'AZURE_DEFAULT_MODEL'],
  ['X.AI', 'xai.rs', 'XAI_DEFAULT_MODEL'],
  ['OpenRouter', 'openrouter.rs', 'OPENROUTER_DEFAULT_MODEL'],
];

/** The model names a provider source file defaults to: one, or one per memory tier. */
export function defaultsIn(rust, constant) {
  const pinned = new RegExp(`pub const ${constant}: &str = "([^"]+)"`).exec(rust);
  if (pinned) return [pinned[1]];
  if (constant !== 'DEFAULT_MODEL') return [];
  const tiered = /fn recommended_model_for_memory_gib\([^)]*\)[^{]*\{([\s\S]*?)\n\}/.exec(rust);
  if (!tiered) return [];
  const body = tiered[1].replace(/\/\/.*$/gm, '');
  return [...new Set([...body.matchAll(/"([^"]+)"/g)].map((match) => match[1]))];
}

const plain = (html) =>
  html
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();

/**
 * The page's sentences and table rows, each marked when it states a default:
 * a row of a table with a Default column, or any row or paragraph that says
 * "default" itself.
 */
export function units(page) {
  const out = [];
  const rest = page.replace(/<table>([\s\S]*?)<\/table>/g, (_, table) => {
    const head = /<thead>([\s\S]*?)<\/thead>/.exec(table)?.[1] ?? '';
    const column = /\bdefault\b/i.test(plain(head));
    const body = table.replace(/<thead>[\s\S]*?<\/thead>/, '');
    for (const [, row] of body.matchAll(/<tr>([\s\S]*?)<\/tr>/g)) {
      out.push({ html: row, states: column || /\bdefault\b/i.test(plain(row)) });
    }
    return '';
  });
  for (const [, , block] of rest.matchAll(/<(p|li)\b[^>]*>([\s\S]*?)<\/\1>/g)) {
    out.push({ html: block, states: /\bdefault\b/i.test(plain(block)) });
  }
  return out;
}

/**
 * What is wrong with how `page` states one provider's default, when the
 * published `release` picks `before` and the tree picks `after`.
 */
export function qualifierProblems(page, { provider, release, before, after }) {
  if (before.length === 1 && before[0] === after) return [];
  const code = (model) => `<code>${model}</code>`;
  const all = units(page);
  const claims = all.filter((unit) => unit.states && unit.html.includes(code(after)));
  const problems = [];
  if (claims.length === 0) problems.push(`${provider}: the page never states its default ${after}`);
  for (const unit of claims) {
    if (!unit.html.includes(`after v${release}`)) {
      problems.push(
        `${provider}: "${plain(unit.html)}" states ${after} as the default without "after v${release}", ` +
          `but v${release} picks ${before.join(' or ')}`
      );
    }
  }
  for (const model of before) {
    if (
      !all.some(
        (unit) => unit.html.includes(code(model)) && unit.html.includes(`v${release} and earlier`)
      )
    ) {
      problems.push(
        `${provider}: the page should say that v${release} and earlier pick ${code(model)}`
      );
    }
  }
  return problems;
}

const RELEASE = '1.92.1';
const rule = (page) =>
  qualifierProblems(page, {
    provider: 'Ollama',
    release: RELEASE,
    before: ['qwen3'],
    after: 'qwen3.8:27b',
  });

test('a default the published release does not pick yet is flagged', () => {
  assert.equal(rule('<p>Default model: <code>qwen3.8:27b</code>.</p>').length, 2);
  assert.equal(
    rule('<p>Default model: <code>qwen3.8:27b</code> after v1.92.1.</p>').length,
    1,
    'naming the new default without the one the release picks is still half the answer'
  );
});

test('a default qualified the way the Versa API Azure row is passes', () => {
  assert.deepEqual(
    rule(
      '<p>Default model: <code>qwen3.8:27b</code> after v1.92.1, <code>qwen3</code> in v1.92.1 and earlier.</p>'
    ),
    []
  );
  const table =
    '<table><thead><tr><th>Provider</th><th>Default model</th></tr></thead><tbody>' +
    '<tr><td>Ollama</td><td><code>qwen3.8:27b</code></td></tr></tbody></table>' +
    '<p>In v1.92.1 and earlier, the default is <code>qwen3</code>.</p>';
  assert.equal(
    rule(table).length,
    1,
    'a row under a Default column states a default without saying the word'
  );
});

test('an unchanged default needs no qualifier', () => {
  assert.deepEqual(
    qualifierProblems('<p>Default model: <code>qwen3</code>.</p>', {
      provider: 'Ollama',
      release: RELEASE,
      before: ['qwen3'],
      after: 'qwen3',
    }),
    []
  );
});

test('memory-tiered defaults are read from a release that had no constant', () => {
  const rust = [
    "pub fn recommended_model_for_memory_gib(gib: u64) -> &'static str {",
    '    if gib >= 64 {',
    '        // (`qwen3.6`) used to be the "default" here',
    '        "gemma4-12b"',
    '    } else {',
    '        "gemma4"',
    '    }',
    '}',
  ].join('\n');
  assert.deepEqual(defaultsIn(rust, 'DEFAULT_MODEL'), ['gemma4-12b', 'gemma4']);
  assert.deepEqual(
    defaultsIn('pub const DEFAULT_MODEL: &str = "qwen3.8-27b";\n' + rust, 'DEFAULT_MODEL'),
    ['qwen3.8-27b']
  );
  assert.deepEqual(
    defaultsIn('pub const OLLAMA_DEFAULT_MODEL: &str = "qwen3";', 'DEFAULT_MODEL'),
    []
  );
});

const published = /\*\*Version:\*\* v(\d+\.\d+\.\d+)/.exec(
  read('landing/assets/landing-site-content.md')
)?.[1];
const atTag = (path) => {
  try {
    return execFileSync('git', ['-C', repo, 'show', `v${published}:${path}`], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
  } catch {
    return null;
  }
};
const tagged = published && atTag('Cargo.toml');

test(
  'the providers page says which release picks each default that changed since the published one',
  { skip: !tagged && `no v${published} tag in this clone; run git fetch --tags` },
  () => {
    assert.ok(published, 'landing-site-content.md names no published version');
    const page = read(PAGE);
    const problems = [];
    for (const [provider, file, constant] of PROVIDERS) {
      const path = `crates/biorouter/src/providers/${file}`;
      const after = defaultsIn(read(path), constant);
      assert.equal(after.length, 1, `${path} should pin one ${constant}`);
      // An empty `before` would make every provider read as changed, so a
      // reader that stopped matching fails here instead of in the copy.
      const before = defaultsIn(atTag(path) ?? '', constant);
      assert.ok(before.length > 0, `v${published}'s ${path} names no default`);
      problems.push(
        ...qualifierProblems(page, { provider, release: published, before, after: after[0] })
      );
    }
    assert.deepEqual(problems, []);
  }
);

test('the providers page writes memory in GB, as the app does', () => {
  // The app writes every memory amount in GB (`localModelsCopy.gigabytes` in
  // ui/desktop/src/components/settings/models/copy.ts), the unit of the
  // download sizes beside it, though the daemon counts GiB.
  const page = read(PAGE);
  assert.deepEqual(page.match(/[^.>]*\bGiB\b[^.<]*/g) ?? [], []);
});
