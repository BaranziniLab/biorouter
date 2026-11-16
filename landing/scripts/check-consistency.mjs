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
// The release step adds a row at the top of the list but does not move the
// oldest one into "Earlier updates", so the visible list would grow by a row a
// release. The newest five stay visible; the rest sit inside the disclosure.
const visibleNews = (about.split('<details class="news-more">')[0].split('class="news-list">')[1] || '').match(/class="news-row"/g) || [];
check(
  visibleNews.length === 5,
  `about.html lists ${visibleNews.length} updates before "Earlier updates"; keep the newest five there and move the oldest into .news-list-earlier`
);
for (const page of [index, download, docs, about, content]) {
  check(!page.includes('1.88.2'), 'landing content should not advertise the retracted 1.88.2 release');
}

const providerChecks = [
  ['crates/biorouter/src/providers/versa_azure.rs', /VERSA_AZURE_DEFAULT_MODEL: &str = "([^"]+)"/, 'Versa Azure'],
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
  if (['OpenAI', 'Ollama', 'Versa Azure', 'Versa Bedrock'].includes(name)) {
    includes(mockups, value, `mockups should include ${name} default ${value}`);
  }
}

// The catalog is read from MODEL_CATALOG itself, not from every `name:` in the
// file: RETIRED_MODELS carries `name:` fields too, and those models are exactly
// the ones the site must stop advertising.
const llamaSource = source('crates/biorouter/src/providers/llamacpp.rs');
const llamaCatalog = capture(
  llamaSource,
  /pub const MODEL_CATALOG: &\[CatalogEntry\] = &\[([\s\S]*?)\n\];/,
  'MODEL_CATALOG in crates/biorouter/src/providers/llamacpp.rs'
);
const llamaEntries = [...llamaCatalog.matchAll(/name: "([^"]+)",\s*display_name: "([^"]+)"/g)].map((match) => ({
  name: match[1],
  displayName: match[2],
}));
check(llamaEntries.length > 0, 'Llama Server catalog should contain named entries');
for (const { name, displayName } of llamaEntries) {
  includes(docs, displayName, `docs should include Llama Server model ${displayName}`);
  includes(docs, `<code>${name}</code>`, `docs should name Llama Server model ${name}`);
}
// The default is one model on every memory tier (DEFAULT_MODEL, returned by
// recommended_model_for_memory_gib), so the mockup names that model. This list
// used to be hardcoded ('Gemma 4 E4B', 'Gemma 4 12B') and kept passing after
// the catalog dropped both, because the stale mockup still said them.
const llamaDefault = capture(llamaSource, /pub const DEFAULT_MODEL: &str = "([^"]+)"/, 'Llama Server DEFAULT_MODEL');
const llamaDefaultEntry = llamaEntries.find((entry) => entry.name === llamaDefault);
check(Boolean(llamaDefaultEntry), `Llama Server DEFAULT_MODEL ${llamaDefault} should be a MODEL_CATALOG entry`);
if (llamaDefaultEntry) {
  includes(
    mockups,
    llamaDefaultEntry.displayName,
    `mockups should include the Llama Server default model ${llamaDefaultEntry.displayName}`
  );
}
// Ollama's supported pair: the default is checked with the other providers'
// defaults above; the second tag is the only other literal in the list.
const ollamaRecommended = [
  ...capture(
    source('crates/biorouter/src/providers/ollama.rs'),
    /pub const OLLAMA_RECOMMENDED_MODELS: &\[RecommendedOllamaModel\] = &\[([\s\S]*?)\n\];/,
    'OLLAMA_RECOMMENDED_MODELS in crates/biorouter/src/providers/ollama.rs'
  ).matchAll(/tag: "([^"]+)"/g),
].map((match) => match[1]);
for (const tag of ollamaRecommended) {
  includes(docs, tag, `docs should include the supported Ollama model ${tag}`);
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

// ---- Every animation a page shows is loaded on that page -------------------
// An art tile is a `<div data-art="NAME">`, filled by `art/NAME.js` (which calls
// `BR.art('NAME', …)`) and styled by `art/NAME.css`. A page that forgets either
// tag shows an empty white frame, and nothing else notices: the guard below is
// the only reader of the pairing.
for (const [label, page] of [['index.html', index], ['download.html', download]]) {
  for (const name of new Set([...page.matchAll(/data-art="([^"]+)"/g)].map((match) => match[1]))) {
    includes(page, `<script src="art/${name}.js"></script>`, `${label} shows data-art="${name}" but does not load art/${name}.js`);
    includes(page, `<link rel="stylesheet" href="art/${name}.css">`, `${label} shows data-art="${name}" but does not load art/${name}.css`);
    let js = '';
    try { js = landing(`art/${name}.js`); } catch { failures.push(`art/${name}.js is missing`); }
    check(!js || js.includes(`BR.art('${name}',`), `art/${name}.js does not register BR.art('${name}', …)`);
  }
}

// ---- Ways to use Biorouter (download.html#ways) ---------------------------
// Five tiles, each showing a way in. Every string a tile quotes from the
// product is read back from the product here, so a renamed hint, banner or
// placeholder fails this check rather than leaving the site showing a screen
// that no longer exists. The questions and answers in the tiles are
// illustrations and are not checked.
const WAYS = ['access-gui', 'access-tui', 'access-cli', 'access-serve', 'access-jupyter'];
const waysSection = (() => {
  const start = download.indexOf('<section class="section wrap" id="ways"');
  check(start !== -1, 'download.html should carry the "Ways to use Biorouter" section (id="ways")');
  if (start === -1) return '';
  return download.slice(start, download.indexOf('</section>', start));
})();
const waysTiles = [...waysSection.matchAll(/data-art="([^"]+)"/g)].map((match) => match[1]);
check(
  JSON.stringify(waysTiles) === JSON.stringify(WAYS),
  `download.html#ways should show ${WAYS.join(', ')} in that order (found ${waysTiles.join(', ') || 'none'})`
);
for (const figure of waysSection.split('<figure').slice(1)) {
  const name = (/data-art="([^"]+)"/.exec(figure) || [])[1] || '(unnamed)';
  check(/role="img" aria-label="[^"]{20,}"/.test(figure), `${name}: the tile needs role="img" and an aria-label that says what it shows`);
  check(/<figcaption><h3>[^<]+<\/h3><p>[^<]+<\/p><a class="text-link" href="[^"]+">/.test(figure), `${name}: the caption is a title, one sentence and where to get it`);
  const href = (/<a class="text-link" href="([^"]+)"/.exec(figure) || [])[1];
  if (!href) continue;
  if (href.startsWith('#')) {
    includes(download, `id="${href.slice(1)}"`, `${name}: the link ${href} names no section of download.html`);
  } else if (href.startsWith('docs.html#')) {
    includes(docs, `id="${href.slice('docs.html#'.length)}"`, `${name}: the link ${href} names no section of docs.html`);
  } else {
    const blob = /^https:\/\/github\.com\/BaranziniLab\/biorouter\/blob\/main\/(.+)$/.exec(href);
    check(Boolean(blob), `${name}: the link ${href} should go to this page, docs.html or a file in the repository`);
    if (blob) {
      let ok = true;
      try { source(blob[1]); } catch { ok = false; }
      check(ok, `${name}: the link names ${blob[1]}, which is not in the repository`);
    }
  }
}

const art = (name) => landing(`art/${name}.js`);
const jsString = (js, name, label) => capture(js, new RegExp(`var ${name} = '([^']*)'`), `${name} in ${label}`);
const jsList = (js, name, label) =>
  [...capture(js, new RegExp(`var ${name} = \\[([\\s\\S]*?)\\];`), `${name} in ${label}`).matchAll(/'([^']*)'/g)].map((match) => match[1]);
const tuiSource = source('crates/biorouter-cli/src/session/tui/mod.rs');

// Terminal chat: the banner, the greeting, the placeholder, the spinner and its
// label, and the key hints, as the TUI draws them.
{
  const tile = art('access-tui');
  const label = 'art/access-tui.js';
  const ascii = [
    ...capture(
      source('crates/biorouter-cli/src/session/output.rs'),
      /pub\(crate\) const BIOROUTER_ASCII: &\[&str\] = &\[([\s\S]*?)\];/,
      'BIOROUTER_ASCII in crates/biorouter-cli/src/session/output.rs'
    ).matchAll(/"([^"]*)"/g),
  ].map((match) => match[1]);
  check(JSON.stringify(jsList(tile, 'BANNER', label)) === JSON.stringify(ascii), `${label}: BANNER differs from BIOROUTER_ASCII in output.rs`);
  for (const name of ['TAGLINE', 'ENTER', 'PLACEHOLDER']) {
    const value = jsString(tile, name, label);
    includes(tuiSource, `"${value}"`, `${label}: ${name} "${value}" is not a string the TUI draws (session/tui/mod.rs)`);
  }
  const spinner = [...capture(tuiSource, /const SPINNER: &\[&str\] = &\[([^\]]*)\];/, 'SPINNER in session/tui/mod.rs').matchAll(/"([^"]*)"/g)].map((match) => match[1]);
  check(JSON.stringify(jsList(tile, 'SPINNER', label)) === JSON.stringify(spinner), `${label}: SPINNER differs from the TUI's frames`);
  const thinking = jsString(tile, 'THINKING', label);
  includes(source('crates/biorouter-cli/src/session/thinking.rs'), `"${thinking}"`, `${label}: "${thinking}" is not one of thinking.rs's labels`);
  // hint_text's candidates, with no dictation key bound (`{dictate}` empty).
  const hintBody = capture(tuiSource, /(fn hint_text\([\s\S]*?\n\}\n)/, 'hint_text in session/tui/mod.rs');
  const candidates = new Set([...hintBody.matchAll(/"((?:↵|esc|↑↓|\{summary\})[^"]*)"/g)].map((match) => match[1].replace(/\{dictate\}/g, '')));
  for (const hint of jsList(tile, 'HINTS', label)) {
    check(candidates.has(hint), `${label}: the hint line "${hint}" is not one of hint_text's candidates`);
  }
}

// Command line: the flags the two runs use, and the commands the docs give.
{
  const tile = art('access-cli');
  const cli = source('crates/biorouter-cli/src/cli.rs');
  includes(cli, `short = 'q',\n        long = "quiet"`, "art/access-cli.js uses -q, which cli.rs should still define as --quiet");
  includes(cli, `short = 't',\n        long = "text"`, "art/access-cli.js uses -t, which cli.rs should still define as --text");
  includes(cli, 'help = "Path to instruction file containing commands. Use - for stdin."', 'art/access-cli.js reads stdin with -i -, which cli.rs should still allow');
  for (const cmd of [...tile.matchAll(/\{ cmd: '([^']+)' \}/g)].map((match) => match[1]).filter((cmd) => cmd.includes('biorouter'))) {
    // The docs show each run without -q; the tile adds it so only the answer prints.
    const plain = cmd.replace(' -q', '');
    const shown = plain.startsWith('cat ') ? plain : plain.replace(/^biorouter run -t "/, '');
    includes(docs, shown.replace(/"$/, ''), `art/access-cli.js runs \`${cmd}\`, which docs.html#cli-run should show`);
  }
}

// Browser: what `biorouter serve` prints, and the interface it opens.
{
  const tile = art('access-serve');
  const serve = source('crates/biorouter-cli/src/commands/serve.rs');
  includes(serve, 'Biorouter is serving at', 'serve.rs should still print "Biorouter is serving at", which art/access-serve.js shows');
  includes(serve, 'Press Ctrl-C to stop.', 'serve.rs should still print "Press Ctrl-C to stop.", which art/access-serve.js shows');
  includes(tile, 'Biorouter is serving at', 'art/access-serve.js should show the line serve prints');
  const port = capture(serve, /pub const DEFAULT_PORT: u16 = (\d+);/, 'DEFAULT_PORT in serve.rs');
  includes(tile, `127.0.0.1:${port}`, `art/access-serve.js should show serve's default port ${port}`);
  for (const nav of ['Home', 'New chat', 'Skills']) {
    check(sidebarLabels.includes(nav), `art/access-serve.js draws the sidebar row "${nav}", which AppSidebar.tsx no longer declares`);
  }
  includes(source('ui/desktop/src/components/BioRouterSidebar/copy.ts'), "header: 'Chats'", "art/access-serve.js draws the sidebar's Chats header");
}

// Desktop app and the browser's composer: the placeholders, the chat's tab and
// the figure's tool row, as the app writes them.
{
  const composer = source('ui/desktop/src/components/composer/copy.ts');
  const gui = art('access-gui');
  for (const [name, key] of [['PH_EMPTY', 'placeholderEmpty'], ['PH_FOLLOW', 'placeholderFollowUp']]) {
    const value = jsString(gui, name, 'art/access-gui.js');
    includes(composer, `${key}: '${value}'`, `art/access-gui.js: ${name} "${value}" should be composer/copy.ts's ${key}`);
    includes(art('access-serve'), value, `art/access-serve.js should use the composer placeholder "${value}"`);
  }
  includes(gui, 'acc-gui-tab-name">New chat<', 'art/access-gui.js should name the chat New chat');
  includes(source('ui/desktop/src/utils/sessionNameSync.ts'), "DEFAULT_SESSION_NAME = 'New chat'", "art/access-gui.js's tab is DEFAULT_SESSION_NAME");
  const copy = source('ui/desktop/src/components/toolCallCopy.ts');
  includes(copy, "chart: 'chart',", 'FIGURE_NAMES should still call the chart kind "chart" (art/access-gui.js shows "Rendering chart")');
  includes(copy, 'render: (figure: string) => `Rendering ${figure}`', 'FIGURE_COPY.render should still read "Rendering <figure>"');
  includes(copy, "Rendering: ['Rendered', 'render']", 'the finished row should still read "Rendered <figure>"');
  includes(gui, 'Rendering chart</span>', 'art/access-gui.js should show the running row');
  includes(gui, 'Rendered chart</span>', 'art/access-gui.js should show the finished row');
}

// JupyterLab: the persona's name, and a tool title written the way
// `biorouter acp` writes one.
{
  const tile = art('access-jupyter');
  includes(source('integrations/jupyter-ai/jupyter_ai_biorouter/persona.py'), `name="${jsString(tile, 'PERSONA', 'art/access-jupyter.js')}"`, "art/access-jupyter.js should use the persona's name");
  const acp = source('crates/biorouter-acp/src/server.rs');
  const formatter = capture(acp, /fn format_tool_name\(tool_name: &str\) -> String \{([\s\S]*?)\n\}/, 'format_tool_name in crates/biorouter-acp/src/server.rs');
  check(
    formatter.includes('split_once("__")') && formatter.includes(".replace('_', \" \")") && formatter.includes('"{}: {}"'),
    'format_tool_name changed shape; check the title art/access-jupyter.js shows'
  );
  const title = 'code_execution__execute_code'
    .split('__')
    .map((part) => part.split('_').map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(' '))
    .join(': ');
  check(jsString(tile, 'TOOL', 'art/access-jupyter.js') === title, `art/access-jupyter.js should title the call "${title}"`);
}

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
