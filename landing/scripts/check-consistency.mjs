#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkDocsPrivacy } from './check-docs-privacy.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const BIOROUTER = resolve(process.env.BIOROUTER_REPO || join(ROOT, '..'));

const read = (path) => readFileSync(path, 'utf8');
const landing = (path) => read(join(ROOT, path));
const source = (path) => read(join(BIOROUTER, path));

const failures = [];
const check = (condition, message) => {
  if (!condition) failures.push(message);
};
const includes = (text, needle, message) => check(text.includes(needle), message || `missing ${needle}`);
const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
// A display name is compared against HTML, where `Web & Documents` is written
// `Web &amp; Documents`. Comparing the raw name never matched it, so the one
// bundled extension with an ampersand always read as absent.
const escapeHtml = (value) => value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const capture = (text, re, label) => {
  const match = re.exec(text);
  if (!match) throw new Error(`Could not find ${label}`);
  return match[1];
};

const docs = landing('docs.html');
const index = landing('index.html');
const download = landing('download.html');
const about = landing('about.html');
const content = landing('assets/landing-site-content.md');
const mockups = landing('app-mockups.js');
const baam = landing('baam.html');
const registry = JSON.parse(landing('registry.json'));

// ---- The BAAM private set, in the three places it is committed -------------
// `landing/registry.json` (what the marketplace publishes),
// `ui/desktop/src/components/baam/registry.fallback.json` (the snapshot bundled
// in the desktop app) and `crates/biorouter/src/privacy/registry_private.rs`
// (the set compiled into the CLI and the daemon) each carry the extensions
// tagged private. `build-registry.mjs` writes all three in one run, which is not
// the same as their still agreeing: a hand edit to the Rust file, or a rebase
// that took one side of a conflict, leaves the app enforcing a different set
// from the one the marketplace publishes — and the failure is silent, because an
// extension missing from the compiled-in set is simply classified Public and
// admitted to a public session.
//
// `--check` runs the privacy checks and nothing else — this one and the
// docs.html table check below it — which is what lets it be wired into `just
// check-everything` and the landing deploy: the landing-copy checks further
// down track a published site that legitimately lags the app's main branch, so
// a mode that ran them too could never be a gate. Anything added to `--check`
// has to hold that property: it may read only files in this repo.
const PRIVACY_ONLY = process.argv.slice(2).includes('--check');

// The form `classify_extension` reduces its argument to before the lookup —
// mirrors `name_to_key` in crates/biorouter/src/privacy/extensions.rs, so an
// entry matches either spelling the registry publishes (id or manifest name).
const nameToKey = (value) => value.replace(/\s+/g, '').toLowerCase();

const privateKeysOf = (catalog, label) => {
  const extensions = catalog.extensions || [];
  check(extensions.length > 0, `${label} publishes no extensions at all`);
  const keys = [];
  for (const ext of extensions) {
    check(
      ext.privacy === 'private' || ext.privacy === 'public',
      `${label}: extension ${ext.id || '(unnamed)'} declares no privacy tier`
    );
    if (ext.privacy !== 'private') continue;
    const join = ext.extension_name || ext.id;
    check(Boolean(join), `${label}: a private extension carries no join key`);
    if (join) keys.push(nameToKey(join));
  }
  return keys.sort();
};

const rustPrivateKeys = () => {
  const rust = source('crates/biorouter/src/privacy/registry_private.rs');
  const body = capture(
    rust,
    /pub const PRIVATE_EXTENSIONS: &\[&str\] = &\[([\s\S]*?)\];/,
    'PRIVATE_EXTENSIONS in crates/biorouter/src/privacy/registry_private.rs'
  );
  return [...body.matchAll(/"([^"]*)"/g)].map((match) => match[1]).sort();
};

const publishedPrivate = privateKeysOf(registry, 'landing/registry.json');
const bundledPrivate = privateKeysOf(
  JSON.parse(source('ui/desktop/src/components/baam/registry.fallback.json')),
  'ui/desktop/src/components/baam/registry.fallback.json'
);
const compiledPrivate = rustPrivateKeys();
// An empty private set is not "there are no private extensions"; it is a gate
// that admits everything, and it satisfies every equality below unless it is
// named separately.
check(
  publishedPrivate.length > 0,
  'landing/registry.json tags no extension private — the tier gate would admit every extension'
);
const showKeys = (keys) => (keys.length ? keys.join(', ') : '(none)');
const regenerate = 'run `node landing/scripts/build-registry.mjs` to regenerate all three';
check(
  JSON.stringify(bundledPrivate) === JSON.stringify(publishedPrivate),
  `private set drift: registry.json has [${showKeys(publishedPrivate)}], ` +
    `registry.fallback.json has [${showKeys(bundledPrivate)}] — ${regenerate}`
);
check(
  JSON.stringify(compiledPrivate) === JSON.stringify(publishedPrivate),
  `private set drift: registry.json has [${showKeys(publishedPrivate)}], ` +
    `registry_private.rs has [${showKeys(compiledPrivate)}] — ${regenerate}`
);

// `docs.html`'s "Extension agents in the marketplace" table is hand-written and
// ungenerated, so it is the one privacy surface with no producer to keep it
// honest. It belongs in the gated `--check` half rather than the landing-copy
// half below: it reads only files in this repo, so it can never fail merely
// because the published site lags main.
for (const failure of checkDocsPrivacy(docs, registry)) failures.push(failure);

if (PRIVACY_ONLY) {
  report(
    `BAAM private set agrees in all three copies (${showKeys(publishedPrivate)}), ` +
      "and docs.html's agents table agrees with the registry."
  );
}

const cargoVersion = capture(source('Cargo.toml'), /version = "([^"]+)"/, 'Cargo workspace version');
const packageVersion = capture(source('ui/desktop/package.json'), /"version": "([^"]+)"/, 'desktop package version');
const landingVersion = capture(content, /\*\*Version:\*\* v([^\s]+)/, 'landing release version');
check(cargoVersion === packageVersion, `Biorouter version mismatch: Cargo ${cargoVersion}, package ${packageVersion}`);
includes(docs, `biorouter v${landingVersion}`, 'docs provider defaults should cite the latest published Biorouter version');
includes(docs, `biorouter v${landingVersion}`, 'docs extension defaults should cite the latest published Biorouter version');
includes(index, `v${landingVersion}`, 'index fallback release version should match the latest published version');
includes(download, `v${landingVersion}`, 'download fallback badge should match the latest published version');
includes(download, `Biorouter-${landingVersion}-arm64.dmg`, 'download macOS arm64 fallback should match release asset');
includes(download, `Biorouter-win32-x64-${landingVersion}.zip`, 'download Windows fallback should match release asset');
includes(about, `releases/tag/v${landingVersion}`, 'about news should link to the latest published release');
for (const page of [index, download, docs, about, content]) {
  check(!page.includes('1.88.2'), 'landing content should not advertise the retracted 1.88.2 release');
}

const providerChecks = [
  ['crates/biorouter/src/providers/versa_bedrock.rs', /VERSA_BEDROCK_DEFAULT_MODEL: &str = "([^"]+)"/, 'Versa Bedrock'],
  ['crates/biorouter/src/providers/openai.rs', /OPEN_AI_DEFAULT_MODEL: &str = "([^"]+)"/, 'OpenAI'],
  ['crates/biorouter/src/providers/anthropic.rs', /ANTHROPIC_DEFAULT_MODEL: &str = "([^"]+)"/, 'Anthropic'],
  ['crates/biorouter/src/providers/azure.rs', /AZURE_DEFAULT_MODEL: &str = "([^"]+)"/, 'Azure OpenAI'],
  ['crates/biorouter/src/providers/google.rs', /GOOGLE_DEFAULT_MODEL: &str = "([^"]+)"/, 'Google Gemini'],
  ['crates/biorouter/src/providers/xai.rs', /XAI_DEFAULT_MODEL: &str = "([^"]+)"/, 'X.AI'],
  ['crates/biorouter/src/providers/openrouter.rs', /OPENROUTER_DEFAULT_MODEL: &str = "([^"]+)"/, 'OpenRouter'],
  ['crates/biorouter/src/providers/ollama.rs', /OLLAMA_DEFAULT_MODEL: &str = "([^"]+)"/, 'Ollama'],
];

for (const [path, re, name] of providerChecks) {
  const value = capture(source(path), re, name);
  includes(docs, value, `docs should include ${name} default ${value}`);
  if (['OpenAI', 'Ollama', 'Versa Bedrock'].includes(name)) {
    includes(mockups, value, `mockups should include ${name} default ${value}`);
  }
}

const llamaSource = source('crates/biorouter/src/providers/llamacpp.rs');
const llamaModels = [...llamaSource.matchAll(/display_name: "([^"]+)"/g)].map((match) => match[1]);
check(llamaModels.length > 0, 'Llama Server catalog should contain display names');
for (const model of llamaModels) {
  includes(docs, model, `docs should include Llama Server model ${model}`);
}
// Memory-tiered defaults from recommended_model_for_memory_gib in
// crates/biorouter/src/providers/llamacpp.rs: Gemma 4 E4B below 64 GiB,
// Gemma 4 12B (fast dense) on high-memory systems (issue #35 replaced the
// Qwen3.6 35B MoE default there).
for (const recommended of ['Gemma 4 E4B', 'Gemma 4 12B']) {
  includes(mockups, recommended, `mockups should include memory-tiered Llama Server recommendation ${recommended}`);
}

// Derive the rail from the app rather than keeping a third copy of the list.
// The hardcoded copy asserted 'Chat', 'History' and 'Apps', three rows the app
// has never had under those names (they are 'New chat', reached from Recents,
// and 'Built apps'), so it was checking the mockup against a list that matched
// neither the app nor the site, and no edit to the site could turn it green.
const appSidebar = source('ui/desktop/src/components/BioRouterSidebar/AppSidebar.tsx');
const sidebarLabels = [...appSidebar.matchAll(/label: '([^']+)'/g)].map((match) => match[1]);
check(
  sidebarLabels.length >= 8,
  `expected the app sidebar to declare its rows as "label: '…'" (found ${sidebarLabels.length}); ` +
    'if that shape changed, update this reader rather than reinstating a hand-written list'
);
for (const label of sidebarLabels) {
  includes(mockups, `label: '${label}'`, `mockup sidebar should include ${label}`);
}

const bundled = JSON.parse(source('ui/desktop/src/components/settings/extensions/bundled-extensions.json'));
// docs.html holds several tables and two of them carry a `Knowledge` row, so
// the default-state lookup is scoped to the built-in extensions table.
const builtinTable = (() => {
  const head = docs.indexOf('<th>Extension</th><th>Default</th>');
  check(head !== -1, 'docs should carry a built-in extensions table with a Default column');
  if (head === -1) return '';
  const end = docs.indexOf('</table>', head);
  return docs.slice(head, end === -1 ? undefined : end);
})();
for (const ext of bundled) {
  const name = escapeHtml(ext.display_name);
  includes(docs, name, `docs should include bundled extension ${ext.display_name}`);
  includes(mockups, name, `mockup should include bundled extension ${ext.display_name}`);
  const expected = ext.enabled ? 'On' : 'Off';
  // Default is the SECOND cell of a four-column row (name, default,
  // description, key tools). The regex used to end at `</td></tr>`, which made
  // it capture the LAST cell instead, so every row failed on the key-tools text
  // and real drift could not be seen inside the noise.
  const rowRe = new RegExp(
    `<tr><td>(?:<strong>)?${escapeRegExp(name)}(?:<\\/strong>)?<\\/td><td>(.*?)<\\/td>`
  );
  const docRow = rowRe.exec(builtinTable);
  // A cell may qualify the state ("On; task approval required"), so the word
  // that opens it is what is checked.
  const state = docRow && docRow[1].replace(/<[^>]+>/g, '').trim();
  check(
    Boolean(state) && new RegExp(`^${expected}\\b`).test(state),
    `docs default state for ${ext.display_name} should be ${expected}`
  );
}

includes(docs, '--schedule-id review-weekly', 'scheduler docs should use named schedule ID argument');
includes(docs, '--workflow-source ./review.yaml', 'scheduler docs should show workflow source argument');
includes(docs, "history.replaceState(null, '', '#' + name)", 'docs navigation should keep URL hash in sync');
const baamBeforeScript = baam.split('<script>')[0];
check(registry.extensions.length === (baamBeforeScript.match(/<div[^>]*class="ext-card(?:\s|")/g) || []).length, 'registry extension count should match BAAM fallback cards');

for (const stale of ['v1.85.0', 'v1.80.0', 'claude-opus-4-1', 'claude-sonnet-4-20250514', 'grok-code-fast-1</code>', 'gpt-5.2</code>']) {
  check(!index.includes(stale) && !download.includes(stale) && !docs.includes(stale), `stale token remains: ${stale}`);
}

report('Landing consistency checks passed.');

// A function declaration so it hoists: `--check` reports and exits from the top
// of the file, long before this line is reached.
function report(passed) {
  if (failures.length) {
    console.error(failures.map((failure) => `- ${failure}`).join('\n'));
    process.exit(1);
  }
  console.log(passed);
  process.exit(0);
}
