#!/usr/bin/env node
// The Crew user manual (`docs/crew/`) and its landing page (`landing/docs.html#crew`)
// are hand-written prose about code that keeps moving, and nothing read them
// against that code. A 2026-09-27 audit of the merged Crew branch found the
// drift this file now refuses, each one a page telling a person something the
// app does not do:
//
//   * DOCS-6: the manual spelled the brand "BioRouter" because it quoted the
//     native approval-secret dialogs verbatim, and those dialogs spelled it so.
//     Rule `brand` keeps the brand "Biorouter" in the manual, and rule
//     `dialog-titles` makes the manual quote every title `main.ts` passes to
//     `promptNativeSecret` exactly, so the dialog and the page can no longer
//     disagree in either direction.
//   * DOCS-1: the manual told a `biorouter serve` browser user how to sign in to
//     Crew, but a serve daemon never holds a user-action key, so every Crew route
//     refuses it (`require_person` in routes/crew.rs answers
//     `crew_human_authority_unavailable`). Rule `serve` makes every passage that
//     names `biorouter serve` say Crew does not work there.
//   * DOCS-1, round 2: a serve browser then stopped reaching the daemon at all.
//     CrewApp shows CrewNeedsDesktop there (CROSSCUT-5), and a daemon that
//     serves the browser refuses a Crew request with its own sentence
//     (CREW_NEEDS_THE_DESKTOP), but the manual still sent a browser reader to
//     "This daemon cannot verify human Crew actions…" and to a sign-in message
//     the browser can no longer show. Rule `needs-desktop` makes the manual
//     quote the page title and the daemon's sentence as the code spells them.
//   * DOCS-2: the manual said the host can add people to any team or channel,
//     but the host's snapshot, like everyone's, holds only the teams and
//     channels the host is in (`read_workspace_snapshot` in
//     crates/biorouter-crew/src/broker.rs). Rule `host-reach` keeps that
//     condition in the "who can do it" tables.
//   * DOCS-5: the repository front page never mentioned Crew. Rule `readme`.
//   * RENDERER-6: the UI redesign spec, marked Current, still said Crew had no
//     drag and drop, plain-text bodies and "Save attachment" buttons, and that
//     nothing was built. Rule `spec` reads the shipped components instead.
//
// The wave-2 fixes of 2026-09-28 reworded many sentences the manual quotes,
// and some of the manual's promises stopped being true (W2-DOC-3, -4, -6, -7,
// -8). The rules below hold the manual to them:
//
//   * `daemon-sentences`: a quote of the daemon's or the workspace server's
//     own sentence (a Crew chat's fixed model, a missing keyring, an outcome
//     that could not be confirmed, a reconnect, a full or failing disk) is
//     that sentence, whole, its first sentences, or its opening words.
//   * `refusal-codes` and `ssh-codes`: every code the daemon refuses with, and
//     every connect failure the command line explains, is in the command-line
//     page, so a script author can look each one up.
//   * `notifications`: the manual said Crew sends no notifications and that a
//     mention notifies nobody, after the desktop app started doing both.
//   * `pause-reasons`: the manual defined "Paused" as "You paused it"; every
//     reason a paused transfer shows is listed.
//   * `cancel-upload`: the manual said an upload cannot be deleted, with no
//     word of the Cancel upload control.
//   * `privacy-confirm`: the manual and the landing page said the privacy
//     commands go public with no typed confirmation (CLI-10).
//   * `share-dialog`: the landing page named the Share window by a title
//     macOS never shows (DW-15).
//   * `settings-title`: the manual named a "Workspace settings" dialog, which
//     no screen shows; its title is the workspace's name, "chen-lab settings"
//     (DW-18).
//   * `product-docs`: the landing page reads here already, and its providers
//     and security pages drifted with the Crew ones (W2-DOC-8): keys "not in a
//     plaintext file" where a keyless machine writes one, a Launch button on
//     every provider card, "Commercial" where the tab says Public, mode names
//     the app does not use, and no SageMaker default. The getting-started
//     guides and the secret-storage page are held to the same code.
//   * `design`: the Crew design documents (the UI spec, the broker protocol,
//     the naming design and the CLI guide) are marked Current and cited from
//     the code, and wave 2 changed rows each of them pinned (W2-STR-1). The UI
//     spec's SSH failure table has a row for every connect failure the desktop
//     words, the protocol names every capability, snapshot and hello field and
//     storage or delivery code the broker sends, the UI spec quotes the wave-2
//     strings it pins, the naming design may not say nothing in it is built,
//     and the CLI guide names the daemon's answers for a lost request.
//
// A third live check of 2026-09-28 (T3-DOC-*) found quotes of the desktop
// app's and the command line's own words that no rule read:
//
//   * `app-sentences`: a quote of a `members add` summary, a share note about
//     a privacy change, or the Identity file note is that string as the code
//     shows it, and the pages a reader is sent to quote it (T3-DOC-1). The
//     joining page quotes the damaged-invitation note and each join conflict
//     to the end of a sentence, since each conflict has its own way out
//     (T3-DOC-4). The command-line page quotes the command line's own
//     sentences for a missing --connection, an over-long message, a join
//     conflict, a disconnected connection, an ended grant and a connection
//     saved under another institution than its workspace's (T3-DOC-3). The
//     troubleshooting and files pages quote what a server that stopped saving,
//     a restarted background service, a name with hidden characters and a
//     download path through a link show (T3-DOC-6). The agents page quotes the Chat access pane's work folder
//     lines and the work folder switch's help (T3-DOC-5).
//   * `pause-reasons` also reads a reason kept as its own constant (T3-DOC-6).
//   * `reconnect-timing`: the pages say how soon Crew tries again, in the
//     keepalive's own figures, and quote each form of the reconnecting
//     sentence and the member's stopped-server sentence (T3-DOC-6).
//   * `source-line`: the agents page quotes the daemon's last line for a post
//     that read a work-folder file, as its render cases write it (T3-DOC-5).
//   * `dashes`: no em or en dash anywhere in docs/crew, design documents
//     included.
//   * `allow-button`: the Chat access pane's Allow button is named as the
//     pane labels it, never with the chat's title it used to carry.
//   * `refusal-codes` also holds every code the command line gives its own
//     errors, and `ssh-codes` the words a login on this machine gets in place
//     of the advice to ask IT (T3-DOC-3).
//   * `data-paths`: "Where Crew keeps its data" has a row for every folder the
//     code writes on a member computer (T3-DOC-2).
//   * `work-folder`: while the server's sandbox gives a work-folder command no
//     network and no other processes, the agents and administration pages say
//     so, and that cluster tools such as sbatch do not run (T3-DOC-5).
//   * `drop-refusals`: the files page says what a file dropped or pasted in
//     shows for a name with hidden characters as the drop flow words it: the
//     daemon's sentence once `crewFileRefusal` words the code, the general
//     note until then, and never the other one (T3-DOC-6, round 2).
//
// Every rule reads the code it depends on, and a rule whose anchor in the code
// is gone FAILS rather than passing vacuously: the fix is then to re-read the
// code and update the rule, never to delete the anchor check.
//
// Usage: node scripts/check-crew-manual.mjs   (exit 1 and one line per failure)
// Its mutant suite, which proves each rule can fail, is check-crew-manual.test.mjs.
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** A read-only view of the repository: `read` answers null for a missing file. */
export function repoTree(root = REPO) {
  return {
    read(path) {
      const full = join(root, path);
      return existsSync(full) ? readFileSync(full, 'utf8') : null;
    },
    list(dir) {
      const full = join(root, dir);
      return existsSync(full) ? readdirSync(full).sort() : [];
    },
  };
}

const MANUAL_DIR = 'docs/crew';
const LANDING = 'landing/docs.html';
const MAIN_TS = 'ui/desktop/src/main.ts';
const CREW_ROUTES = 'crates/biorouter-server/src/routes/crew.rs';
const SIDEBAR = 'ui/desktop/src/components/BioRouterSidebar/AppSidebar.tsx';
const BROWSER_ACCESS = 'docs/deployment/browser-access.md';
const SPEC = 'docs/crew/design/ui-redesign-spec.md';
const NAMING_DESIGN = 'docs/crew/design/naming-design.md';
const PROTOCOL = 'docs/crew/design/protocol-contract.md';
const CLI_GUIDE = 'docs/crew/design/cli-guide.md';
const CONNECT_FAILURE = 'ui/desktop/src/components/crew/state/connectFailure.ts';
const TIMELINE_COPY = 'ui/desktop/src/components/crew/timeline/copy.ts';
const ACCESS_COPY = 'ui/desktop/src/components/crew/access/copy.ts';
const NAMES_RS = 'crates/biorouter-crew/src/names.rs';
const DROP_ZONE = 'ui/desktop/src/components/crew/files/FileDropZone.tsx';
const MESSAGE_BODY = 'ui/desktop/src/components/crew/timeline/MessageBody.tsx';
const FILES_COPY = 'ui/desktop/src/components/crew/files/copy.ts';
const DIALOGS_COPY = 'ui/desktop/src/components/crew/dialogs/copy.ts';
const CREW_APP = 'ui/desktop/src/components/crew/CrewApp.tsx';
const NEEDS_DESKTOP = 'ui/desktop/src/components/crew/CrewNeedsDesktop.tsx';
const CREW_AUTHENTICATION = 'crates/biorouter-server/src/routes/crew_authentication.rs';
const TROUBLESHOOTING = 'docs/crew/connections-and-troubleshooting.md';
const COMMAND_LINE = 'docs/crew/command-line.md';
const AGENTS_PAGE = 'docs/crew/agents-and-chat-access.md';
const MESSAGES_PAGE = 'docs/crew/messages-and-files.md';
const REFUSAL_RS = 'crates/biorouter/src/crew/refusal.rs';
const CREW_CORE = 'crates/biorouter/src/crew/mod.rs';
const BROKER = 'crates/biorouter-crew/src/broker.rs';
const CLI_OUTPUT = 'crates/biorouter-cli/src/commands/crew/output.rs';
const CLI_ARGS = 'crates/biorouter-cli/src/commands/crew/args.rs';
const ATTENTION = 'ui/desktop/src/components/crew/attention/crewAttention.ts';
const CREW_STATUS = 'ui/desktop/src/components/crew/state/crewStatus.ts';
const SHARE_PATH = 'ui/desktop/src/utils/crewSharePath.ts';
const HOSTING_PAGE = 'docs/crew/hosting-a-workspace.md';
const CLI_CREW = 'crates/biorouter-cli/src/commands/crew/mod.rs';
/** The command line's own words and codes. */
const CLI_SOURCES = [
  CLI_CREW,
  'crates/biorouter-cli/src/commands/crew/output.rs',
  'crates/biorouter-cli/src/daemon_client.rs',
];
const INVITATION_RS = 'crates/biorouter/src/crew/authentication.rs';
const ADMINISTRATION = 'docs/crew/administration.md';
const JOINING_PAGE = 'docs/crew/joining-a-workspace.md';
const ONBOARDING_COPY = 'ui/desktop/src/components/crew/onboarding/copy.ts';
const BAR_COPY = 'ui/desktop/src/components/crew/channel/copy.ts';
const INSTITUTION_RS = 'crates/biorouter/src/crew/institution.rs';
const DECLASSIFY_RS = 'crates/biorouter/src/privacy/declassify.rs';
const KEEPALIVE_RS = 'crates/biorouter/src/crew/keepalive.rs';
const SOURCE_LINE_CASES = 'ui/desktop/src/components/crew/daemonSourceLine.cases.json';
const LOCAL_FILES_RS = 'crates/biorouter-server/src/crew/local_files.rs';
const REMOTE_RS = 'crates/biorouter-crew/src/remote.rs';
/** Where the code that writes Crew's files on a member computer lives. */
const CREW_SOURCE_DIRS = [
  'crates/biorouter/src/crew',
  'crates/biorouter-server/src/crew',
  'crates/biorouter-server/src/routes',
];
const KEY_NOTICE =
  'ui/desktop/src/components/settings/providers/modal/subcomponents/SecureStorageNotice.tsx';
const CARD_BUTTONS =
  'ui/desktop/src/components/settings/providers/subcomponents/buttons/DefaultCardButtons.tsx';
const PROVIDER_ORDERING = 'ui/desktop/src/components/settings/providers/providerOrdering.ts';
const MODE_ITEM = 'ui/desktop/src/components/settings/mode/ModeSelectionItem.tsx';
const SAGEMAKER = 'crates/biorouter/src/providers/sagemaker_tgi.rs';
/** The product pages held to the provider and key storage code (W2-DOC-8). */
const PRODUCT_DOCS = [
  'docs/getting-started/choosing-a-model-provider.md',
  'docs/getting-started/installation.md',
  'docs/security/secret-storage.md',
  'docs/providers/xiaomi-mimo.md',
  'docs/providers/zai-glm.md',
];

/**
 * The value of `pub const <name>: &str = "…";` in Rust source, with the string's
 * line continuations (a backslash, the newline and the next line's leading
 * whitespace) removed as rustc removes them. Null when the constant is absent.
 */
export function rustStrConst(source, name) {
  const match = new RegExp(
    `\\bconst ${name}:\\s*&(?:'static\\s+)?str\\s*=\\s*"((?:[^"\\\\]|\\\\[\\s\\S])*)"`
  ).exec(source);
  return match ? match[1].replace(/\\\n\s*/g, '').replace(/\\(["'\\])/g, '$1') : null;
}

/** Straight and curly apostrophes read alike: the claim is the words, not the glyph. */
const sameApostrophes = (text) => text.replace(/[’‘]/g, "'");

/**
 * The single-quoted string a TypeScript copy object gives `path`, such as `newer` or
 * `removeChannelMember.description`: each segment but the last names a nested object, and the
 * last a property whose value is a plain string literal. Null when any segment is absent, so a
 * rule reading it fails instead of passing on a copy object that changed shape.
 */
export function tsCopyString(source, path) {
  let scope = source;
  const keys = path.split('.');
  for (const key of keys.slice(0, -1)) {
    const open = new RegExp(`\\b${key}:\\s*\\{`).exec(scope);
    if (!open) return null;
    let depth = 0;
    let end = -1;
    for (let i = open.index + open[0].length - 1; i < scope.length; i += 1) {
      if (scope[i] === '{') depth += 1;
      else if (scope[i] === '}' && --depth === 0) {
        end = i;
        break;
      }
    }
    if (end < 0) return null;
    scope = scope.slice(open.index, end + 1);
  }
  const last = keys[keys.length - 1];
  const match = new RegExp(`\\b${last}:\\s*'((?:[^'\\\\\\n]|\\\\.)*)'`).exec(scope);
  return match ? match[1].replace(/\\(['\\])/g, '$1') : null;
}

/** Source without its whole-line comments, whose quotes are not strings the code shows. */
const withoutLineComments = (source) =>
  source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

/**
 * Every string literal in Rust source outside its test modules, as rustc reads it: line
 * continuations removed and escapes undone. A format string keeps its `{…}` holes, which is how a
 * manual quote of it is matched.
 */
export function rustLiterals(source) {
  const shipped = withoutLineComments(source.split(/\n#\[cfg\(test\)\]\nmod \w+ \{/)[0]);
  return [...shipped.matchAll(/(?<![\w#'])"((?:[^"\\]|\\[\s\S])*)"/g)].map((m) =>
    m[1]
      .replace(/\\\n\s*/g, '')
      .replace(/\\u\{([0-9a-fA-F]{1,6})\}/g, (_, hex) => String.fromCodePoint(parseInt(hex, 16)))
      .replace(/\\n/g, '\n')
      .replace(/\\(["'\\])/g, '$1')
  );
}

/**
 * Every string literal in TypeScript source, comments left out: quoted strings as they read, and
 * template literals with each `${…}` hole written `{…}`, as a manual quote of it is matched.
 */
export function tsLiterals(source) {
  return [
    ...withoutLineComments(source).matchAll(
      /'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)"|`((?:[^`\\]|\\[\s\S])*)`/g
    ),
  ].map(([, single, double, template]) =>
    template !== undefined
      ? template.replace(/\$\{/g, '{').replace(/\\([`\\$])/g, '$1')
      : (single ?? double).replace(/\\(['"\\])/g, '$1')
  );
}

const decode = (html) =>
  html
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<[^>]+>/g, '')
    .replace(/&ldquo;|&rdquo;/g, '"')
    .replace(/&lsquo;|&rsquo;/g, "'")
    .replace(/&hellip;/g, '…')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&');

/**
 * Markdown as the reader meets it, in blocks: a paragraph, a table row or a
 * list item with its wrapped lines. Fenced code is dropped, because a command
 * example is not a claim.
 */
export function markdownBlocks(text) {
  const blocks = [];
  let paragraph = [];
  let fence = null;
  const flush = () => {
    if (paragraph.length) blocks.push(paragraph.join(' '));
    paragraph = [];
  };
  for (const line of text.split('\n')) {
    const opener = /^\s*(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (opener && opener[1][0] === fence[0] && opener[1].length >= fence.length) fence = null;
      continue;
    }
    if (opener) {
      flush();
      fence = opener[1];
      continue;
    }
    if (!line.trim()) flush();
    else if (/^\s*(\||>)/.test(line)) {
      flush();
      blocks.push(line.trim());
    } else if (/^\s*([-*+] |\d+\. )/.test(line)) {
      // A list item goes on over its wrapped lines, so a quote that wraps stays whole.
      flush();
      paragraph.push(line.trim());
    } else paragraph.push(line.trim());
  }
  flush();
  return blocks;
}

/** The landing site's Crew page, from its own `doc-page` div to the next one. */
export function landingCrewPage(html) {
  const start = html.indexOf('<div class="doc-page" id="doc-crew">');
  if (start === -1) return null;
  const end = html.indexOf('<div class="doc-page"', start + 1);
  return html.slice(start, end === -1 ? undefined : end);
}

/** HTML in the same blocks as `markdownBlocks`: a paragraph, a list item or a table row. */
export function htmlBlocks(html) {
  return html
    .replace(/<pre[\s\S]*?<\/pre>/gi, '')
    .split(/<\/(?:p|li|tr|h[1-6]|figcaption)>/i)
    .map((block) => decode(block).replace(/\s+/g, ' ').trim())
    .filter(Boolean);
}

/**
 * Every double-quoted phrase in a block of prose, in straight or curly quotes. A straight-quoted
 * phrase may hold curly quotes, as a quoted message that names “chen-lab” does.
 */
const quotedPhrases = (text) =>
  [...text.matchAll(/"([^"\n]+)"|“([^”\n]+)”/g)].map((m) => m[1] ?? m[2]);

/**
 * Every inline code span in a Markdown block, its content trimmed as Markdown trims it. A span
 * opened by two backticks may hold single ones, as a quoted sentence naming a command does.
 */
export const codeSpans = (text) =>
  [...text.matchAll(/(`+)(?!`)([\s\S]+?)(?<!`)\1(?!`)/g)].map((m) => m[2].trim());

/** Quotes and apostrophes read alike, straight or curly: the claim is the words. */
const sameQuotes = (text) => sameApostrophes(text).replace(/[“”]/g, '"');

const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** A template's `{name}` holes, which the code fills with a name. */
const TEMPLATE_HOLE = /\{[^{}]*\}/g;

/** `template` as a pattern: its words as they are, each hole matching any text within a sentence. */
const templatePattern = (template) =>
  sameQuotes(template).split(TEMPLATE_HOLE).map(escapeRegExp).join('(?:(?![.!?] )[^\\n])+?');

/**
 * Whether `phrase` quotes `template`, a sentence from the code whose `{…}` holes a name fills
 * (the manual's own `{workspace}` included). A phrase ending in a full stop, "!" or "?" is all
 * of it, or its first sentences, ending at one of its own full stops. A phrase that ends in
 * "…", or without such a mark, is its opening words.
 */
export function saysTemplate(phrase, template) {
  const said = sameQuotes(phrase).trim();
  const whole = sameQuotes(template).trim();
  if (!said || !whole) return false;
  const full = (source) => new RegExp(`^${source}$`, 's').test(said);
  if (full(templatePattern(whole))) return true;
  if (/[.!?]$/.test(said)) {
    // Its first sentences: every full stop followed by a space ends one.
    for (const boundary of whole.matchAll(/[.!?](?= )/g)) {
      if (full(templatePattern(whole.slice(0, boundary.index + 1)))) return true;
    }
    return false;
  }
  const stem = said.replace(/\s*(…|\.\.\.)$/, '').trimEnd();
  const opening = (end) => new RegExp(`^${templatePattern(whole.slice(0, end))}$`, 's').test(stem);
  for (let end = 1; end <= whole.length; end += 1) {
    // Never cut a hole in half, and never take a hole alone for the opening: a template that
    // starts with a name would then open every phrase.
    const before = whole.slice(0, end);
    if (before.lastIndexOf('{') > before.lastIndexOf('}')) continue;
    if (before.replace(TEMPLATE_HOLE, '').replace(/[^\p{L}\p{N}]/gu, '').length < 4) continue;
    if (opening(end)) return true;
  }
  return false;
}

export function checkCrewManual(tree = repoTree()) {
  const failures = [];
  const fail = (rule, message) => failures.push(`${rule}: ${message}`);
  const need = (path, rule) => {
    const text = tree.read(path);
    if (text === null) fail(rule, `${path} is missing, so this rule cannot read what it checks`);
    return text;
  };

  const manualFiles = tree.list(MANUAL_DIR).filter((name) => name.endsWith('.md'));
  if (manualFiles.length < 5) {
    fail('manual', `found ${manualFiles.length} pages under ${MANUAL_DIR}; the reader is broken`);
  }
  const manual = manualFiles.map((name) => ({
    path: `${MANUAL_DIR}/${name}`,
    blocks: markdownBlocks(tree.read(`${MANUAL_DIR}/${name}`) || ''),
  }));
  const landingHtml = need(LANDING, 'manual') || '';
  const crewPageHtml = landingCrewPage(landingHtml);
  if (crewPageHtml === null)
    fail('manual', `${LANDING} has no <div class="doc-page" id="doc-crew">`);
  const surfaces = [...manual, { path: `${LANDING}#crew`, blocks: htmlBlocks(crewPageHtml || '') }];

  // ── brand ────────────────────────────────────────────────────────────────
  // The brand a person reads is "Biorouter" (see check-brand-consistency.sh).
  // Inline code is exempt: it may name an identifier, which is not the brand.
  for (const { path, blocks } of surfaces) {
    for (const block of blocks) {
      const prose = block.replace(/`[^`]*`/g, '');
      if (/BioRouter/.test(prose)) {
        fail('brand', `${path} spells the brand "BioRouter" in: ${block.slice(0, 140)}`);
      }
    }
  }

  // ── dialog-titles ────────────────────────────────────────────────────────
  // The manual tells a person to trust the native secret windows by name, so
  // it must name them exactly as main.ts titles them. A title is a string
  // literal of three or more words in promptNativeSecret's first argument
  // (the vault prompt picks its title with a ternary). The three daemon
  // approval-secret windows are the ones "The approval secret" in
  // getting-started.md walks a person through, so that page quotes each one.
  const mainTs = need(MAIN_TS, 'dialog-titles');
  if (mainTs !== null) {
    const titles = [
      ...new Set(
        [...mainTs.matchAll(/promptNativeSecret\(\s*([^,]*?),/g)].flatMap((call) =>
          [...call[1].matchAll(/(['"])((?:(?!\1)[^\\\n])+)\1/g)]
            .map((literal) => literal[2])
            .filter((literal) => literal.trim().split(/\s+/).length >= 3)
        )
      ),
    ];
    const daemonTitles = titles.filter((title) => /\bdaemon\b/.test(title));
    if (daemonTitles.length < 3) {
      fail(
        'dialog-titles',
        `found ${daemonTitles.length} daemon approval-secret titles in ${MAIN_TS} (expected set, confirm and connect); ` +
          'if promptNativeSecret changed shape, update this reader'
      );
    }
    const gettingStarted = tree.read(`${MANUAL_DIR}/getting-started.md`) || '';
    for (const title of daemonTitles) {
      if (!gettingStarted.includes(`"${title}"`)) {
        fail(
          'dialog-titles',
          `${MANUAL_DIR}/getting-started.md does not quote the dialog title "${title}" exactly as ${MAIN_TS} shows it`
        );
      }
    }
    for (const title of titles) {
      for (const { path, blocks } of surfaces) {
        for (const phrase of blocks.flatMap(quotedPhrases)) {
          if (phrase !== title && phrase.toLowerCase() === title.toLowerCase()) {
            fail(
              'dialog-titles',
              `${path} quotes "${phrase}", but ${MAIN_TS} titles it "${title}"`
            );
          }
        }
      }
    }
  }

  // ── serve ────────────────────────────────────────────────────────────────
  // `biorouter serve` spawns its daemon with no user-action key (SD-1), and
  // every Crew route asks `require_person` (or its sibling in another
  // routes/crew_*.rs file), which answers a keyless daemon with
  // crew_human_authority_unavailable. The anchor is that mapping in
  // routes/crew.rs, spelled as the literal or as a HUMAN_AUTHORITY constant.
  const routes = need(CREW_ROUTES, 'serve');
  const refusesKeyless =
    routes !== null &&
    /UserActionProof::NoKeyInstalled\s*=>[\s\S]{0,600}?("crew_human_authority_unavailable"|HUMAN_AUTHORITY_UNAVAILABLE)/.test(
      routes
    );
  if (routes !== null && !refusesKeyless) {
    fail(
      'serve',
      `${CREW_ROUTES} no longer maps NoKeyInstalled to crew_human_authority_unavailable; ` +
        're-read it, and if Crew now works on a keyless daemon, rewrite what the manual says about biorouter serve'
    );
  }
  if (refusesKeyless) {
    const mentionsServe = (block) => /biorouter serve/.test(block);
    const saysNo = (block) =>
      /\b(does not work|doesn['’]t work|never works|not available|cannot use|can['’]t use|not in a)\b/i.test(
        block
      ) || /^\|?\s*Crew\s*\|\s*\*\*Not available/.test(block);
    for (const { path, blocks } of surfaces) {
      for (const block of blocks.filter(mentionsServe)) {
        if (!saysNo(block)) {
          fail(
            'serve',
            `${path} names biorouter serve without saying Crew does not work there: ${block.slice(0, 160)}`
          );
        }
      }
    }
    const gettingStarted = markdownBlocks(tree.read(`${MANUAL_DIR}/getting-started.md`) || '');
    if (!gettingStarted.some((block) => mentionsServe(block) && saysNo(block))) {
      fail(
        'serve',
        `${MANUAL_DIR}/getting-started.md must say that Crew does not work in a biorouter serve browser`
      );
    }
    const readme = need('README.md', 'serve');
    for (const block of markdownBlocks(readme || '').filter(
      (b) => mentionsServe(b) && /\bCrew\b/.test(b)
    )) {
      if (!saysNo(block))
        fail(
          'serve',
          `README.md names Crew and biorouter serve without saying Crew does not work there`
        );
    }
    const browserAccess = markdownBlocks(need(BROWSER_ACCESS, 'serve') || '');
    if (!browserAccess.some((block) => /^\|\s*Crew\s*\|\s*\*\*Not available/.test(block))) {
      fail(
        'serve',
        `${BROWSER_ACCESS} needs a "| Crew | **Not available.** …" row in its browser capability table`
      );
    }
    if (!/<tr><td>Crew<\/td><td>No\b/.test(landingHtml)) {
      fail('serve', `${LANDING}'s "What works in a browser" table needs a Crew row that says No`);
    }
  }

  // ── needs-desktop ────────────────────────────────────────────────────────
  // In a `biorouter serve` browser CrewApp shows CrewNeedsDesktop and mounts
  // nothing that asks the daemon (CROSSCUT-5), so that page's title is what a
  // browser reader meets. Every place the manual sends such a reader quotes it
  // exactly: getting-started, the manual's index, a troubleshooting row, and
  // both browser capability tables, whose Crew rows open as SD-8's rows do,
  // since Crew now says so before anyone tries. A daemon that serves the
  // browser refuses a Crew request with CREW_NEEDS_THE_DESKTOP, and the
  // troubleshooting page quotes its opening words: every quote of it must be
  // the start of that sentence, and no quote of it may outlive the constant.
  const crewApp = need(CREW_APP, 'needs-desktop');
  const showsNeedsDesktop =
    crewApp !== null &&
    /if\s*\(\s*isBrowserSurface\(\)\s*\)\s*return\s*<CrewNeedsDesktop\s*\/>/.test(crewApp);
  if (crewApp !== null && !showsNeedsDesktop) {
    fail(
      'needs-desktop',
      `${CREW_APP} no longer returns <CrewNeedsDesktop /> on the browser surface; ` +
        're-read it, and rewrite what the manual says a biorouter serve browser shows'
    );
  }
  const needsDesktopSource = need(NEEDS_DESKTOP, 'needs-desktop');
  const pageTitle =
    needsDesktopSource === null
      ? null
      : (/crewNeedsDesktopCopy\s*=\s*\{[\s\S]*?\btitle:\s*(['"])((?:(?!\1)[^\\\n])+)\1/.exec(
          needsDesktopSource
        )?.[2] ?? null);
  if (needsDesktopSource !== null && pageTitle === null) {
    fail(
      'needs-desktop',
      `found no crewNeedsDesktopCopy.title in ${NEEDS_DESKTOP}; update this reader`
    );
  }
  const browserAccessCrewRow =
    markdownBlocks(tree.read(BROWSER_ACCESS) || '').find((block) =>
      /^\|\s*Crew\s*\|/.test(block)
    ) || '';
  const landingServeCrewRow = decode(
    /<tr><td>Crew<\/td><td>(No\b[\s\S]*?)<\/td><\/tr>/.exec(landingHtml)?.[1] || ''
  );
  const messageRows = markdownBlocks(tree.read(TROUBLESHOOTING) || '').filter((block) =>
    block.startsWith('|')
  );
  const everyQuote = [
    ...surfaces.flatMap(({ path, blocks }) =>
      blocks.flatMap(quotedPhrases).map((phrase) => ({ path, phrase }))
    ),
    ...quotedPhrases(browserAccessCrewRow).map((phrase) => ({ path: BROWSER_ACCESS, phrase })),
    ...quotedPhrases(landingServeCrewRow).map((phrase) => ({ path: LANDING, phrase })),
  ];
  const authSource = need(CREW_AUTHENTICATION, 'needs-desktop');
  const serveRefusal =
    authSource === null ? null : rustStrConst(authSource, 'CREW_NEEDS_THE_DESKTOP');
  // A quote of the refusal is its opening words ending in "…", or all of it.
  const quotesRefusal = (phrase) => {
    if (serveRefusal === null) return false;
    const refusal = sameApostrophes(serveRefusal);
    const said = sameApostrophes(phrase);
    if (!said.endsWith('…')) return said === refusal;
    const stem = said.slice(0, -1).trimEnd();
    return stem.length >= 24 && refusal.startsWith(stem);
  };
  const looksLikeRefusal = (phrase) =>
    /^Crew isn['’]t available\b/i.test(phrase) ||
    (serveRefusal !== null &&
      sameApostrophes(phrase).startsWith(
        sameApostrophes(serveRefusal).split(/\s+/).slice(0, 4).join(' ')
      ));
  if (showsNeedsDesktop && pageTitle !== null) {
    const quote = `"${pageTitle}"`;
    for (const path of [`${MANUAL_DIR}/getting-started.md`, `${MANUAL_DIR}/README.md`]) {
      if (!(tree.read(path) || '').includes(quote)) {
        fail(
          'needs-desktop',
          `${path} does not quote the page a serve browser shows, ${quote}, exactly as ${NEEDS_DESKTOP} titles it`
        );
      }
    }
    if (!messageRows.some((row) => (row.split('|')[1] || '').includes(quote))) {
      fail('needs-desktop', `${TROUBLESHOOTING} has no message row for ${quote}`);
    }
    if (
      !/^\|\s*Crew\s*\|\s*\*\*Not available, and it says so before you try\.\*\*/.test(
        browserAccessCrewRow
      ) ||
      !browserAccessCrewRow.includes(quote)
    ) {
      fail(
        'needs-desktop',
        `${BROWSER_ACCESS}'s Crew row must open "**Not available, and it says so before you try.**", ` +
          `as the other SD-8 rows do, and quote ${quote}`
      );
    }
    if (!landingServeCrewRow.includes(quote)) {
      fail(
        'needs-desktop',
        `${LANDING}'s "What works in a browser" Crew row does not quote ${quote}`
      );
    }
    // A quote that is almost the title is an old title left behind.
    for (const { path, phrase } of everyQuote) {
      if (phrase !== pageTitle && /^Crew needs the\b/i.test(phrase)) {
        fail(
          'needs-desktop',
          `${path} quotes "${phrase}", but ${NEEDS_DESKTOP} titles the page ${quote}`
        );
      }
    }
    // A serve browser shows that page, and a refusal only in the daemon's own
    // sentence. A passage about the browser that quotes any other message sends
    // its reader to one the browser never shows, as the audited sign-in row and
    // the "This daemon cannot verify human Crew actions…" row did.
    const aboutTheBrowser = (block) => /biorouter serve|web browser/i.test(block);
    for (const { path, blocks } of surfaces) {
      for (const block of blocks.filter(aboutTheBrowser)) {
        for (const phrase of quotedPhrases(block)) {
          if (phrase !== pageTitle && !quotesRefusal(phrase)) {
            fail(
              'needs-desktop',
              `${path} tells a serve browser reader about "${phrase}", which that browser never shows: ${block.slice(0, 140)}`
            );
          }
        }
      }
    }
    // Restarting the background service cannot help a browser, so the section
    // that says how does not send a browser reader there.
    const replaceSection =
      /\n### Replace an old background service\n([\s\S]*?)(?=\n#{2,3} |$)/.exec(
        tree.read(TROUBLESHOOTING) || ''
      );
    if (!replaceSection) {
      fail(
        'needs-desktop',
        `${TROUBLESHOOTING} has no "### Replace an old background service" section; update this reader`
      );
    } else {
      for (const block of markdownBlocks(replaceSection[1]).filter(aboutTheBrowser)) {
        fail(
          'needs-desktop',
          `${TROUBLESHOOTING}'s "Replace an old background service" sends a browser reader to a restart that cannot help: ${block.slice(0, 140)}`
        );
      }
    }
  }
  for (const { path, phrase } of everyQuote.filter(({ phrase }) => looksLikeRefusal(phrase))) {
    if (serveRefusal === null) {
      fail(
        'needs-desktop',
        `${path} quotes "${phrase}", but ${CREW_AUTHENTICATION} defines no CREW_NEEDS_THE_DESKTOP`
      );
    } else if (!quotesRefusal(phrase)) {
      fail(
        'needs-desktop',
        `${path} quotes "${phrase}", which is not how ${CREW_AUTHENTICATION}'s CREW_NEEDS_THE_DESKTOP begins`
      );
    }
  }
  if (serveRefusal !== null) {
    if (!messageRows.some((row) => quotedPhrases(row.split('|')[1] || '').some(quotesRefusal))) {
      fail(
        'needs-desktop',
        `${TROUBLESHOOTING} has no message row quoting the refusal a serve daemon gives: "${serveRefusal.slice(0, 60)}…"`
      );
    }
  }

  // ── host-reach ───────────────────────────────────────────────────────────
  // A who-can-add cell that names the host must say the host has to be in the
  // team or channel: the host sees only those, so it can add people only there.
  const namesHostWithoutReach = (cell) => /\bhost\b/i.test(cell) && !/\bhost is in\b/i.test(cell);
  const teams = tree.read(`${MANUAL_DIR}/teams-channels-and-people.md`);
  if (teams === null) fail('host-reach', `${MANUAL_DIR}/teams-channels-and-people.md is missing`);
  const addRows = markdownBlocks(teams || '').filter((block) =>
    /^\|\s*Add people to a (team|channel)\s*\|/.test(block)
  );
  if (teams !== null && addRows.length !== 2) {
    fail(
      'host-reach',
      `expected the two "Add people to a team/channel" rows in the roles table, found ${addRows.length}`
    );
  }
  for (const row of addRows) {
    const who = row.split('|')[2] || '';
    if (namesHostWithoutReach(who)) {
      fail(
        'host-reach',
        `${MANUAL_DIR}/teams-channels-and-people.md lets the host add people without "the host is in": ${row}`
      );
    }
  }
  const landingAdd = /<tr><td>Add people<\/td><td>([\s\S]*?)<\/td>/.exec(crewPageHtml || '');
  if (!landingAdd) fail('host-reach', `${LANDING}#crew has no "Add people" row in its teams table`);
  else if (namesHostWithoutReach(decode(landingAdd[1]))) {
    fail(
      'host-reach',
      `${LANDING}#crew lets the host add people without "the host is in": ${decode(landingAdd[1])}`
    );
  }

  // ── readme ───────────────────────────────────────────────────────────────
  // Crew is a sidebar destination, so the repository's front page names it.
  const sidebar = need(SIDEBAR, 'readme');
  if (sidebar !== null) {
    const labels = [...sidebar.matchAll(/label: '([^']+)'/g)].map((m) => m[1]);
    if (labels.length === 0)
      fail('readme', `found no "label: '…'" rows in ${SIDEBAR}; update this reader`);
    if (labels.includes('Crew')) {
      const readme = tree.read('README.md') || '';
      if (!readme.includes('](docs/crew/README.md)'))
        fail('readme', 'README.md does not link the Crew user manual');
      const whatYouCanDo = /## What you can do\n([\s\S]*?)\n## /.exec(readme);
      if (!whatYouCanDo) fail('readme', 'README.md has no "## What you can do" section');
      else if (
        !markdownBlocks(whatYouCanDo[1]).some((b) => b.startsWith('|') && /\bCrew\b/.test(b))
      ) {
        fail('readme', `README.md's "What you can do" table has no row for Crew`);
      }
    }
  }

  // ── spec ─────────────────────────────────────────────────────────────────
  // The redesign spec is marked Current, so it may not deny what shipped.
  const spec = need(SPEC, 'spec');
  if (spec !== null) {
    if (
      /Nothing in it is built yet/.test(spec) &&
      tree.read('ui/desktop/src/components/crew/CrewApp.tsx') !== null
    ) {
      fail('spec', `${SPEC} says nothing in it is built, but crew/CrewApp.tsx ships it`);
    }
    if (tree.read(DROP_ZONE) !== null && /No drag-and-drop/.test(spec)) {
      fail('spec', `${SPEC} says Crew has no drag and drop, but ${DROP_ZONE} ships it (D-DROP)`);
    }
    const body = tree.read(MESSAGE_BODY);
    if (
      body !== null &&
      /from 'react-markdown'/.test(body) &&
      /\| \*\*Body\*\* \|[^\n]*plain text as today/.test(spec)
    ) {
      fail(
        'spec',
        `${SPEC} says message bodies are plain text, but ${MESSAGE_BODY} renders Markdown`
      );
    }
    const copy = tree.read(FILES_COPY);
    const fileRow = spec.split('\n').find((line) => line.startsWith('| `file.*` |')) || '';
    if (
      copy !== null &&
      !copy.includes("'Save attachment'") &&
      fileRow.includes('**Save attachment**')
    ) {
      fail(
        'spec',
        `${SPEC}'s file.* copy row names "Save attachment", which ${FILES_COPY} replaced (Q3-13)`
      );
    }
  }

  // What each surface quotes: double-quoted phrases, and in the manual its code spans too, since
  // the command-line page quotes terminal output that way.
  const quotesOf = ({ path, blocks }) =>
    blocks.flatMap((block) =>
      path.endsWith('.md') ? [...quotedPhrases(block), ...codeSpans(block)] : quotedPhrases(block)
    );
  const pageQuotes = (page) =>
    quotesOf(surfaces.find(({ path }) => path === page) || { path: page, blocks: [] });

  // ── daemon-sentences ─────────────────────────────────────────────────────
  // A sentence the daemon or the workspace server writes for a person, quoted
  // by the manual, is that sentence (W2-DOC-7). Each family is found by its
  // opening words, and some pages must quote it: the one place a reader who
  // meets it is sent.
  const refusalSource = need(REFUSAL_RS, 'daemon-sentences');
  const coreSource = need(CREW_CORE, 'daemon-sentences');
  const brokerSource = need(BROKER, 'daemon-sentences');
  const formatSentence = (source, opening) =>
    source === null
      ? null
      : (new RegExp(`"(${escapeRegExp(opening)}[^"\\n]*)"`).exec(source)?.[1] ?? null);
  const families = [
    {
      name: `MODEL_FIXED_TEXT in ${REFUSAL_RS}`,
      opens: /^This chat's model is fixed\b/,
      templates: [refusalSource === null ? null : rustStrConst(refusalSource, 'MODEL_FIXED_TEXT')],
      source: refusalSource,
      requiredIn: [AGENTS_PAGE],
    },
    {
      name: `CREDENTIAL_STORE_UNAVAILABLE_TEXT in ${CREW_CORE}`,
      opens: /^This computer has no keyring service\b/,
      templates: [
        coreSource === null ? null : rustStrConst(coreSource, 'CREDENTIAL_STORE_UNAVAILABLE_TEXT'),
      ],
      source: coreSource,
      requiredIn: [COMMAND_LINE],
    },
    {
      name: `KEYRING_NOT_RUNNING_TEXT in ${CREW_CORE}`,
      opens: /^This computer's keyring service isn't answering\b/,
      templates: [
        coreSource === null ? null : rustStrConst(coreSource, 'KEYRING_NOT_RUNNING_TEXT'),
      ],
      source: coreSource,
      requiredIn: [],
    },
    {
      name: `the crew_outcome_unknown sentence in ${CREW_CORE}`,
      opens: /^Crew couldn't confirm whether this reached\b/,
      templates: [formatSentence(coreSource, "Crew couldn't confirm whether this reached {")],
      source: coreSource,
      requiredIn: [COMMAND_LINE],
    },
    {
      name: `the crew_reconnecting sentence in ${CREW_CORE}`,
      opens: /^Reconnecting to\b/,
      // Every form: a wait over ten seconds names how long (T3-BE-16).
      templates: rustLiterals(coreSource || '').filter((text) =>
        text.startsWith('Reconnecting to {')
      ),
      source: coreSource,
      requiredIn: [COMMAND_LINE, TROUBLESHOOTING],
      requiredEach: true,
    },
    {
      name: `the storage_full and storage_failed sentences in ${BROKER}`,
      opens: /^The workspace server (is out of|ran out of|could not)\b/,
      templates:
        brokerSource === null
          ? [null]
          : [...brokerSource.matchAll(/const STORAGE_[A-Z_]+: &str =\s*"((?:[^"\\]|\\.)*)";/g)].map(
              (m) => m[1].replace(/^storage_(?:full|failed): /, '')
            ),
      source: brokerSource,
      requiredIn: [COMMAND_LINE],
    },
  ];
  /**
   * Hold every surface to each family of sentences: a quote that opens as one of the family does
   * says one of its templates, and each page in `requiredIn` quotes one. A family whose templates
   * are gone from the code fails rather than passing vacuously.
   */
  const holdToFamilies = (rule, list) => {
    for (const family of list) {
      if (family.source === null) continue;
      const templates = family.templates.filter((template) => typeof template === 'string');
      if (templates.length === 0) {
        fail(
          rule,
          `found no ${family.name}; re-read the code and update this rule and the manual together`
        );
        continue;
      }
      // `onlyIn` and `notIn` scope a family to the pages that quote that surface's words: the
      // desktop and the command line can say one thing in two ways.
      const inScope = ({ path }) =>
        (!family.onlyIn || family.onlyIn.includes(path)) && !(family.notIn || []).includes(path);
      for (const surface of surfaces.filter(inScope)) {
        for (const phrase of quotesOf(surface)) {
          if (!family.opens.test(sameQuotes(phrase))) continue;
          if (!templates.some((template) => saysTemplate(phrase, template))) {
            fail(
              rule,
              `${surface.path} quotes "${phrase}", which is not how ${family.name} reads: "${templates[0]}"`
            );
          }
        }
      }
      // A family that says `requiredWhole` is quoted at least to the end of a sentence there, so
      // a quote cut short cannot hide which of the family's sentences, old or new, it means. One
      // that says `requiredEach` is quoted so for every sentence in it.
      const whole = (phrase, template) => /[.!?]$/.test(phrase) && saysTemplate(phrase, template);
      for (const page of family.requiredIn) {
        const quoted = pageQuotes(page).filter((phrase) => family.opens.test(sameQuotes(phrase)));
        const missing = family.requiredEach
          ? templates.filter((template) => !quoted.some((phrase) => whole(phrase, template)))
          : family.requiredWhole
            ? quoted.some((phrase) => templates.some((template) => whole(phrase, template)))
              ? []
              : [templates[0]]
            : quoted.length
              ? []
              : [templates[0]];
        const how = family.requiredEach || family.requiredWhole ? ' to the end of a sentence' : '';
        for (const template of missing) {
          fail(rule, `${page} does not quote ${family.name}${how}: "${template}"`);
        }
      }
    }
  };
  holdToFamilies('daemon-sentences', families);

  // ── app-sentences ────────────────────────────────────────────────────────
  // A sentence the desktop app or the command line shows, quoted by the manual, is that sentence
  // (T3-DOC-1). The manual quoted a `members add` summary, a share note and an Identity file note
  // that had each been reworded, and no rule read them. Each family is found by its opening
  // words, the retired ones included, so a quote of the old words is refused while the code no
  // longer says them.
  const cliCrewSource = need(CLI_CREW, 'app-sentences');
  const dialogsSource = need(DIALOGS_COPY, 'app-sentences');
  const shareSource = need(SHARE_PATH, 'app-sentences');
  const coreForApp = need(CREW_CORE, 'app-sentences');
  const invitationSource = need(INVITATION_RS, 'app-sentences');
  const onboardingSource = need(ONBOARDING_COPY, 'app-sentences');
  const barSource = need(BAR_COPY, 'app-sentences');
  const accessSource = need(ACCESS_COPY, 'app-sentences');
  const institutionSource = need(INSTITUTION_RS, 'app-sentences');
  const declassifySource = need(DECLASSIFY_RS, 'app-sentences');
  const localFilesSource = need(LOCAL_FILES_RS, 'app-sentences');
  const opening = (texts, opens) =>
    texts.map((text) => text.trim()).filter((text) => opens.test(sameQuotes(text)));
  const cliLiterals = CLI_SOURCES.flatMap((path) => rustLiterals(tree.read(path) || ''));
  // A person added to a team or channel. A device's "Added September 24, 2026" is another string.
  const addedOpens = /^Added\b(?! [A-Z][a-z]+ \d)/;
  holdToFamilies('app-sentences', [
    {
      name: `the "Added …" summaries in ${CLI_CREW} and ${DIALOGS_COPY}`,
      opens: addedOpens,
      templates: [
        ...opening(rustLiterals(cliCrewSource || ''), addedOpens),
        ...opening(tsLiterals(dialogsSource || ''), /^Added .* to /),
      ],
      source: cliCrewSource === null || dialogsSource === null ? null : cliCrewSource,
      requiredIn: [COMMAND_LINE, HOSTING_PAGE],
    },
    {
      name: `the crew_mode_mismatch share notes in ${SHARE_PATH}`,
      opens:
        /^(?:Your connection is now|Your connection's privacy changed|Connection privacy changed)\b/,
      templates: opening(
        tsLiterals(shareSource || ''),
        /^Your connection(?: is now|'s privacy changed)\b/
      ),
      source: shareSource,
      requiredIn: [MESSAGES_PAGE],
    },
    {
      name: `the Identity file note in ${DIALOGS_COPY}`,
      opens: /^Use the key file\b/,
      templates: opening(tsLiterals(dialogsSource || ''), /^Use the key file\b/),
      source: dialogsSource,
      requiredIn: [TROUBLESHOOTING],
    },
    {
      name: `the command line's join conflicts in ${CLI_CREW}`,
      opens: /^(?:This computer already has|This workspace is already saved as)\b/,
      templates: opening(
        cliLiterals,
        /^(?:This computer already has|This workspace is already saved as)\b/
      ),
      source: cliCrewSource,
      requiredIn: [COMMAND_LINE],
      requiredEach: true,
      onlyIn: [COMMAND_LINE],
    },
    ...[
      ['the login mismatch warning', /^This invitation is for @/],
      ['the refusal of a missing --connection', /^Several Crew connections are saved\b/],
      ['the refusal of a message over 64 KB', /^Messages can be up to\b/],
      ['the ended grant sentences', /^This chat's Crew access ended\b/],
      ['the disconnected sentence', /^\S+ is disconnected\. Run\b/],
      ['host line for a server that stopped saving', /^You host this workspace\. /],
    ].map(([what, opens]) => ({
      name: `the command line's ${what} in ${CLI_SOURCES.join(', ')}`,
      opens,
      templates: opening(cliLiterals, opens),
      source: cliCrewSource,
      requiredIn: [COMMAND_LINE],
      requiredWhole: true,
      onlyIn: [COMMAND_LINE],
    })),
    {
      name: `the stopped-saving sentence in ${BAR_COPY} and ${CLI_OUTPUT}`,
      opens: /^The workspace server has stopped saving\b/,
      templates: opening(
        [...tsLiterals(barSource || ''), ...cliLiterals],
        /^The workspace server has stopped saving\b/
      ),
      source: barSource,
      requiredIn: [TROUBLESHOOTING],
      requiredWhole: true,
    },
    {
      name: `the stopped-saving next steps in ${BAR_COPY}`,
      opens:
        /^(?:Free space on the server|Check the server's storage|Ask .+ to (?:free space|check the server's storage))\b/,
      templates: opening(
        tsLiterals(barSource || ''),
        /^(?:Free space on the server|Check the server's storage|Ask .+ to (?:free space|check the server's storage))\b/
      ),
      source: barSource,
      requiredIn: [TROUBLESHOOTING],
      notIn: [COMMAND_LINE],
    },
    {
      name: `the background-service note in ${BAR_COPY}`,
      opens: /^Biorouter's background service restarted, so\b/,
      templates: opening(
        tsLiterals(barSource || ''),
        /^Biorouter's background service restarted, so\b/
      ),
      source: barSource,
      requiredIn: [TROUBLESHOOTING],
      requiredWhole: true,
    },
    {
      // The daemon's sentence, and the drop flow's copy of it once `crewFileRefusal` words the
      // code: the page quotes each whole, so the two can only differ by the page quoting both.
      name: `the hidden-character refusal in ${LOCAL_FILES_RS} and ${SHARE_PATH}`,
      opens: /^"[^"]*" has an invisible or formatting character\b/,
      templates: opening(
        [...rustLiterals(localFilesSource || ''), ...tsLiterals(shareSource || '')],
        /^"[^"]*" has an invisible or formatting character\b/
      ),
      source: localFilesSource,
      requiredIn: [MESSAGES_PAGE],
      requiredEach: true,
    },
    {
      name: `the Chat access pane's work folder lines in ${ACCESS_COPY}`,
      opens: /^Reads? and writes? files\b/,
      templates: opening(tsLiterals(accessSource || ''), /^Reads? and writes? files\b/),
      source: accessSource,
      requiredIn: [AGENTS_PAGE],
    },
    {
      name: `the work folder switch's help in ${DIALOGS_COPY}`,
      opens: /^Commands run with no network\b/,
      templates: opening(tsLiterals(dialogsSource || ''), /^Commands run with no network\b/),
      source: dialogsSource,
      requiredIn: [AGENTS_PAGE],
      requiredWhole: true,
    },
    {
      // A save under another institution than the workspace's is refused (T3-DOC-3), where the
      // command line used to save it and every task was refused afterwards.
      name: `the institution refusal in ${INSTITUTION_RS} and ${DIALOGS_COPY}`,
      opens: /^This connection is for\b/,
      templates: opening(
        [...rustLiterals(institutionSource || ''), ...tsLiterals(dialogsSource || '')],
        /^This connection is for\b/
      ),
      source: institutionSource,
      requiredIn: [COMMAND_LINE, TROUBLESHOOTING],
      requiredWhole: true,
    },
    {
      name: `the refusal to make a Crew chat public in ${DECLASSIFY_RS}`,
      opens: /^This chat read Crew channels\b/,
      templates: opening(rustLiterals(declassifySource || ''), /^This chat read Crew channels\b/),
      source: declassifySource,
      requiredIn: [AGENTS_PAGE],
      requiredWhole: true,
    },
    {
      name: `the re-dial state in ${ONBOARDING_COPY}`,
      opens: /^The connection dropped, and Crew has been dialling\b/,
      templates: opening(
        tsLiterals(onboardingSource || ''),
        /^The connection dropped, and Crew has been dialling\b/
      ),
      source: onboardingSource,
      requiredIn: [TROUBLESHOOTING],
      requiredWhole: true,
    },
    {
      name: `the stopped-server sentence for a member in ${ONBOARDING_COPY} and ${BAR_COPY}`,
      opens: /^The workspace server isn't running\b/,
      templates: opening(
        [...tsLiterals(onboardingSource || ''), ...tsLiterals(barSource || '')],
        /^The workspace server isn't running\b/
      ),
      source: onboardingSource,
      requiredIn: [TROUBLESHOOTING, JOINING_PAGE],
      requiredWhole: true,
    },
    {
      name: `the refusal of a path through a link in ${LOCAL_FILES_RS}`,
      opens: /^\S+ goes through a link\b/,
      templates: opening(rustLiterals(localFilesSource || ''), /^\S+ goes through a link\b/),
      source: localFilesSource,
      requiredIn: [MESSAGES_PAGE, COMMAND_LINE],
      requiredWhole: true,
    },
    {
      name: `the damaged-invitation note in ${ONBOARDING_COPY}`,
      opens: /^This invitation is incomplete\b/,
      templates: opening(tsLiterals(onboardingSource || ''), /^This invitation is incomplete\b/),
      source: onboardingSource,
      requiredIn: [JOINING_PAGE],
      requiredWhole: true,
    },
    {
      // Each one, since each has its own way out: open the saved connection, or replace it.
      name: `the daemon's join conflicts in ${INVITATION_RS}`,
      opens: /^This computer already has\b/,
      templates: opening(rustLiterals(invitationSource || ''), /^This computer already has\b/),
      source: invitationSource,
      requiredIn: [JOINING_PAGE],
      requiredEach: true,
      notIn: [COMMAND_LINE],
    },
    {
      name: `the daemon's identity file refusals in ${CREW_CORE} and ${INVITATION_RS}`,
      opens: /^(?:Identity file must be|Choose the identity file)\b/,
      templates: opening(
        [...rustLiterals(coreForApp || ''), ...rustLiterals(invitationSource || '')],
        /^(?:Identity file must be|Choose the identity file)\b/
      ),
      source: coreForApp === null || invitationSource === null ? null : coreForApp,
      requiredIn: [],
    },
  ]);

  // ── refusal-codes ────────────────────────────────────────────────────────
  // Every code the daemon refuses a Crew request with is in the command-line
  // page, which is where a script author looks a code up (W2-DOC-6).
  if (refusalSource !== null) {
    const codes = [...refusalSource.matchAll(/pub const [A-Z_]+: &str = "(crew_[a-z_]+)";/g)].map(
      (m) => m[1]
    );
    if (codes.length < 8) {
      fail(
        'refusal-codes',
        `found ${codes.length} refusal codes in ${REFUSAL_RS}; if refusal.rs changed shape, update this reader`
      );
    }
    const commandLine = tree.read(COMMAND_LINE) || '';
    for (const code of codes) {
      if (!commandLine.includes(`\`${code}\``)) {
        fail('refusal-codes', `${COMMAND_LINE} never names the daemon's refusal code \`${code}\``);
      }
    }
    // And every code the command line gives its own errors (T3-DOC-3): a wrong command line,
    // several connections and no --connection, a message too long to send.
    const cliCodes = [
      ...new Set(
        CLI_SOURCES.flatMap((path) => [
          ...(tree.read(path) || '')
            .split(/\n#\[cfg\(test\)\]\nmod \w+ \{/)[0]
            .matchAll(/\bconst [A-Z_]+: &str = "(crew_[a-z_]+)";/g),
        ]).map((m) => m[1])
      ),
    ];
    if (cliCodes.length < 8) {
      fail(
        'refusal-codes',
        `found ${cliCodes.length} codes in ${CLI_SOURCES.join(', ')}; update this reader`
      );
    }
    for (const code of cliCodes.filter((code) => !codes.includes(code))) {
      if (!commandLine.includes(`\`${code}\``)) {
        fail('refusal-codes', `${COMMAND_LINE} never names the command line's code \`${code}\``);
      }
    }
  }

  // ── ssh-codes ────────────────────────────────────────────────────────────
  // Every connect failure the command line explains has a row in the
  // command-line page's SSH failure table (W2-DOC-6).
  const cliOutput = need(CLI_OUTPUT, 'ssh-codes');
  if (cliOutput !== null) {
    const body = /pub fn connect_failure_text\([\s\S]*?\n\}/.exec(cliOutput)?.[0] ?? '';
    const codes = [...body.matchAll(/"(crew_[a-z_]+)" =>/g)].map((m) => m[1]);
    if (codes.length < 5) {
      fail(
        'ssh-codes',
        `found ${codes.length} codes in ${CLI_OUTPUT}'s connect_failure_text; update this reader`
      );
    }
    const rows = markdownBlocks(tree.read(COMMAND_LINE) || '').filter((b) => b.startsWith('|'));
    const rowOf = (code) => rows.find((row) => new RegExp(`^\\|\\s*\`${code}\`\\s*\\|`).test(row));
    for (const code of codes) {
      if (!rowOf(code)) {
        fail('ssh-codes', `${COMMAND_LINE}'s SSH failure table has no row for \`${code}\``);
      }
    }
    // A failure the command line words otherwise for a login on this machine (T3-DOC-3) quotes
    // those words in its row too: the general advice sends a member on the server to IT.
    const sameHost = /pub fn same_host_connect_failure_text\([\s\S]*?\n\}/.exec(cliOutput)?.[0];
    if (sameHost === undefined) {
      fail(
        'ssh-codes',
        `${CLI_OUTPUT} has no same_host_connect_failure_text; re-read it and update this rule`
      );
    } else {
      for (const [, code, template] of sameHost.matchAll(
        /"(crew_[a-z_]+)" => Some\(format!\(\s*"((?:[^"\\]|\\.)*)"/g
      )) {
        const row = rowOf(code);
        const said = codeSpans(row?.split('|').slice(2).join('|') ?? '');
        if (row && !said.some((phrase) => saysTemplate(phrase, template))) {
          fail(
            'ssh-codes',
            `${COMMAND_LINE}'s row for \`${code}\` does not quote what a login on this machine gets: "${template}"`
          );
        }
      }
    }
  }

  // ── notifications ────────────────────────────────────────────────────────
  // The desktop app counts unread Crew messages and notifies the person while
  // they are elsewhere, a mention by name (M2). The manual said the opposite.
  const attention = need(ATTENTION, 'notifications');
  if (attention !== null) {
    const notifies =
      /mentioned you in \$\{/.test(attention) && /new messages`?\}? in \$\{/.test(attention);
    if (!notifies) {
      fail(
        'notifications',
        `${ATTENTION} no longer words "mentioned you in" and "new messages in" notifications; ` +
          're-read it, and rewrite what the manual says about notifications'
      );
    } else {
      const denies = (block) =>
        /\bCrew sends no (system )?notifications\b/i.test(block) ||
        /\bdoes(?: not|n['’]t) notify\b/i.test(block);
      for (const { path, blocks } of surfaces) {
        for (const block of blocks.filter(denies)) {
          fail(
            'notifications',
            `${path} says Crew does not notify, but ${ATTENTION} does: ${block.slice(0, 140)}`
          );
        }
      }
      const quoted = pageQuotes(MESSAGES_PAGE).map(sameQuotes);
      if (!quoted.some((phrase) => /^.+ mentioned you in #[\w-]+$/.test(phrase))) {
        fail(
          'notifications',
          `${MESSAGES_PAGE} does not quote a "… mentioned you in #channel" notification`
        );
      }
      if (!quoted.some((phrase) => /^\d+ new messages in [\w-]+$/.test(phrase))) {
        fail(
          'notifications',
          `${MESSAGES_PAGE} does not quote a "3 new messages in workspace" notification`
        );
      }
    }
  }

  // ── pause-reasons ────────────────────────────────────────────────────────
  // "Paused" is every stop a transfer can resume from, and its row says why
  // (FILES-F4). The manual lists every reason the row can show.
  const crewStatus = need(CREW_STATUS, 'pause-reasons');
  if (crewStatus !== null) {
    const table = /PAUSE_REASONS[^=]*=\s*\[([\s\S]*?)\n\];/.exec(crewStatus)?.[1] ?? '';
    // The table's reasons, and each reason kept as its own constant, such as the one a transfer
    // the workspace server could not save shows (T3-DOC-6).
    const reasons = [
      ...[...table.matchAll(/,\s*'([^']+)'\s*\]/g)].map((m) => m[1]),
      ...[...crewStatus.matchAll(/export const [A-Z_]*PAUSE_REASON = '([^']+)';/g)].map(
        (m) => m[1]
      ),
    ].map(sameQuotes);
    if (reasons.length < 3) {
      fail(
        'pause-reasons',
        `found ${reasons.length} pause reasons in ${CREW_STATUS}; update this reader`
      );
    }
    const quoted = pageQuotes(MESSAGES_PAGE).map(sameQuotes);
    for (const reason of reasons) {
      if (!quoted.includes(reason)) {
        fail('pause-reasons', `${MESSAGES_PAGE} does not list the pause reason "${reason}"`);
      }
    }
  }

  // ── cancel-upload ────────────────────────────────────────────────────────
  // An upload can be cancelled now (FILES-F7): its unfinished part stays up to
  // a day. "Cannot be deleted" is true only of a finished upload.
  const filesCopy = need(FILES_COPY, 'cancel-upload');
  if (filesCopy !== null) {
    if (!/cancelUpload:\s*'Cancel upload'/.test(filesCopy)) {
      fail('cancel-upload', `${FILES_COPY} no longer offers 'Cancel upload'; update this rule`);
    } else {
      for (const { path, blocks } of surfaces) {
        for (const block of blocks) {
          if (
            /\b(?:cannot|can['’]t) delete an upload\b|\bAn upload cannot be deleted\b/i.test(block)
          ) {
            fail(
              'cancel-upload',
              `${path} says an upload cannot be deleted: ${block.slice(0, 140)}`
            );
          }
        }
      }
      if (!(tree.read(MESSAGES_PAGE) || '').includes('**Cancel upload**')) {
        fail('cancel-upload', `${MESSAGES_PAGE} does not name **Cancel upload**`);
      }
    }
  }

  // ── privacy-confirm ──────────────────────────────────────────────────────
  // Going public from a terminal asks for the workspace's name, as the desktop
  // does, and a script passes --confirm (CLI-10).
  const cliArgs = need(CLI_ARGS, 'privacy-confirm');
  if (cliArgs !== null) {
    if (!/SetPersonal\s*\{[\s\S]{0,900}?confirm:\s*Option<String>/.test(cliArgs)) {
      fail(
        'privacy-confirm',
        `${CLI_ARGS}'s privacy set-personal no longer takes --confirm; re-read it and update the manual`
      );
    } else {
      for (const { path, blocks } of surfaces) {
        for (const block of blocks) {
          if (
            /no typed confirmation/i.test(block) ||
            (/privacy set-/.test(block) && /take effect at once/i.test(block))
          ) {
            fail(
              'privacy-confirm',
              `${path} says the privacy commands need no typed confirmation: ${block.slice(0, 140)}`
            );
          }
        }
      }
      if (!(tree.read(COMMAND_LINE) || '').includes('--confirm WORKSPACE')) {
        fail('privacy-confirm', `${COMMAND_LINE} does not say a script passes --confirm WORKSPACE`);
      }
    }
  }

  // ── share-dialog ─────────────────────────────────────────────────────────
  // The drop confirmation is a message box. macOS shows its message, never its
  // title, so the manual quotes the message (DW-15).
  const sharePath = need(SHARE_PATH, 'share-dialog');
  if (sharePath !== null) {
    const title = /title:\s*'([^']+)',\s*\n\s*message:\s*`Share "\$\{/.exec(sharePath)?.[1] ?? null;
    if (title === null) {
      fail(
        'share-dialog',
        `found no message box titled beside a 'Share "…" (…) to Crew?' message in ${SHARE_PATH}; update this reader`
      );
    } else {
      const sayMessage = /Share "[^"]+" \([^)]+\) to Crew\?/;
      for (const { path, blocks } of surfaces) {
        for (const phrase of blocks.flatMap(quotedPhrases)) {
          if (phrase === title) {
            fail(
              'share-dialog',
              `${path} names the Share window "${title}", a title macOS never shows; quote its message`
            );
          }
        }
      }
      for (const path of [MESSAGES_PAGE, `${LANDING}#crew`]) {
        const surface = surfaces.find((entry) => entry.path === path);
        if (!surface?.blocks.some((block) => sayMessage.test(sameQuotes(block)))) {
          fail(
            'share-dialog',
            `${path} does not quote the Share message, such as 'Share "counts.csv" (55 KB) to Crew?'`
          );
        }
      }
    }
  }

  // ── settings-title ───────────────────────────────────────────────────────
  // The workspace's settings dialog is titled with its name (DW-18), so a
  // reader looking for "Workspace settings" finds nothing on screen.
  const dialogsCopy = need(DIALOGS_COPY, 'settings-title');
  if (dialogsCopy !== null) {
    if (
      !/workspaceSettingsCopy\s*=\s*\{\s*title:\s*\(workspace: string\) => `\$\{workspace\} settings`/.test(
        dialogsCopy
      )
    ) {
      fail(
        'settings-title',
        `${DIALOGS_COPY}'s workspaceSettingsCopy.title changed; update this rule`
      );
    } else {
      for (const { path, blocks } of surfaces) {
        for (const block of blocks.filter((b) => /\bWorkspace settings\b/.test(b))) {
          fail(
            'settings-title',
            `${path} names a "Workspace settings" dialog, which is titled "{workspace} settings": ${block.slice(0, 140)}`
          );
        }
      }
    }
  }

  // ── product-docs ─────────────────────────────────────────────────────────
  // The landing page as a whole, and the product pages, against the provider
  // screens and key storage (W2-DOC-8). Each claim is refused only while the
  // code it contradicts is there.
  const productPages = [
    // A space after each cell, so a table row's cells stay words apart.
    { path: LANDING, blocks: htmlBlocks(landingHtml.replace(/<\/t[dh]>/g, '$& ')) },
    ...PRODUCT_DOCS.map((path) => ({
      path,
      blocks: markdownBlocks(need(path, 'product-docs') || ''),
    })),
  ];
  const refuseEverywhere = (claim, why) => {
    for (const { path, blocks } of productPages) {
      for (const block of blocks.filter((b) => claim.test(b))) {
        fail('product-docs', `${path} ${why}: ${block.slice(0, 140)}`);
      }
    }
  };
  const keyNotice = need(KEY_NOTICE, 'product-docs');
  if (keyNotice !== null) {
    if (
      !/KEY_STORAGE_NOTICE\s*=\s*\n?\s*"[^"]*when one is available, otherwise in a private file/.test(
        keyNotice
      )
    ) {
      fail(
        'product-docs',
        `${KEY_NOTICE}'s KEY_STORAGE_NOTICE changed; re-read it and update this rule`
      );
    } else {
      refuseEverywhere(
        /not in a plaintext file|API keys are encrypted|Secrets never touch disk in plaintext|stored securely in the keychain/,
        'promises keys never reach a plaintext file, but a machine with no credential store writes secrets.yaml'
      );
    }
  }
  const cardButtons = need(CARD_BUTTONS, 'product-docs');
  if (cardButtons !== null) {
    if (!/provider\.is_configured && isOnboardingPage && \(\s*<RocketButton/.test(cardButtons)) {
      fail(
        'product-docs',
        `${CARD_BUTTONS} no longer shows Launch on the first run screen only; update this rule`
      );
    } else {
      refuseEverywhere(
        /\ba (?:"Launch"|Launch) button to switch\b|\bConfigure or Launch\b/,
        'promises a Launch button on every provider card, which only the first run screen has'
      );
    }
  }
  const ordering = need(PROVIDER_ORDERING, 'product-docs');
  if (ordering !== null) {
    if (!/tabLabel:\s*'Public'/.test(ordering)) {
      fail('product-docs', `${PROVIDER_ORDERING} has no 'Public' tab; update this rule`);
    } else {
      refuseEverywhere(
        /\bCommercial Models\b|\bunder Commercial\b|\bgrouped as\b[^.]*\bCommercial\b/,
        'names a "Commercial" group, where the provider tab is Public'
      );
    }
  }
  const modeItem = need(MODE_ITEM, 'product-docs');
  if (modeItem !== null) {
    if (!/label:\s*'Autonomous'/.test(modeItem) || !/label:\s*'Chat only'/.test(modeItem)) {
      fail('product-docs', `${MODE_ITEM}'s mode labels changed; update this rule`);
    } else {
      refuseEverywhere(
        /\b(?:Completely Autonomous|Manual Approval|Smart Approval|Chat Only)\b/,
        'names a permission mode as the app does not (Autonomous, Manual, Smart, Chat only)'
      );
    }
  }
  const sagemaker = need(SAGEMAKER, 'product-docs');
  if (sagemaker !== null) {
    const model = rustStrConst(sagemaker, 'SAGEMAKER_TGI_DEFAULT_MODEL');
    if (model === null) {
      fail('product-docs', `${SAGEMAKER} defines no SAGEMAKER_TGI_DEFAULT_MODEL; update this rule`);
    } else {
      const row = htmlBlocks(landingHtml).find((b) => /^AWS SageMaker TGI/.test(b)) || '';
      if (!row.includes(model)) {
        fail(
          'product-docs',
          `${LANDING}'s SageMaker TGI row does not name its default model ${model}`
        );
      }
    }
  }

  // ── design ───────────────────────────────────────────────────────────────
  // The Crew design documents are marked Current and cited from source
  // comments, so they may not leave out what the code now does. The wave-2
  // fixes changed rows each of them pinned (W2-STR-1): the UI spec's SSH
  // failure table had no row for a refused key or a stopped workspace server,
  // the protocol contract said nothing of presence, the host's usage report or
  // a storage fault, the naming design said nothing in it was built, and the
  // CLI guide promised a code on every JSON error. A reviewer trusting such a
  // row reads the fix as a regression. Each check reads the list it holds the
  // document to from the code.
  const designSpec = tree.read(SPEC);
  const connectFailure = need(CONNECT_FAILURE, 'design');
  if (designSpec !== null && connectFailure !== null) {
    const table =
      /CONNECT_FAILURE_CODES\b[^=]*=\s*\{([\s\S]*?)\n\};/.exec(connectFailure)?.[1] ?? '';
    const codes = [...table.matchAll(/^\s*(crew_[a-z_]+):/gm)].map((m) => m[1]);
    if (codes.length < 8) {
      fail(
        'design',
        `found ${codes.length} codes in ${CONNECT_FAILURE}'s CONNECT_FAILURE_CODES; update this reader`
      );
    }
    const section = /### SSH failure classification\n([\s\S]*?)\n#{2,3} /.exec(designSpec)?.[1];
    if (section === undefined) {
      fail('design', `${SPEC} has no "### SSH failure classification" section`);
    } else {
      const firstCells = markdownBlocks(section)
        .filter((block) => block.startsWith('|'))
        .map((row) => row.split('|')[1] || '');
      for (const code of codes) {
        if (!firstCells.some((cell) => cell.includes(`\`${code}\``))) {
          fail('design', `${SPEC}'s SSH failure table has no row for \`${code}\``);
        }
      }
    }
  }
  const brokerForDesign = need(BROKER, 'design');
  const protocol = need(PROTOCOL, 'design');
  if (brokerForDesign !== null && protocol !== null) {
    const capabilities = [
      ...(/fn capabilities\(\)[\s\S]*?\n {4}\}/.exec(brokerForDesign)?.[0] ?? '').matchAll(
        /"([a-z0-9_]+)"/g
      ),
    ].map((m) => m[1]);
    if (capabilities.length < 6) {
      fail(
        'design',
        `found ${capabilities.length} capabilities in ${BROKER}'s capabilities(); update this reader`
      );
    }
    const fields = [
      ...new Set(
        [...brokerForDesign.matchAll(/\b(?:snapshot|hello)\["([a-z_]+)"\] = /g)].map((m) => m[1])
      ),
    ];
    if (!fields.includes('unread') || !fields.includes('state')) {
      fail(
        'design',
        `found no snapshot["unread"] or hello["state"] in ${BROKER}; update this reader`
      );
    }
    const codes = ['not_delivered', 'storage_full', 'storage_failed'].filter((code) =>
      brokerForDesign.includes(`"${code}"`)
    );
    for (const [kind, names] of [
      ['capability', capabilities],
      ['field', fields],
      ['refusal code', codes],
    ]) {
      for (const name of names) {
        // A code span that starts with the name: `usage {state_bytes, …}` names `usage`.
        if (!new RegExp(`\`${name}(?![a-z0-9_])`).test(protocol)) {
          fail('design', `${PROTOCOL} never names the ${kind} \`${name}\` that ${BROKER} sends`);
        }
      }
    }
  }
  const copyQuotes = [
    [TIMELINE_COPY, ['newer', 'jumpToFirstUnread', 'mentionsYou']],
    [ACCESS_COPY, ['fixedOnFirstAccess']],
    [DIALOGS_COPY, ['removeChannelMember.description']],
  ];
  for (const [file, keys] of copyQuotes) {
    const source = need(file, 'design');
    if (source === null || designSpec === null) continue;
    for (const key of keys) {
      const text = tsCopyString(source, key);
      if (text === null) {
        fail('design', `${file} has no string ${key}; update this rule's copy list`);
      } else if (!sameApostrophes(designSpec).includes(sameApostrophes(text))) {
        fail('design', `${SPEC} does not quote ${key} from ${file}: "${text}"`);
      }
    }
  }
  const naming = need(NAMING_DESIGN, 'design');
  if (
    naming !== null &&
    tree.read(NAMES_RS) !== null &&
    /Nothing in it is built yet/.test(naming)
  ) {
    fail('design', `${NAMING_DESIGN} says nothing in it is built, but ${NAMES_RS} ships it`);
  }
  const cliGuide = need(CLI_GUIDE, 'design');
  if (cliGuide !== null && refusalSource !== null) {
    for (const name of ['OUTCOME_UNKNOWN', 'NOT_SENT']) {
      const code = rustStrConst(refusalSource, name);
      if (code === null) fail('design', `${REFUSAL_RS} has no ${name}; update this reader`);
      else if (!cliGuide.includes(`\`${code}\``)) {
        fail(
          'design',
          `${CLI_GUIDE} never names \`${code}\`, the daemon's answer for a lost request`
        );
      }
    }
  }

  // ── data-paths ───────────────────────────────────────────────────────────
  // "Where Crew keeps its data" lists every folder Crew writes on a member computer (T3-DOC-2).
  // It named the saved connections and left out the task records, the file transfer receipts and
  // the folder agent tasks start in. The code names each as a `Paths::config_dir()`,
  // `data_dir()` or `state_dir()` joined with a `crew…` path, which on macOS and Linux is under
  // ~/.config/biorouter, ~/.local/share/biorouter and ~/.local/state/biorouter.
  const bases = {
    config: '~/.config/biorouter',
    data: '~/.local/share/biorouter',
    state: '~/.local/state/biorouter',
  };
  const crewSources = [
    ...CREW_SOURCE_DIRS.flatMap((dir) =>
      tree
        .list(dir)
        .filter((name) => name.endsWith('.rs') && !/tests?\.rs$/.test(name))
        .filter((name) => !dir.endsWith('/routes') || name.startsWith('crew'))
        .map((name) => `${dir}/${name}`)
    ),
  ];
  const written = [
    ...new Set(
      crewSources.flatMap((path) =>
        [
          ...(tree.read(path) || '')
            .split(/\n#\[cfg\(test\)\]\nmod \w+ \{/)[0]
            .matchAll(/Paths::(config|data|state)_dir\(\)((?:\s*\.join\("[^"]+"\))+)/g),
        ]
          .map(([, base, joins]) => [
            base,
            [...joins.matchAll(/"([^"]+)"/g)].map((m) => m[1]).join('/'),
          ])
          .filter(([, rest]) => /^crew/.test(rest))
          .map(([base, rest]) => `${bases[base]}/${rest}`)
      )
    ),
  ];
  if (written.length < 3) {
    fail(
      'data-paths',
      `found ${written.length} Crew paths in ${CREW_SOURCE_DIRS.join(', ')}; update this reader`
    );
  }
  const administration = tree.read(ADMINISTRATION) || '';
  const dataSection = /\n## Where Crew keeps its data\n([\s\S]*?)(?=\n## |$)/.exec(administration);
  if (!dataSection) {
    fail('data-paths', `${ADMINISTRATION} has no "## Where Crew keeps its data" section`);
  } else {
    const listed = markdownBlocks(dataSection[1])
      .filter((block) => block.startsWith('|'))
      .flatMap((row) => codeSpans(row.split('|')[1] || ''))
      .map((path) => path.replace(/\/+$/, ''));
    for (const path of written) {
      if (!listed.some((row) => path === row || path.startsWith(`${row}/`))) {
        fail(
          'data-paths',
          `${ADMINISTRATION}'s "Where Crew keeps its data" has no row for ${path}, which Crew writes on each computer`
        );
      }
    }
  }

  // ── allow-button ─────────────────────────────────────────────────────────
  // The Chat access pane's Allow button is one word that never changes, and the sentence above it
  // names the chat (UXN-10). The manual and the landing page named a button, "Allow “Plot review”
  // to read and post in #methods", that no screen shows any more.
  const accessForAllow = need(ACCESS_COPY, 'allow-button');
  if (accessForAllow !== null) {
    const allow = tsCopyString(accessForAllow, 'allow');
    if (allow === null) {
      fail('allow-button', `${ACCESS_COPY} has no string allow; update this rule`);
    } else if (!/[{“"]/.test(allow)) {
      for (const { path, blocks } of surfaces) {
        for (const block of blocks) {
          if (
            /\bAllow (?:“|")[^”"]+(?:”|") to read and post\b|\bAllow this conversation to read\b/.test(
              block
            )
          ) {
            fail(
              'allow-button',
              `${path} names an Allow button that carries the chat, but ${ACCESS_COPY} labels it "${allow}": ${block.slice(0, 140)}`
            );
          }
        }
      }
      if (!(tree.read(AGENTS_PAGE) || '').includes(`**${allow}**`)) {
        fail(
          'allow-button',
          `${AGENTS_PAGE} does not name the Chat access pane's **${allow}** button`
        );
      }
    }
  }

  // ── source-line ──────────────────────────────────────────────────────────
  // The daemon's last line names the work-folder files a post read (T3-BE-8), where it used to
  // say no shared file was read. The agents page quotes that form as the daemon's own render
  // cases write it, and every work-folder line it quotes is one of theirs, or a sentence of one.
  const casesText = need(SOURCE_LINE_CASES, 'source-line');
  if (casesText !== null) {
    let cases = [];
    try {
      cases = JSON.parse(casesText);
    } catch {
      fail('source-line', `${SOURCE_LINE_CASES} is not JSON; update this reader`);
    }
    const lines = (Array.isArray(cases) ? cases : cases.cases || [])
      .map((entry) => entry && entry.line)
      .filter((line) => typeof line === 'string')
      .map(sameQuotes);
    const folderLines = lines.filter((line) => /remote work folder/.test(line));
    if (folderLines.length === 0) {
      fail(
        'source-line',
        `found no work-folder source line in ${SOURCE_LINE_CASES}; update this reader`
      );
    } else {
      const quoted = pageQuotes(AGENTS_PAGE).map(sameQuotes);
      if (!quoted.some((phrase) => folderLines.includes(phrase))) {
        fail(
          'source-line',
          `${AGENTS_PAGE} does not quote a source line naming a work-folder file, such as "${folderLines[0]}"`
        );
      }
      for (const { path, blocks } of surfaces) {
        for (const phrase of quotesOf({ path, blocks }).map(sameQuotes)) {
          if (
            /remote work folder on\b/.test(phrase) &&
            !lines.some((line) => line.includes(phrase))
          ) {
            fail(
              'source-line',
              `${path} quotes "${phrase}", which no case in ${SOURCE_LINE_CASES} writes`
            );
          }
        }
      }
    }
  }

  // ── reconnect-timing ─────────────────────────────────────────────────────
  // How soon Crew connects by itself (T3-DOC-6): members were told only that it would, and waited
  // minutes after the host had started the server again. The pages give the keepalive's own
  // figures: the gaps after a drop, the late retries, and the steady retry while the server is
  // not running.
  const keepalive = need(KEEPALIVE_RS, 'reconnect-timing');
  if (keepalive !== null) {
    const defaults = /impl Default for KeepaliveTiming[\s\S]*?\n\}/.exec(keepalive)?.[0] ?? '';
    const seconds = (field) => {
      const match = new RegExp(`\\b${field}:\\s*Duration::from_secs\\(([\\d\\s*]+)\\)`).exec(
        defaults
      );
      return match
        ? match[1].split('*').reduce((product, n) => product * Number(n.trim()), 1)
        : null;
    };
    const gaps = [
      ...(/retry_delays:\s*\[([\s\S]*?)\]/.exec(defaults)?.[1] ?? '').matchAll(
        /from_secs\((\d+)\)/g
      ),
    ].map((m) => Number(m[1]));
    const late = seconds('late_retry_every');
    const down = seconds('broker_down_every');
    if (gaps.length < 2 || late === null || down === null) {
      fail(
        'reconnect-timing',
        `found no KeepaliveTiming defaults in ${KEEPALIVE_RS}; update this reader`
      );
    } else {
      const spoken = (n) =>
        n % 60 === 0 ? `${n / 60} minute${n === 60 ? '' : 's'}` : `${n} seconds`;
      const claims = [
        [TROUBLESHOOTING, `after ${spoken(gaps[0])}`],
        [TROUBLESHOOTING, `every ${spoken(late)}`],
        [TROUBLESHOOTING, `every ${spoken(down)}`],
        [COMMAND_LINE, `every ${spoken(down)}`],
      ];
      for (const [page, phrase] of claims) {
        if (!(tree.read(page) || '').includes(phrase)) {
          fail(
            'reconnect-timing',
            `${page} does not say Crew tries again "${phrase}", as ${KEEPALIVE_RS}'s KeepaliveTiming does`
          );
        }
      }
    }
  }

  // ── drop-refusals ────────────────────────────────────────────────────────
  // A file dropped or pasted into a channel is shared by `shareDroppedFile` in the main process,
  // not by the paperclip's picker. There a refusal whose code `crewFileRefusal` words gets that
  // sentence, and every other refusal gets the flow's general note ("Crew couldn't take …"),
  // whatever the daemon said; the picker and the command line show the daemon's own sentence.
  // The files page quoted the daemon's sentence for a name with hidden characters (T3-BE-10) as
  // what Crew says, while a drag got the general note (T3-DOC-6, round 2). So the page says what
  // a drag shows by reading the code: while crewFileRefusal does not word the code, the row says
  // a dropped file gets the general note, and once it does, the row may no longer say so.
  const dropSource = need(SHARE_PATH, 'drop-refusals');
  const dropDaemon = need(LOCAL_FILES_RS, 'drop-refusals');
  if (dropSource !== null && dropDaemon !== null) {
    const code = withoutLineComments(dropSource);
    const body = (name) =>
      new RegExp(`\\nexport (?:async )?function ${name}\\([\\s\\S]*?\\n\\}`).exec(code)?.[0] ?? '';
    const refusal = body('crewFileRefusal');
    const drop = body('shareDroppedFile');
    const constants = new Map(
      [...code.matchAll(/\bexport const (\w+) = '([^'\\]+)';/g)].map((m) => [m[1], m[2]])
    );
    const worded = [...refusal.matchAll(/\bcase (?:(\w+)|'([^'\\]+)'):/g)].map(
      (m) => m[2] ?? constants.get(m[1])
    );
    const general = tsLiterals(dropSource).filter((text) =>
      /^Crew couldn't take\b/.test(sameQuotes(text))
    );
    const unread = [
      [worded.length >= 3 && worded.every(Boolean), 'the codes crewFileRefusal words'],
      [
        /\bcrewFileRefusal\(/.test(drop) && /\bcrewShareCopy\.daemonRefused\(/.test(drop),
        "shareDroppedFile's refusal sentences",
      ],
      [general.length > 0, `the general note "Crew couldn't take …"`],
    ].filter(([read]) => !read);
    for (const [, what] of unread) {
      fail('drop-refusals', `could not read ${what} in ${SHARE_PATH}; update this reader`);
    }
    const quoted = [
      {
        constant: 'FILE_NAME_INVISIBLE_CODE',
        what: 'hidden-character',
        opens: /^"[^"]*" has an invisible or formatting character\b/,
      },
    ];
    for (const { constant, what, opens } of unread.length ? [] : quoted) {
      const refused = rustStrConst(dropDaemon, constant);
      if (!refused) {
        fail('drop-refusals', `${LOCAL_FILES_RS} has no ${constant}; update this reader`);
        continue;
      }
      const cased = worded.includes(refused);
      // A drop that fell back to the daemon's own sentence would show it for every code.
      const shown = cased || /\bdaemonRefusalSentence\(/.test(drop);
      // The sentence the drop flow gives the code: the one copy its case arm returns, past any
      // labels that fall through to it.
      const names = [...constants].filter(([, value]) => value === refused).map(([name]) => name);
      const arm = new RegExp(
        `\\bcase (?:${[...names, `'${refused}'`].map(escapeRegExp).join('|')}):(?:\\s*case [^:]+:)*([\\s\\S]*?)(?=\\n\\s*(?:case\\b|default:)|$)`
      ).exec(refusal);
      const keys = new Set(
        [...(arm?.[1] ?? '').matchAll(/\bcrewShareCopy\.(\w+)\b/g)].map((m) => m[1])
      );
      const copy =
        keys.size === 1 &&
        new RegExp(
          `\\n  ${[...keys][0]}:\\s*(?:\\([^)]*\\)\\s*=>\\s*)?(\`(?:[^\`\\\\]|\\\\[\\s\\S])*\`|'(?:[^'\\\\\\n]|\\\\.)*')`
        ).exec(code);
      const sentence = copy ? tsLiterals(copy[1])[0] : null;
      if (cased && !sentence) {
        fail(
          'drop-refusals',
          `could not read the sentence crewFileRefusal gives ${refused} in ${SHARE_PATH}; update this reader`
        );
      }
      const rows = markdownBlocks(tree.read(MESSAGES_PAGE) || '').filter((block) =>
        quotedPhrases(block).some((phrase) => opens.test(sameQuotes(phrase)))
      );
      if (rows.length === 0) {
        fail('drop-refusals', `${MESSAGES_PAGE} does not quote the ${what} refusal (${refused})`);
      }
      for (const row of rows) {
        const saysGeneral =
          /\bgeneral note\b/i.test(row) ||
          quotedPhrases(row).some((phrase) =>
            general.some((template) => saysTemplate(phrase, template))
          );
        const quotesDrop =
          !cased ||
          !sentence ||
          quotedPhrases(row).some(
            (phrase) => /[.!?]$/.test(phrase.trim()) && saysTemplate(phrase, sentence)
          );
        if (!quotesDrop) {
          fail(
            'drop-refusals',
            `${MESSAGES_PAGE}'s ${what} row does not quote, to the end of a sentence, what a file dropped in gets from crewFileRefusal in ${SHARE_PATH}: "${sentence}"`
          );
        }
        if (shown && saysGeneral) {
          fail(
            'drop-refusals',
            `${MESSAGES_PAGE} says a file dropped in with a ${what} name gets the general note, but crewFileRefusal in ${SHARE_PATH} now words ${refused}; delete that sentence`
          );
        } else if (!shown && !saysGeneral) {
          fail(
            'drop-refusals',
            `${MESSAGES_PAGE} quotes the daemon's ${what} refusal as what Crew says, but a file dropped or pasted in gets "${general[0]}" from shareDroppedFile in ${SHARE_PATH}, whose crewFileRefusal does not word ${refused}; word the code there, or say what a drop shows`
          );
        }
      }
    }
  }

  // ── dashes ───────────────────────────────────────────────────────────────
  // The Crew pages and their design documents are written without em or en dashes, as the
  // documentation style asks: a range is "1 to 64", an empty table cell says None, and a clause
  // takes a comma, colon or full stop. The design documents still held 156 when they were filed
  // beside the manual.
  for (const dir of [MANUAL_DIR, `${MANUAL_DIR}/design`]) {
    for (const name of tree.list(dir).filter((file) => file.endsWith('.md'))) {
      const lines = (tree.read(`${dir}/${name}`) || '').split('\n');
      lines.forEach((line, index) => {
        if (/[\u2013\u2014]/.test(line)) {
          fail(
            'dashes',
            `${dir}/${name}:${index + 1} uses an em or en dash: ${line.trim().slice(0, 100)}`
          );
        }
      });
    }
  }

  // ── work-folder ──────────────────────────────────────────────────────────
  // A command in the remote work folder runs under `confine` in the server program: Landlock
  // limits its files, and a seccomp allow list gives it no socket and no fork or clone. The
  // manual listed its size limits only, so an agent asked to submit a Slurm job spent dozens of
  // calls finding out that sbatch cannot run there (T3-DOC-5). While the allow list refuses
  // them, the pages that describe the folder say what a command cannot do.
  const remote = need(REMOTE_RS, 'work-folder');
  if (remote !== null) {
    const confine = /\nfn confine\([\s\S]*?\n\}/.exec(remote)?.[0] ?? '';
    const allowed = new Set([...confine.matchAll(/libc::SYS_([a-z0-9_]+)/g)].map((m) => m[1]));
    if (allowed.size < 20) {
      fail(
        'work-folder',
        `found ${allowed.size} system calls in ${REMOTE_RS}'s confine(); update this reader`
      );
    } else {
      const noNetwork = !['socket', 'connect', 'socketpair'].some((call) => allowed.has(call));
      const noProcesses = !['fork', 'vfork', 'clone', 'clone3'].some((call) => allowed.has(call));
      const claims = [
        { holds: noNetwork, what: 'no network', says: /\bno network\b/i },
        {
          holds: noProcesses,
          what: 'no other processes',
          says: /\bcannot start other processes\b/i,
        },
        {
          holds: noNetwork && noProcesses,
          what: 'no cluster tools such as sbatch',
          says: /`sbatch`/,
        },
      ];
      for (const { holds, what, says } of claims) {
        for (const page of [AGENTS_PAGE, ADMINISTRATION]) {
          const said = markdownBlocks(tree.read(page) || '').some((block) => says.test(block));
          if (holds && !said) {
            fail(
              'work-folder',
              `${page} does not say that a work-folder command has ${what}, which ${REMOTE_RS}'s confine() enforces`
            );
          } else if (!holds && said) {
            fail(
              'work-folder',
              `${page} says a work-folder command has ${what}, but ${REMOTE_RS}'s confine() now allows it`
            );
          }
        }
      }
    }
  }

  return failures;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const failures = checkCrewManual();
  if (failures.length) {
    console.error(failures.map((failure) => `- ${failure}`).join('\n'));
    process.exit(1);
  }
  console.log('The Crew manual agrees with the code it describes.');
}
