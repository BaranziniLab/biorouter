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
  codeSpans,
  landingCrewPage,
  markdownBlocks,
  repoTree,
  rustStrConst,
  saysTemplate,
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

// ── The wave-2 rules (W2-DOC-3, -4, -6, -7, -8). Each mutant puts back the
// manual's text from before the 2026-09-28 update, or rewords the code it
// quotes, and the rule must notice.
const COMMAND_LINE = 'docs/crew/command-line.md';
const AGENTS = 'docs/crew/agents-and-chat-access.md';
const MESSAGES = 'docs/crew/messages-and-files.md';
const PRIVACY = 'docs/crew/privacy-and-security.md';
const LANDING = 'landing/docs.html';

test('codeSpans reads single and double backtick spans', () => {
  assert.deepEqual(codeSpans('Run `auth`, then ``It said `x` twice.`` and `y`.'), [
    'auth',
    'It said `x` twice.',
    'y',
  ]);
});

test('saysTemplate: all of a sentence, its first sentences, or its opening words', () => {
  const template =
    "Crew couldn't confirm whether this reached {workspace}. Check the channel, then retry.";
  assert.ok(
    saysTemplate(
      "Crew couldn't confirm whether this reached lab. Check the channel, then retry.",
      template
    )
  );
  assert.ok(saysTemplate('Crew couldn’t confirm whether this reached lab.', template));
  assert.ok(saysTemplate('Crew couldn’t confirm whether this reached {workspace}…', template));
  assert.ok(saysTemplate("Crew couldn't confirm whether", template));
  assert.ok(!saysTemplate("Crew couldn't confirm whether this arrived at lab.", template));
  assert.ok(!saysTemplate("Crew couldn't confirm whether this reached lab. Try again.", template));
  assert.ok(!saysTemplate("Crew couldn't confirm that…", template));
});

test("daemon-sentences: a quote of the daemon's sentence is that sentence, and the pages that need it quote it", () => {
  // The manual rewords a sentence the daemon writes.
  assertCaught(
    {
      [AGENTS]: swap(
        '"This chat\'s model is fixed by its Crew access. Start a new chat to use another model."',
        '"This chat\'s model is fixed. Start a new chat."'
      ),
    },
    'daemon-sentences',
    /quotes "This chat's model is fixed\. Start a new chat\.", which is not how MODEL_FIXED_TEXT/
  );
  // The daemon rewords it and the manual keeps the old words.
  assertCaught(
    {
      'crates/biorouter/src/crew/refusal.rs': swap(
        '"This chat\'s model is fixed by its Crew access. Start a new chat to use another model."',
        '"This chat\'s model is fixed by its Crew grant. Start a new chat to use another model."'
      ),
    },
    'daemon-sentences',
    /which is not how MODEL_FIXED_TEXT/
  );
  // The command-line page loses its quote of the missing-keyring refusal (SETUPHPC-F1).
  assertCaught(
    {
      [COMMAND_LINE]: swap(
        '``This computer has no keyring service Biorouter can use. Run `biorouter crew credentials init` to keep Crew keys in an encrypted vault, then try again.``',
        'an opaque refusal'
      ),
    },
    'daemon-sentences',
    /command-line\.md does not quote CREDENTIAL_STORE_UNAVAILABLE_TEXT/
  );
  // The retry section without the crew_outcome_unknown sentence (R-3).
  assertCaught(
    {
      [COMMAND_LINE]: swap(
        "such as `Crew couldn't confirm whether this reached lab. Check the channel, then retry with the same request ID.`, ",
        ''
      ),
    },
    'daemon-sentences',
    /does not quote the crew_outcome_unknown sentence/
  );
  // A storage sentence the broker never wrote.
  assertCaught(
    {
      [COMMAND_LINE]: swap(
        'A sentence that starts `The workspace server is out of disk space`',
        'A sentence that starts `The workspace server is out of room, please restart.`'
      ),
    },
    'daemon-sentences',
    /quotes "The workspace server is out of room, please restart\.", which is not how the storage_full/
  );
  // A reader whose constant is gone fails instead of passing.
  assertCaught(
    {
      'crates/biorouter/src/crew/refusal.rs': (text) =>
        text.replaceAll('MODEL_FIXED_TEXT', 'CHAT_MODEL_SENTENCE'),
    },
    'daemon-sentences',
    /found no MODEL_FIXED_TEXT/
  );
});

test('refusal-codes and ssh-codes: every code the daemon and the command line give is in the command-line page', () => {
  // The manual as it was: no word of an uncertain outcome's code.
  assertCaught(
    {
      [COMMAND_LINE]: (text) =>
        text.replaceAll('`crew_outcome_unknown`', 'the uncertain-outcome code'),
    },
    'refusal-codes',
    /never names the daemon's refusal code `crew_outcome_unknown`/
  );
  // A new refusal code the page does not know.
  assertCaught(
    {
      'crates/biorouter/src/crew/refusal.rs': (text) =>
        `${text}\npub const SOMETHING_NEW: &str = "crew_something_new";\n`,
    },
    'refusal-codes',
    /`crew_something_new`/
  );
  // The SSH table as it was, without the key-refused and stopped-server rows (W2-DMN-5).
  assertCaught(
    {
      [COMMAND_LINE]: (text) =>
        text
          .split('\n')
          .filter((line) => !/^\| `crew_(ssh_key_refused|broker_not_running)` \|/.test(line))
          .join('\n'),
    },
    'ssh-codes',
    /no row for `crew_ssh_key_refused`/
  );
});

test('notifications: the manual may not deny what the desktop app does (M2)', () => {
  assertCaught(
    {
      [MESSAGES]: swap(
        'A channel with unread messages shows its name in bold, with a count, in the Crew sidebar. The',
        'Crew sends no system notifications or sounds. A channel with unread messages shows its name in bold, with a count, in the Crew sidebar. The'
      ),
    },
    'notifications',
    /messages-and-files\.md says Crew does not notify/
  );
  assertCaught(
    {
      [LANDING]: swap(
        '<li>Typing <code>@bob</code> mentions Bob.',
        '<li>Typing <code>@bob</code> does not notify Bob.'
      ),
    },
    'notifications',
    /landing\/docs\.html#crew says Crew does not notify/
  );
  assertCaught(
    { [MESSAGES]: (text) => text.replaceAll('mentioned you in #general', 'wrote in #general') },
    'notifications',
    /does not quote a "… mentioned you in #channel" notification/
  );
});

test('pause-reasons: every reason a paused transfer shows is listed (FILES-F4)', () => {
  assertCaught(
    { [MESSAGES]: (text) => text.replaceAll('"The connection dropped"', 'a dropped connection') },
    'pause-reasons',
    /does not list the pause reason "The connection dropped"/
  );
  assertCaught(
    {
      'ui/desktop/src/components/crew/state/crewStatus.ts': swap(
        "[/^Transfer stopped\\b/, 'It stopped'],",
        "[/^Transfer stopped\\b/, 'It stopped'],\n  [/^Disk full\\b/, 'The disk is full'],"
      ),
    },
    'pause-reasons',
    /does not list the pause reason "The disk is full"/
  );
});

test('cancel-upload: an unfinished upload can be cancelled, so the manual may not say it cannot (FILES-F7)', () => {
  assertCaught(
    {
      [MESSAGES]: swap(
        '- Crew cannot delete a finished upload, even one you remove from your message.',
        '- Crew cannot delete an upload, even one you remove from your message.'
      ),
    },
    'cancel-upload',
    /says an upload cannot be deleted/
  );
  assertCaught(
    { [MESSAGES]: (text) => text.replaceAll('**Cancel upload**', 'the ×') },
    'cancel-upload',
    /does not name \*\*Cancel upload\*\*/
  );
});

test('privacy-confirm: going public from a terminal asks for the name (CLI-10)', () => {
  assertCaught(
    {
      [PRIVACY]: swap(
        'To change it from a terminal, see [Privacy settings](command-line.md#privacy-settings). Going public there asks you to type the workspace name, as the desktop does; in a script, add `--confirm WORKSPACE`.',
        'To change it from a terminal, see [Privacy settings](command-line.md#privacy-settings), where the commands ask for no typed confirmation.'
      ),
    },
    'privacy-confirm',
    /privacy-and-security\.md says the privacy commands need no typed confirmation/
  );
  assertCaught(
    {
      [LANDING]: swap(
        'ask you to type the workspace name first, as the desktop app does. In a script, add <code>--confirm WORKSPACE</code>.',
        'take effect at once, with no typed confirmation.'
      ),
    },
    'privacy-confirm',
    /landing\/docs\.html#crew says the privacy commands need no typed confirmation/
  );
});

test('share-dialog: the manual quotes the Share message, never the title macOS hides (DW-15)', () => {
  assertCaught(
    {
      [LANDING]: swap(
        `Crew asks, for example, 'Share "counts.csv" (55 KB) to Crew?'. Check the full path it shows, and choose`,
        'check the full path in the "Share file to Crew" window and choose'
      ),
    },
    'share-dialog',
    /names the Share window "Share file to Crew"/
  );
});

test('settings-title: the settings dialog is named by its title (DW-18)', () => {
  assertCaught(
    {
      [AGENTS]: swap(
        '- The **Agent access** tab of "{workspace} settings": workspace menu > **Agent access…**.',
        '- **Agent access** in Workspace settings: workspace menu > **Agent access…**.'
      ),
    },
    'settings-title',
    /agents-and-chat-access\.md names a "Workspace settings" dialog/
  );
});

test('product-docs: the landing and product pages match the provider screens and key storage (W2-DOC-8)', () => {
  // The audited key storage promise, back in the secret-storage page.
  assertCaught(
    {
      'docs/security/secret-storage.md': swap(
        'While the credential store answers, secrets never\ntouch disk in plaintext,',
        'Secrets never touch disk in plaintext,'
      ),
    },
    'product-docs',
    /secret-storage\.md promises keys never reach a plaintext file/
  );
  // A Launch button on every card.
  assertCaught(
    {
      'docs/getting-started/installation.md': swap(
        '**Desktop:** to change the model of a chat,',
        '**Desktop:** Settings > Models > select a provider card > Configure or Launch. To change the model of a chat,'
      ),
    },
    'product-docs',
    /installation\.md promises a Launch button/
  );
  // The "Commercial" group, in a provider page and in the landing page.
  assertCaught(
    {
      'docs/providers/zai-glm.md': swap(
        'Appears in the one provider list, which has no group headings,',
        'Appears in the provider list under Commercial,'
      ),
    },
    'product-docs',
    /zai-glm\.md names a "Commercial" group/
  );
  // The old mode names in a table row, whose cells a reader must keep apart.
  assertCaught(
    {
      [LANDING]: swap(
        '<tr><td>Manual</td><td><code>approve</code></td>',
        '<tr><td>Manual Approval</td><td><code>approve</code></td>'
      ),
    },
    'product-docs',
    /names a permission mode as the app does not/
  );
  // The SageMaker row without its default.
  assertCaught(
    {
      [LANDING]: swap(
        '<td><strong>AWS SageMaker TGI</strong></td><td><code>sagemaker-tgi-endpoint</code></td>',
        '<td><strong>AWS SageMaker TGI</strong></td><td>none</td>'
      ),
    },
    'product-docs',
    /SageMaker TGI row does not name its default model sagemaker-tgi-endpoint/
  );
  // The provider screens stop showing Launch on the first run screen only: the rule says so.
  assertCaught(
    {
      'ui/desktop/src/components/settings/providers/subcomponents/buttons/DefaultCardButtons.tsx': (
        text
      ) =>
        text.replace(
          'provider.is_configured && isOnboardingPage && (',
          'provider.is_configured && ('
        ),
    },
    'product-docs',
    /no longer shows Launch on the first run screen only/
  );
});

test('the tree as committed passes every rule', () => {
  assert.deepEqual(checkCrewManual(real), []);
});
