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
  rustLiterals,
  rustStrConst,
  saysTemplate,
  tsCopyString,
  tsLiterals,
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
      'docs/crew/README.md': swap('[Crew build campaign]', '[BioRouter Crew build campaign]'),
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
  const spec = 'docs/crew/design/ui-redesign-spec.md';
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
const CLI_OUTPUT_RS = 'crates/biorouter-cli/src/commands/crew/output.rs';

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

test('tsCopyString reads a nested copy string and says when the path is gone', () => {
  const source = [
    'export const copy = {',
    "  newer: 'Newer messages',",
    '  removeMember: {',
    '    title: (who: string) => `Remove ${who}?`,',
    '    description:',
    "      'They’ll lose access. It\\'s kept.',",
    '  },',
    '};',
  ].join('\n');
  assert.equal(tsCopyString(source, 'newer'), 'Newer messages');
  assert.equal(tsCopyString(source, 'removeMember.description'), "They’ll lose access. It's kept.");
  assert.equal(tsCopyString(source, 'removeMember.confirm'), null);
  assert.equal(tsCopyString(source, 'missing.description'), null);
});

// The Crew design documents as they stood before the wave-2 amendments
// (W2-STR-1): each mutant puts one pre-amendment row back into the real tree.
const SPEC_DOC = 'docs/crew/design/ui-redesign-spec.md';
const PROTOCOL_DOC = 'docs/crew/design/protocol-contract.md';
const NAMING_DOC = 'docs/crew/design/naming-design.md';
const CLI_GUIDE_DOC = 'docs/crew/design/cli-guide.md';

test('design: the UI spec has a row for every connect failure the desktop words (W2-STR-1, F5, R-7)', () => {
  const dropRow = (code) => (text) =>
    text
      .split('\n')
      .filter((line) => !line.startsWith(`| \`${code}\` |`))
      .join('\n');
  assertCaught({ [SPEC_DOC]: dropRow('crew_ssh_key_refused') }, 'design', /crew_ssh_key_refused/);
  assertCaught(
    { [SPEC_DOC]: dropRow('crew_broker_not_running') },
    'design',
    /crew_broker_not_running/
  );
  // The other direction: a code the desktop learns later needs a row too.
  assertCaught(
    {
      'ui/desktop/src/components/crew/state/connectFailure.ts': swap(
        "  crew_ssh_failed: 'ssh_failed',",
        "  crew_ssh_failed: 'ssh_failed',\n  crew_ssh_banner_refused: 'ssh_failed',"
      ),
    },
    'design',
    /crew_ssh_banner_refused/
  );
  // A reader that finds nothing fails rather than passing.
  assertCaught(
    {
      'ui/desktop/src/components/crew/state/connectFailure.ts': (text) =>
        text.replaceAll('CONNECT_FAILURE_CODES', 'CONNECT_CODES'),
    },
    'design',
    /update this reader/
  );
});

test('design: the protocol contract names what the broker sends (W2-STR-1, M1, M18, R-2, R-3)', () => {
  assertCaught(
    { [PROTOCOL_DOC]: (text) => text.replaceAll('`presence_v1`', 'presence') },
    'design',
    /capability `presence_v1`/
  );
  assertCaught(
    { [PROTOCOL_DOC]: (text) => text.replaceAll('`online_principal_ids`', 'who is online') },
    'design',
    /field `online_principal_ids`/
  );
  assertCaught(
    { [PROTOCOL_DOC]: (text) => text.replaceAll('`usage', '`the usage report') },
    'design',
    /field `usage`/
  );
  assertCaught(
    { [PROTOCOL_DOC]: (text) => text.replaceAll('`not_delivered`', 'a refusal') },
    'design',
    /refusal code `not_delivered`/
  );
  // A capability the broker advertises later is caught from the code's side.
  assertCaught(
    {
      'crates/biorouter-crew/src/broker.rs': swap(
        '            "presence_v1",\n',
        '            "presence_v1",\n            "typing_v1",\n'
      ),
    },
    'design',
    /capability `typing_v1`/
  );
});

test('design: the UI spec quotes the wave-2 strings it pins (W2-STR-1, M6, M7, AG-F1, M11)', () => {
  assertCaught(
    { [SPEC_DOC]: (text) => text.replaceAll('Jump to first unread', 'Jump to the unread') },
    'design',
    /jumpToFirstUnread/
  );
  assertCaught(
    {
      [SPEC_DOC]: (text) =>
        text.replaceAll(
          "The first access fixes this chat's workspace, channel and model.",
          'Access ends when you revoke it.'
        ),
    },
    'design',
    /fixedOnFirstAccess/
  );
  assertCaught(
    {
      [SPEC_DOC]: (text) =>
        text.replaceAll('You can add them again with Add people.', 'You can invite them again.'),
    },
    'design',
    /removeChannelMember\.description/
  );
  assertCaught(
    {
      'ui/desktop/src/components/crew/timeline/copy.ts': (text) =>
        text.replace('jumpToFirstUnread:', 'jumpToUnread:'),
    },
    'design',
    /has no string jumpToFirstUnread/
  );
});

test('design: the naming design and the CLI guide may not deny what shipped (W2-STR-1, DW-10, R-3)', () => {
  const statusLine = (text) => text.split('\n').find((line) => line.startsWith('> **Status:**'));
  assertCaught(
    {
      [NAMING_DOC]: (text) =>
        text.replace(
          statusLine(text),
          '> **Status:** Current. Approved design, 2026-09-23. Nothing in it is built yet; [implementation status](implementation-status.md) records progress.'
        ),
    },
    'design',
    /says nothing in it is built/
  );
  assertCaught(
    { [CLI_GUIDE_DOC]: (text) => text.replaceAll('`crew_outcome_unknown`', 'an unknown outcome') },
    'design',
    /crew_outcome_unknown/
  );
});

// ── The third round (T3-DOC-*). Each mutant puts back the manual's text as the
// 2026-09-28 live check found it, or rewords the code it quotes.
const HOSTING = 'docs/crew/hosting-a-workspace.md';
const SHARE_PATH = 'ui/desktop/src/utils/crewSharePath.ts';
const DIALOGS_COPY = 'ui/desktop/src/components/crew/dialogs/copy.ts';

test('markdownBlocks keeps a list item whole over its wrapped lines', () => {
  assert.deepEqual(markdownBlocks('- After "one\n  two", three.\n- next\n\npara'), [
    '- After "one two", three.',
    '- next',
    'para',
  ]);
});

test('rustLiterals and tsLiterals read the strings the code shows, not its comments or tests', () => {
  const rust = [
    '/// "Added in a comment"',
    'fn f() { format!("Added {person} to {}. \\',
    '    They can now see {}.", a, b); }',
    '#[cfg(test)]',
    'mod tests {',
    '    const X: &str = "Added in a test";',
    '}',
  ].join('\n');
  assert.deepEqual(rustLiterals(rust), ['Added {person} to {}. They can now see {}.']);
  const ts = [
    '/** "Identity file must be an absolute path" */',
    "// 'a comment'",
    'export const copy = {',
    "  plain: 'It’s \\'plain\\'.',",
    '  held: (who: string) => `Added ${who} to ${team}.`,',
    '};',
  ].join('\n');
  assert.deepEqual(tsLiterals(ts), ["It’s 'plain'.", 'Added {who} to {team}.']);
});

test('app-sentences: the manual quotes the members add line, the share note and the Identity file note as they are shown (T3-DOC-1)', () => {
  // The three quotes as the live check found them.
  assertCaught(
    {
      [HOSTING]: swap(
        'It prints a line such as `Added "Bob Lee" (@bob) to #methods.`',
        'It prints "Added. @bob can now see #methods."'
      ),
    },
    'app-sentences',
    /hosting-a-workspace\.md quotes "Added\. @bob can now see #methods\.", which is not how the "Added …" summaries/
  );
  assertCaught(
    {
      [MESSAGES]: (text) =>
        text.replace(
          /- After "Your connection is now Private;[\s\S]*?note that asks you to refresh the workspace\./,
          '- After "Connection privacy changed" or a note that asks you to refresh the workspace, wait until\n  the status row reads "Connected", then drop or choose the file again.'
        ),
    },
    'app-sentences',
    /messages-and-files\.md quotes "Connection privacy changed", which is not how the crew_mode_mismatch share notes/
  );
  assertCaught(
    {
      [TROUBLE]: (text) =>
        text.replace(
          /^\| "Use the key file’s full path…".*$/m,
          '| "Identity file must be an absolute path" | Enter a path that starts with `/`, or leave the field empty. |'
        ),
    },
    'app-sentences',
    /connections-and-troubleshooting\.md does not quote the Identity file note/
  );
  // The code rewords a note and the manual keeps the old words.
  assertCaught(
    {
      [SHARE_PATH]: swap(
        'this file was checked for ${modeName(expected)}. Refresh Crew and',
        'this file was checked for ${modeName(expected)}. Reload Crew and'
      ),
    },
    'app-sentences',
    /quotes "Your connection is now Private; this file was checked for Public\. Refresh Crew and drop the file again\.", which is not how/
  );
  // A family whose strings are gone from the code fails instead of passing.
  assertCaught(
    { [DIALOGS_COPY]: (text) => text.replaceAll('Use the key file’s', 'Give the key file’s') },
    'app-sentences',
    /found no the Identity file note/
  );
});

test('data-paths: every folder Crew writes on a member computer has a row (T3-DOC-2)', () => {
  const ADMINISTRATION = 'docs/crew/administration.md';
  // The table as the live check found it: no task records, receipts or task folder.
  assertCaught(
    {
      [ADMINISTRATION]: (text) =>
        text
          .split('\n')
          .filter(
            (line) =>
              !line.startsWith('| `~/.local/state/biorouter/crew/` |') &&
              !line.startsWith('| `~/.local/share/biorouter/crew/tasks/` |')
          )
          .join('\n'),
    },
    'data-paths',
    /has no row for ~\/\.local\/state\/biorouter\/crew\/runs\.json/
  );
  // A folder the code starts writing later needs a row too.
  assertCaught(
    {
      'crates/biorouter-server/src/routes/crew.rs': swap(
        '.join("crew")\n        .join("tasks")',
        '.join("crew-tasks")'
      ),
    },
    'data-paths',
    /has no row for ~\/\.local\/share\/biorouter\/crew-tasks/
  );
  // A reader that finds no paths fails instead of passing.
  assertCaught(
    Object.fromEntries(
      [
        'crates/biorouter/src/crew',
        'crates/biorouter-server/src/crew',
        'crates/biorouter-server/src/routes',
      ].flatMap((dir) =>
        real
          .list(dir)
          .filter((name) => name.endsWith('.rs'))
          .map((name) => [`${dir}/${name}`, (text) => text.replaceAll('Paths::', 'Dirs::')])
      )
    ),
    'data-paths',
    /update this reader/
  );
});

test('work-folder: the manual says what a command in the work folder cannot do (T3-DOC-5)', () => {
  // The agents page as the live check found it: no word of the sandbox.
  assertCaught(
    {
      [AGENTS]: (text) =>
        text.replace(/## Work in the remote work folder\n[\s\S]*?(?=## Refusals)/, ''),
    },
    'work-folder',
    /agents-and-chat-access\.md does not say that a work-folder command has no network/
  );
  assertCaught(
    {
      'docs/crew/administration.md': (text) =>
        text
          .split('\n')
          .filter((line) => !line.startsWith('| What one agent command can reach |'))
          .join('\n'),
    },
    'work-folder',
    /administration\.md does not say that a work-folder command has no other processes/
  );
  // The sandbox lets a command open a socket: the manual's "no network" is then wrong.
  assertCaught(
    {
      'crates/biorouter-crew/src/remote.rs': swap(
        '        libc::SYS_execve,\n',
        '        libc::SYS_execve,\n        libc::SYS_socket,\n'
      ),
    },
    'work-folder',
    /says a work-folder command has no network, but .* now allows it/
  );
  // A reader that finds no allow list fails instead of passing.
  assertCaught(
    {
      'crates/biorouter-crew/src/remote.rs': (text) =>
        text.replace('\nfn confine(', '\nfn confine_command('),
    },
    'work-folder',
    /update this reader/
  );
});

test('rustLiterals reads a \\u{…} escape as its character', () => {
  assert.deepEqual(rustLiterals('fn f() { g("has \\u{201c}{}\\u{201d} here"); }'), [
    'has “{}” here',
  ]);
});

test('app-sentences: the joining page quotes the damaged-invitation note and both join conflicts whole (T3-DOC-4)', () => {
  const JOINING = 'docs/crew/joining-a-workspace.md';
  // The rows as the live check found them: a quote cut short, and one answer for two sentences.
  assertCaught(
    {
      [JOINING]: (text) =>
        text.replace(
          /^\| "This invitation is incomplete or was changed\..*$/m,
          '| "This invitation is incomplete…" | Email or chat may have wrapped the `brcrew1:` line or cut it short. Paste the whole message again, or ask your host to send it as an attachment. |'
        ),
    },
    'app-sentences',
    /joining-a-workspace\.md does not quote the damaged-invitation note .* to the end of a sentence/
  );
  assertCaught(
    {
      [JOINING]: (text) =>
        text
          .replace(
            /^\| "This computer already has “chen-lab” for this workspace, signing in.*\n/m,
            ''
          )
          .replace(
            /^\| "This computer already has “chen-lab” for this workspace\. Change it.*$/m,
            '| "This computer already has “chen-lab” for this workspace…" | Choose **Open chen-lab**. |'
          ),
    },
    'app-sentences',
    /does not quote the daemon's join conflicts .* "This computer already has “\{\}” for this workspace, signing in as another account/
  );
  // The app rewords the note and the manual keeps the old words.
  assertCaught(
    {
      'ui/desktop/src/components/crew/onboarding/copy.ts': swap(
        "'This invitation is incomplete or was changed. Paste",
        "'This invitation is incomplete or damaged. Paste"
      ),
    },
    'app-sentences',
    /quotes "This invitation is incomplete or was changed\. Paste the whole message again, or ask your host to send it again\.", which is not how/
  );
});

test('saysTemplate never takes a name alone for the opening of a template that starts with one', () => {
  const template = "{}'s host key isn't in your ~/.ssh/known_hosts yet. Add it.";
  assert.ok(!saysTemplate('crew_ssh_host_key_unknown', template));
  assert.ok(saysTemplate("localhost's host key isn't in your ~/.ssh/known_hosts yet.", template));
  assert.ok(saysTemplate("localhost's host key isn't…", template));
});

test('command line: same-host rows, the codes the command line gives, and its new sentences (T3-DOC-3)', () => {
  // The SSH rows as the live check found them: a member on the server sent to IT.
  assertCaught(
    {
      [COMMAND_LINE]: (text) =>
        text
          .replace(/ For a login on this machine, the sentence is ``Couldn't sign in[^|]*\|/, ' |')
          .replace(
            / For a login on this machine, the sentence is ``localhost's host key[^|]*\|/,
            ' |'
          ),
    },
    'ssh-codes',
    /row for `crew_ssh_key_refused` does not quote what a login on this machine gets/
  );
  // A code the command line gives, missing from the page.
  assertCaught(
    {
      [COMMAND_LINE]: (text) =>
        text
          .split('\n')
          .filter((line) => !line.startsWith('| `crew_connection_required` |'))
          .join('\n'),
    },
    'refusal-codes',
    /never names the command line's code `crew_connection_required`/
  );
  // One the command line starts to give later needs a row too.
  assertCaught(
    {
      'crates/biorouter-cli/src/commands/crew/mod.rs': (text) =>
        text.replace(
          'const CONNECTION_REQUIRED: &str',
          'const SOMETHING_NEW: &str = "crew_something_new";\nconst CONNECTION_REQUIRED: &str'
        ),
    },
    'refusal-codes',
    /never names the command line's code `crew_something_new`/
  );
  // The 64 KB refusal, unquoted, and then reworded in the code.
  assertCaught(
    {
      [COMMAND_LINE]: swap(
        'A longer one is refused before anything is sent, with ``Messages can be up to 64 KB. Save the text to a file and share it with biorouter crew files upload.`` (exit',
        'A longer one is refused before anything is sent (exit'
      ),
    },
    'app-sentences',
    /does not quote the command line's the refusal of a message over 64 KB/
  );
  assertCaught(
    {
      'crates/biorouter-cli/src/commands/crew/output.rs': swap(
        'Messages can be up to 64 KB. Save the text to a file',
        'Messages can be up to 64 KB. Put the text in a file'
      ),
    },
    'app-sentences',
    /quotes "Messages can be up to 64 KB\. Save the text to a file and share it with biorouter crew files upload\.", which is not how/
  );
  // The join paragraph without --replace: the refusal the live check met, with no way out.
  assertCaught(
    {
      [COMMAND_LINE]: (text) =>
        text.replace(/\n\nAn invitation names one server account\.[^\n]*/, ''),
    },
    'app-sentences',
    /does not quote the command line's join conflicts .* "This computer already has \{name\} for this workspace, signing in as another account, and it has never connected\. Run it again with --replace/
  );
});

test('troubleshooting: a server that stopped saving, a restarted background service, hidden characters and a paused upload (T3-DOC-6)', () => {
  // The pages as the live check found them, one gap at a time.
  assertCaught(
    {
      [TROUBLE]: (text) =>
        text
          .replace(
            /\n## A server that stopped saving\n[\s\S]*?(?=\n## Messages and what to do)/,
            ''
          )
          .replace(/^\| "Connected" with "The workspace server has stopped saving.*\n/m, ''),
    },
    'app-sentences',
    /does not quote the stopped-saving sentence/
  );
  assertCaught(
    {
      [TROUBLE]: (text) =>
        text.replace(/^\| "Biorouter’s background service restarted, so Crew.*\n/m, ''),
    },
    'app-sentences',
    /does not quote the background-service note/
  );
  assertCaught(
    {
      [MESSAGES]: (text) =>
        text.replace(/^\| A name with an invisible or formatting character.*\n/m, ''),
    },
    'app-sentences',
    /does not quote the hidden-character refusal/
  );
  assertCaught(
    {
      [MESSAGES]: (text) =>
        text.replaceAll('"The workspace server couldn’t save it"', 'a full server'),
    },
    'pause-reasons',
    /does not list the pause reason "The workspace server couldn't save it"/
  );
  // The app rewords the bar and the manual keeps the old words.
  assertCaught(
    {
      'ui/desktop/src/components/crew/channel/copy.ts': swap(
        "serverStorage: 'The workspace server has stopped saving changes.",
        "serverStorage: 'The workspace server has stopped saving new changes."
      ),
      [CLI_OUTPUT_RS]: (text) =>
        text.replace(
          '"The workspace server has stopped saving changes.',
          '"The workspace server has stopped saving new changes.'
        ),
    },
    'app-sentences',
    /quotes "The workspace server has stopped saving changes\. Reading still works\.", which is not how/
  );
});

test('agents page: a chat grant includes the work folder, as the Chat access pane says (T3-DOC-5)', () => {
  // The /crew steps as the live check found them: no word of the folder the chat receives.
  assertCaught(
    {
      [AGENTS]: (text) =>
        text.replace(
          / If your connection has a \*\*Remote work folder\*\* and the model is not public, the chat gets the folder too, and the pane lists it: [^\n]*? is on\./,
          ''
        ),
    },
    'app-sentences',
    /does not quote the Chat access pane's work folder lines/
  );
  assertCaught(
    {
      [AGENTS]: (text) =>
        text.replace(
          / The switch in \*\*Connection settings…\*\* says so too: "Commands run with no network[^"]*"/,
          ''
        ),
    },
    'app-sentences',
    /does not quote the work folder switch's help/
  );
});

test('command line: Privacy settings says a connection under another institution is refused (T3-DOC-3)', () => {
  assertCaught(
    {
      [COMMAND_LINE]: (text) =>
        text.replace(
          /A Private connection's institution must be the workspace's[^`]*``[^`]*`` \([^)]*\)\. /,
          ''
        ),
    },
    'app-sentences',
    /command-line\.md does not quote the institution refusal/
  );
  // The daemon rewords it and the page keeps the old words.
  assertCaught(
    {
      'crates/biorouter/src/crew/institution.rs': swap(
        'belongs to {theirs}. Use {theirs} here.',
        'belongs to {theirs}. Choose {theirs} instead.'
      ),
      'ui/desktop/src/components/crew/dialogs/copy.ts': swap(
        'belongs to ${institution}. Use ${institution} here.',
        'belongs to ${institution}. Choose ${institution} instead.'
      ),
    },
    'app-sentences',
    /quotes "This connection is for stanford, but lab belongs to ucsf\. Use ucsf here\.", which is not how/
  );
});

test('allow-button: the Allow button is named as the pane labels it', () => {
  assertCaught(
    {
      [AGENTS]: swap(
        '5. Read what the pane lists under "“Plot review” will be able to" ("This chat will be able to" when the title is unknown), then choose **Allow**.',
        '5. Choose **Allow “Plot review” to read and post in #methods** (**Allow this conversation to read and post here** when the title is unknown).'
      ),
    },
    'allow-button',
    /agents-and-chat-access\.md names an Allow button that carries the chat/
  );
  assertCaught(
    {
      [LANDING]: swap(
        'then choose <strong>Allow</strong>. The pane shows',
        'Choose <strong>Allow “{chat title}” to read and post in #methods</strong>. The pane shows'
      ),
    },
    'allow-button',
    /landing\/docs\.html#crew names an Allow button/
  );
});

test('app-sentences: the agents page quotes the refusal to make a Crew chat public', () => {
  assertCaught(
    {
      [AGENTS]: (text) =>
        text.replace(
          / \*\*Make this chat public\*\* in the chat history is refused with "[^"]*"/,
          ''
        ),
    },
    'app-sentences',
    /does not quote the refusal to make a Crew chat public/
  );
});

test('reconnect-timing: the pages say how soon Crew tries again, and quote each reconnect sentence (T3-DOC-6)', () => {
  // The member row as the live check found it: "ask your host", with no time.
  assertCaught(
    {
      [TROUBLE]: (text) =>
        text
          .replace(
            /Otherwise, the screen says "The workspace server isn’t running\.[^|]*\|/,
            'Otherwise, ask your host to start Crew. Crew connects by itself once it runs. |'
          )
          .replace(/ A member reads "The workspace server isn’t running\.[^"]*"/, ''),
    },
    'app-sentences',
    /troubleshooting\.md does not quote the stopped-server sentence for a member/
  );
  // The old member sentence, back in a quote.
  assertCaught(
    {
      'docs/crew/joining-a-workspace.md': (text) =>
        text.replace(
          'Once Alice Chen starts Crew, this computer connects by itself within a few minutes, or you can connect now.',
          'Ask Alice Chen to start Crew.'
        ),
    },
    'app-sentences',
    /quotes "The workspace server isn’t running\. Ask Alice Chen to start Crew\.", which is not how/
  );
  // The timing, gone from the page, or changed in the keepalive.
  assertCaught(
    { [TROUBLE]: (text) => text.replaceAll('every 30 seconds', 'regularly') },
    'reconnect-timing',
    /does not say Crew tries again "every 30 seconds"/
  );
  assertCaught(
    {
      'crates/biorouter/src/crew/keepalive.rs': swap(
        'broker_down_every: Duration::from_secs(30),',
        'broker_down_every: Duration::from_secs(15),'
      ),
    },
    'reconnect-timing',
    /does not say Crew tries again "every 15 seconds"/
  );
  // The longer reconnect sentence the daemon now gives, unquoted.
  assertCaught(
    {
      [COMMAND_LINE]: (text) =>
        text.replace(/When the next try is further off, it says when, such as `[^`]*` /, ''),
    },
    'daemon-sentences',
    /command-line\.md does not quote the crew_reconnecting sentence .* in about \{when\}/
  );
});

test('source-line: the agents page quotes the source line for a work-folder file as the daemon writes it (T3-DOC-5)', () => {
  // The result table as the live check found it: no work-folder form.
  assertCaught(
    {
      [AGENTS]: (text) =>
        text
          .replace(/^\| A file in the remote work folder \|.*\n/m, '')
          .replace(/^\| Shared files and work-folder files \|.*\n/m, '')
          .replace(
            / or ``Source: `samples_result\.txt` from the remote work folder on hpc\.``/,
            ''
          ),
    },
    'source-line',
    /does not quote a source line naming a work-folder file/
  );
  // The daemon rewords the line and the page keeps the old words.
  assertCaught(
    {
      'ui/desktop/src/components/crew/daemonSourceLine.cases.json': (text) =>
        text.replaceAll('from the remote work folder on hpc', 'in the work folder on hpc'),
    },
    'source-line',
    /quotes "Source: `samples_result\.txt` from the remote work folder on hpc\.", which no case/
  );
});

test('the tree as committed passes every rule', () => {
  assert.deepEqual(checkCrewManual(real), []);
});
