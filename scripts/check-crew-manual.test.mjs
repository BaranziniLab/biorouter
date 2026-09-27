// The mutant suite for check-crew-manual.mjs. A checker that cannot fail is not
// a gate, so each rule is shown failing on the text it exists to refuse: the
// exact wording the 2026-09-27 Crew audit found (DOCS-1, DOCS-2, DOCS-5, DOCS-6,
// RENDERER-6), put back into an otherwise real tree. The last test is the gate
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

test('the tree as committed passes every rule', () => {
  assert.deepEqual(checkCrewManual(real), []);
});
