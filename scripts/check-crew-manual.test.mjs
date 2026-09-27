// The mutant suite for check-crew-manual.mjs. A checker that cannot fail is not
// a gate, so each rule is shown failing on the text it exists to refuse: the
// exact wording the 2026-09-27 Crew audit found (DOCS-1, DOCS-2, DOCS-5, DOCS-6,
// RENDERER-6), or the first fix's wording its review refused, put back into an
// otherwise real tree. The last test is the gate
// itself: the tree as committed passes every rule.
//
// Run: node --test scripts/check-crew-manual.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  checkCrewManual,
  landingCrewPage,
  markdownBlocks,
  repoTree,
  rustStrConst,
} from './check-crew-manual.mjs';

const real = repoTree();

/**
 * The real tree with some files rewritten. `edits` maps a path to a function of
 * its current text; a function that returns null deletes the file.
 */
function overlay(edits, base = real) {
  return {
    read(path) {
      const text = base.read(path);
      return path in edits ? edits[path](text) : text;
    },
    list: (dir) => base.list(dir),
  };
}

/** Replace `from` with `to` exactly once, so a mutant never silently misses. */
const swap = (from, to) => (text) => {
  assert.equal(
    text.split(from).length - 1,
    1,
    `expected exactly one occurrence of: ${from.slice(0, 80)}`
  );
  return text.replace(from, to);
};

// The dialog titles are the other half of DOCS-6 (ui/desktop/src/main.ts). The
// mutants below are about the manual, so they run against main.ts with the
// brand spelled right whatever state that file is in, and the brand mutants
// then re-break exactly one side.
const brandFixedMain = (text) =>
  text
    .replace(
      "'Set approval secret for shared BioRouter daemon'",
      "'Set approval secret for shared Biorouter daemon'"
    )
    .replaceAll("'Connect to existing BioRouter daemon'", "'Connect to existing Biorouter daemon'");
const fixed = overlay({ 'ui/desktop/src/main.ts': brandFixedMain });

const failuresOf = (tree, rule) =>
  checkCrewManual(tree).filter((failure) => failure.startsWith(`${rule}:`));

/** The mutant adds a failure for `rule` that the baseline does not have. */
function assertCaught(edits, rule, pattern) {
  const before = failuresOf(fixed, rule);
  const after = failuresOf(overlay(edits, fixed), rule);
  const added = after.filter((failure) => !before.includes(failure));
  assert.ok(
    added.length > 0,
    `rule ${rule} did not catch the mutant; it reported ${JSON.stringify(after)}`
  );
  if (pattern)
    assert.ok(
      added.some((failure) => pattern.test(failure)),
      `unexpected failures: ${added.join('\n')}`
    );
}

test('markdownBlocks keeps table rows and list items apart and drops fenced code', () => {
  const blocks = markdownBlocks(
    'One\ntwo.\n\n| a | b |\n- item\n```bash\nbiorouter serve\n```\nlast'
  );
  assert.deepEqual(blocks, ['One two.', '| a | b |', '- item', 'last']);
});

test('landingCrewPage reads only the Crew page', () => {
  const html =
    '<div class="doc-page" id="doc-cli">serve</div><div class="doc-page" id="doc-crew">crew</div><div class="doc-page" id="doc-x">x</div>';
  assert.equal(landingCrewPage(html), '<div class="doc-page" id="doc-crew">crew</div>');
});

test('brand: the audited "BioRouter" quote is refused (DOCS-6)', () => {
  assertCaught(
    {
      'docs/crew/privacy-and-security.md': swap(
        '"Set approval secret for shared Biorouter daemon"',
        '"Set approval secret for shared BioRouter daemon"'
      ),
    },
    'brand',
    /privacy-and-security\.md spells the brand/
  );
  assertCaught(
    {
      'docs/crew/README.md': swap(
        '[Crew implementation and evidence]',
        '[BioRouter Crew implementation and evidence]'
      ),
    },
    'brand'
  );
});

test('brand: an identifier in inline code is not the brand', () => {
  const tree = overlay(
    {
      'docs/crew/administration.md': (text) =>
        `${text}\n\nThe helper is \`BioRouter Computer Use.app\`.\n`,
    },
    fixed
  );
  assert.deepEqual(failuresOf(tree, 'brand'), failuresOf(fixed, 'brand'));
});

test('dialog-titles: the manual and main.ts cannot disagree in either direction (DOCS-6)', () => {
  // The app keeps the old spelling while the manual moved: the manual quotes a window nobody sees.
  assertCaught(
    {
      'ui/desktop/src/main.ts': (text) =>
        text.replace(
          "'Connect to existing Biorouter daemon'",
          "'Connect to existing BioRouter daemon'"
        ),
    },
    'dialog-titles',
    /quotes "Connect to existing Biorouter daemon", but .* titles it "Connect to existing BioRouter daemon"/
  );
  // The app renames a window and the manual does not follow.
  assertCaught(
    {
      'ui/desktop/src/main.ts': swap(
        "'Confirm shared daemon approval secret'",
        "'Confirm your shared daemon approval secret'"
      ),
    },
    'dialog-titles',
    /does not quote the dialog title "Confirm your shared daemon approval secret"/
  );
  // The vault windows, titled through a ternary, are read too.
  assertCaught(
    {
      'docs/crew/privacy-and-security.md': swap(
        '"Initialize Crew encrypted vault"',
        '"Initialize crew encrypted vault"'
      ),
    },
    'dialog-titles',
    /titles it "Initialize Crew encrypted vault"/
  );
});

test('dialog-titles: a main.ts the reader cannot parse fails instead of passing', () => {
  assertCaught(
    { 'ui/desktop/src/main.ts': (text) => text.replaceAll('promptNativeSecret(', 'askForSecret(') },
    'dialog-titles',
    /found 0 daemon approval-secret titles/
  );
});

test('serve: the audited sign-in advice for a serve browser is refused (DOCS-1)', () => {
  assertCaught(
    {
      'docs/crew/getting-started.md': swap(
        'In a terminal, `biorouter crew auth` signs you in instead',
        'In a web browser (`biorouter serve`), sign in from the desktop app or with `biorouter crew auth` instead'
      ),
    },
    'serve',
    /getting-started\.md names biorouter serve without saying Crew does not work there/
  );
});

test('serve: getting-started must state the limit, and the browser guides must list Crew', () => {
  assertCaught(
    {
      'docs/crew/getting-started.md': (text) =>
        text
          .split('\n\n')
          .filter((paragraph) => !paragraph.includes('`biorouter serve`'))
          .join('\n\n'),
    },
    'serve',
    /getting-started\.md must say that Crew does not work/
  );
  assertCaught(
    {
      'docs/deployment/browser-access.md': (text) =>
        text
          .split('\n')
          .filter((l) => !l.startsWith('| Crew |'))
          .join('\n'),
    },
    'serve',
    /browser-access\.md needs a "\| Crew \| \*\*Not available/
  );
  assertCaught(
    {
      'landing/docs.html': (text) =>
        text
          .split('\n')
          .filter((l) => !l.includes('<tr><td>Crew</td>'))
          .join('\n'),
    },
    'serve',
    /What works in a browser" table needs a Crew row/
  );
  assertCaught(
    {
      'README.md': swap(
        'Crew runs in the desktop app and in the `biorouter crew` commands, not in a `biorouter serve` browser.',
        'Crew runs in the desktop app, the `biorouter crew` commands and a `biorouter serve` browser.'
      ),
    },
    'serve',
    /README\.md names Crew and biorouter serve/
  );
});

test('serve: a crew.rs that stops refusing keyless daemons fails the rule instead of passing it', () => {
  assertCaught(
    {
      'crates/biorouter-server/src/routes/crew.rs': (text) =>
        text.replaceAll('NoKeyInstalled', 'NoKeyAvailable'),
    },
    'serve',
    /no longer maps NoKeyInstalled/
  );
});

test('host-reach: the audited "or the host" rows are refused (DOCS-2)', () => {
  assertCaught(
    {
      'docs/crew/teams-channels-and-people.md': swap(
        '| Add people to a team | The team owner, or the host if the host is in the team |',
        '| Add people to a team | The team owner or the host |'
      ),
    },
    'host-reach',
    /Add people to a team/
  );
  assertCaught(
    {
      'docs/crew/teams-channels-and-people.md': swap(
        '| Add people to a channel | The channel owner, or the host if the host is in the channel |',
        '| Add people to a channel | The channel owner, or the host through the team |'
      ),
    },
    'host-reach',
    /Add people to a channel/
  );
  assertCaught(
    {
      'landing/docs.html': swap(
        "<tr><td>Add people</td><td>The team's creator or the channel owner, and the host in a team or channel the host is in</td>",
        "<tr><td>Add people</td><td>The team's creator or the channel owner, and the host</td>"
      ),
    },
    'host-reach',
    /landing\/docs\.html#crew lets the host add people/
  );
});

test('readme: the front page names Crew while the sidebar has it (DOCS-5)', () => {
  assertCaught(
    {
      'README.md': (text) =>
        text
          .split('\n')
          .filter((l) => !l.startsWith('| **Work with your lab** |'))
          .join('\n'),
    },
    'readme',
    /no row for Crew/
  );
  assertCaught(
    { 'README.md': (text) => text.replaceAll('](docs/crew/README.md)', '](docs/README.md)') },
    'readme',
    /does not link the Crew user manual/
  );
});

test('spec: the redesign spec may not deny what shipped (RENDERER-6)', () => {
  const spec = 'docs/research/biorouter-crew/ui-redesign-spec.md';
  const statusLine = (text) => text.split('\n').find((line) => line.startsWith('> **Status:**'));
  assertCaught(
    {
      [spec]: (text) =>
        text.replace(
          statusLine(text),
          '> **Status:** Current. Approved design, 2026-09-23. Nothing in it is built yet; the implementation workplan builds it in packages.'
        ),
    },
    'spec',
    /says nothing in it is built/
  );
  assertCaught(
    {
      [spec]: (text) =>
        text.replace(
          /\| Attach \| .*\|/,
          "| Attach | Ghost round `Paperclip`. No drag-and-drop: Crew's file capability comes only from the main-process picker. |"
        ),
    },
    'spec',
    /no drag and drop/
  );
  assertCaught(
    {
      [spec]: (text) =>
        text.replace(
          /\| \*\*Body\*\* \| .*\|/,
          '| **Body** | `text-body whitespace-pre-wrap [overflow-wrap:anywhere]`, plain text as today; long bodies fold with `utils/messageClamp.ts`. |'
        ),
    },
    'spec',
    /plain text/
  );
  assertCaught(
    {
      [spec]: (text) =>
        text.replace(
          /\| `file\.\*` \| .*\|/,
          '| `file.*` | **Save attachment** (tooltip Save {name}) · **Preview image** · Attachment · Copy file ID · Copy SHA-256 |'
        ),
    },
    'spec',
    /Save attachment/
  );
});

test('rustStrConst reads a Rust string constant as rustc would', () => {
  const source =
    'pub const OTHER: &str = "no";\n' +
    'pub const SENTENCE: &str = "Crew isn\'t \\\n    here. Say \\"why\\".";\n';
  assert.equal(rustStrConst(source, 'SENTENCE'), 'Crew isn\'t here. Say "why".');
  assert.equal(rustStrConst(source, 'MISSING'), null);
});

// The page a `biorouter serve` browser shows in place of Crew (CROSSCUT-5), and
// the sentence its daemon refuses a Crew request with.
const TROUBLE = 'docs/crew/connections-and-troubleshooting.md';
const NEEDS_DESKTOP = 'ui/desktop/src/components/crew/CrewNeedsDesktop.tsx';
const AUTHENTICATION = 'crates/biorouter-server/src/routes/crew_authentication.rs';
const ROUND_ONE_BROWSER_ACCESS =
  '| Crew | **Not available.** Every Crew action, even listing saved workspaces, needs the approval secret a person types into the desktop application or `biorouter crew`, and the daemon `biorouter serve` starts never holds one. The Crew routes refuse it with `crew_human_authority_unavailable`, and no setting in the browser changes that. Use Crew in the desktop application, or with `biorouter crew` in a terminal. See the [Crew user manual](../crew/README.md). |';

test('needs-desktop: the first fix, which named no page a serve browser shows, is refused (DOCS-1)', () => {
  assertCaught(
    {
      'docs/crew/getting-started.md': (text) =>
        text
          .split('\n')
          .map((line) =>
            line.startsWith('Crew works only in the desktop app')
              ? 'Crew works only in the desktop app and with `biorouter crew`. It does not work in a web browser opened with `biorouter serve`: the background service that `biorouter serve` starts never holds the [approval secret](#the-approval-secret) Crew needs, so it refuses every Crew action, even listing your workspaces. Signing in again or restarting does not change that.'
              : line
          )
          .join('\n'),
    },
    'needs-desktop',
    /getting-started\.md does not quote the page a serve browser shows, "Crew needs the Biorouter desktop app"/
  );
  assertCaught(
    {
      'docs/crew/README.md': swap(
        ': it shows "Crew needs the Biorouter desktop app" there, for the reason',
        ', for the reason'
      ),
    },
    'needs-desktop',
    /docs\/crew\/README\.md does not quote the page/
  );
  assertCaught(
    {
      'docs/deployment/browser-access.md': (text) =>
        text
          .split('\n')
          .map((line) => (line.startsWith('| Crew |') ? ROUND_ONE_BROWSER_ACCESS : line))
          .join('\n'),
    },
    'needs-desktop',
    /browser-access\.md's Crew row must open "\*\*Not available, and it says so before you try\.\*\*"/
  );
  assertCaught(
    {
      'landing/docs.html': (text) =>
        text.replace(
          /<tr><td>Crew<\/td><td>No\b[^\n]*<\/td><\/tr>/,
          '<tr><td>Crew</td><td>No. Every Crew action needs the approval secret a person types into the desktop app or <code>biorouter crew</code>, and the daemon <code>serve</code> starts never holds one. Use Crew in the desktop app or with <code>biorouter crew</code> in a terminal.</td></tr>'
        ),
    },
    'needs-desktop',
    /"What works in a browser" Crew row does not quote "Crew needs the Biorouter desktop app"/
  );
  assertCaught(
    {
      [TROUBLE]: (text) =>
        text
          .split('\n')
          .filter((line) => !line.startsWith('| "Crew needs the Biorouter desktop app"'))
          .join('\n'),
    },
    'needs-desktop',
    /has no message row for "Crew needs the Biorouter desktop app"/
  );
});

test('needs-desktop: the manual may not send a serve browser to a message it never shows (DOCS-1)', () => {
  // The sign-in row: CrewAuthentication never mounts in a serve browser any more.
  assertCaught(
    {
      [TROUBLE]: swap(
        '| "The local daemon is not available." or "Invalid daemon authentication session." | Quit and reopen Biorouter. |\n',
        '| "The local daemon is not available." or "Invalid daemon authentication session." | Quit and reopen Biorouter. |\n' +
          '| "Signing in needs the Biorouter desktop app." | You are in a web browser, where Crew does not work ([Getting started](getting-started.md)). Use the desktop app, or run `biorouter crew auth` in a terminal. |\n'
      ),
    },
    'needs-desktop',
    /tells a serve browser reader about "Signing in needs the Biorouter desktop app\."/
  );
  // The keyless refusal's row, led by browser advice for a message the browser no longer shows.
  assertCaught(
    {
      [TROUBLE]: swap(
        '| "This daemon cannot verify human Crew actions…" | See [Replace an old background service](#replace-an-old-background-service). |',
        '| "This daemon cannot verify human Crew actions…" | In a web browser opened with `biorouter serve`, Crew never works, and nothing you do there changes that. Use the desktop app, or `biorouter crew` in a terminal. In the desktop app, see [Replace an old background service](#replace-an-old-background-service). |'
      ),
    },
    'needs-desktop',
    /tells a serve browser reader about "This daemon cannot verify human Crew actions…"/
  );
  // The restart section's note that the refusal "also appears in every web browser".
  assertCaught(
    {
      [TROUBLE]: swap(
        '- "This daemon cannot verify human Crew actions…"\n\nTo replace',
        '- "This daemon cannot verify human Crew actions…"\n\n' +
          'The last one also appears in every web browser opened with `biorouter serve`. That service never holds the approval secret Crew needs, so Crew never works there, and the steps below do not help. Use the desktop app or `biorouter crew` instead.\n\nTo replace'
      ),
    },
    'needs-desktop',
    /"Replace an old background service" sends a browser reader to a restart/
  );
});

test('needs-desktop: the page title and the manual cannot drift in either direction', () => {
  // The page is renamed and the manual does not follow: its old quotes are refused too.
  assertCaught(
    {
      [NEEDS_DESKTOP]: swap(
        "title: 'Crew needs the Biorouter desktop app'",
        "title: 'Crew needs the desktop app'"
      ),
    },
    'needs-desktop',
    /quotes "Crew needs the Biorouter desktop app", but .* titles the page "Crew needs the desktop app"/
  );
  // A reader that cannot find the title fails instead of passing.
  assertCaught(
    { [NEEDS_DESKTOP]: swap("title: 'Crew needs", "heading: 'Crew needs") },
    'needs-desktop',
    /found no crewNeedsDesktopCopy\.title/
  );
  // CrewApp stops showing the page in a browser: the manual's account of it is then unfounded.
  assertCaught(
    {
      'ui/desktop/src/components/crew/CrewApp.tsx': swap(
        'if (isBrowserSurface()) return <CrewNeedsDesktop />;',
        ''
      ),
    },
    'needs-desktop',
    /no longer returns <CrewNeedsDesktop \/> on the browser surface/
  );
});

test("needs-desktop: the daemon's serve sentence is quoted as it begins, and only while it exists", () => {
  const refusal = rustStrConst(real.read(AUTHENTICATION) || '', 'CREW_NEEDS_THE_DESKTOP');
  assert.ok(refusal, `${AUTHENTICATION} defines CREW_NEEDS_THE_DESKTOP`);
  // The daemon rewords its sentence and the manual keeps the old words.
  assertCaught(
    {
      [AUTHENTICATION]: swap(
        '"Crew isn\'t available in a browser opened with \\',
        '"Crew is not available in a browser that \\'
      ),
    },
    'needs-desktop',
    /quotes "Crew isn't available in a browser opened with biorouter serve…", which is not how/
  );
  // The constant is gone: a manual that still quotes it describes a refusal nobody gives.
  assertCaught(
    {
      [AUTHENTICATION]: (text) =>
        text.replaceAll('CREW_NEEDS_THE_DESKTOP', 'SERVE_BROWSER_REFUSAL'),
    },
    'needs-desktop',
    /defines no CREW_NEEDS_THE_DESKTOP/
  );
  // The manual misquotes it.
  assertCaught(
    {
      [TROUBLE]: swap(
        '"Crew isn\'t available in a browser opened with biorouter serve…"',
        '"Crew isn\'t available in the browser…"'
      ),
    },
    'needs-desktop',
    /quotes "Crew isn't available in the browser…", which is not how/
  );
});

test('the tree as committed passes every rule', () => {
  assert.deepEqual(checkCrewManual(real), []);
});
