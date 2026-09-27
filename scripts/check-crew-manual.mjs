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
const SPEC = 'docs/research/biorouter-crew/ui-redesign-spec.md';
const DROP_ZONE = 'ui/desktop/src/components/crew/files/FileDropZone.tsx';
const MESSAGE_BODY = 'ui/desktop/src/components/crew/timeline/MessageBody.tsx';
const FILES_COPY = 'ui/desktop/src/components/crew/files/copy.ts';
const CREW_APP = 'ui/desktop/src/components/crew/CrewApp.tsx';
const NEEDS_DESKTOP = 'ui/desktop/src/components/crew/CrewNeedsDesktop.tsx';
const CREW_AUTHENTICATION = 'crates/biorouter-server/src/routes/crew_authentication.rs';
const TROUBLESHOOTING = 'docs/crew/connections-and-troubleshooting.md';

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
 * list item. Fenced code is dropped, because a command example is not a claim.
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
    else if (/^\s*(\||[-*+] |\d+\. |>)/.test(line)) {
      flush();
      blocks.push(line.trim());
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

/** Every double-quoted phrase in a block of prose, straight or curly quotes. */
const quotedPhrases = (text) => [...text.matchAll(/["“]([^"”\n]+)["”]/g)].map((m) => m[1]);

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
