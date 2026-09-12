// Regression suite for build-registry.mjs.
//
//     node --test landing/scripts/build-registry.test.mjs
//
// The fixtures in scripts/fixtures/ were live but inert: they were run by hand
// once and never again, so nothing stopped a later edit from making a rule
// unreachable. This file is what makes them a gate.
//
// Three things every rejection test asserts, because each one has been the
// difference between a gate and a decoration:
//
//   * the exit status is EXACTLY 1, not merely non-zero. A ReferenceError, an
//     ENOENT on a renamed fixture and an unrecognised flag all exit non-zero,
//     and all three are what a half-finished implementation produces.
//   * stderr names the RULE that fired. Without this a crash reads as a pass.
//   * the file the run asked for does not exist afterwards. `--out /dev/null`
//     cannot tell "wrote nothing" from "wrote garbage and then failed".
//
// No npm dependency: node:test ships with the runtime, so this runs anywhere
// the generator does.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const LANDING = resolve(SCRIPTS, '..');
const REPO = resolve(LANDING, '..');
const GENERATOR = join(SCRIPTS, 'build-registry.mjs');
const FIXTURES = join(SCRIPTS, 'fixtures');
// The compiled-in security artifact, named up here because two tests read it
// and one of them runs before the PROTECTED map below is evaluated.
const PROTECTED_PRIVATE_SET = join(REPO, 'crates/biorouter/src/privacy/registry_private.rs');

let tmpSeq = 0;
const scratch = mkdtempSync(join(tmpdir(), 'registry-gen-'));
process.on('exit', () => rmSync(scratch, { recursive: true, force: true }));

/** Run the generator the way a person would, from the repo root. */
function run({ input, out, args = [], cwd = REPO, script = GENERATOR }) {
  const argv = [script];
  if (input !== undefined) argv.push('--input', input);
  if (out !== undefined) argv.push('--out', out);
  argv.push(...args);
  const r = spawnSync(process.execPath, argv, { cwd, encoding: 'utf8' });
  if (r.error) throw r.error;
  return { code: r.status, stdout: r.stdout, stderr: r.stderr, both: r.stdout + r.stderr };
}

/** Generator copies currently on disk, so a crash cannot leave one in the tree. */
const copies = new Set();
process.on('exit', () => {
  for (const c of copies) rmSync(c, { force: true });
});

/**
 * Run `body` against a copy of the generator whose `INSTITUTIONS` literal has
 * been replaced by `literal`.
 *
 * ⚠ **A copy rather than a flag, deliberately.** `INSTITUTIONS` decides what the
 * compiled-in Rust snapshot declares, so a `--institutions` override would be a
 * command-line way to change a security artifact — the exact shape this
 * generator's other guards exist to prevent. Rewriting the literal in a throwaway
 * copy exercises the rules without adding a knob anybody could reach in
 * production.
 *
 * ⚠ **The copy sits beside the original.** Every path the generator resolves —
 * the fixtures, the three protected artifacts, the lock — hangs off
 * `import.meta.url`, so a copy in /tmp would resolve none of them. Its output is
 * byte-identical to the original's for identical input: nothing rendered embeds
 * the running script's filename.
 */
function withInstitutions(literal, body) {
  const src = readFileSync(GENERATOR, 'utf8');
  const declaration = /^const INSTITUTIONS = .*$/m;
  assert.match(
    src,
    declaration,
    'the INSTITUTIONS literal must still be a single line for these tests to rewrite it'
  );
  const copy = join(SCRIPTS, `.build-registry.institutions-${tmpSeq++}.mjs`);
  copies.add(copy);
  try {
    writeFileSync(copy, src.replace(declaration, () => `const INSTITUTIONS = ${literal};`));
    return body(copy);
  } finally {
    rmSync(copy, { force: true });
    copies.delete(copy);
  }
}

const fixture = (name) => join(FIXTURES, `${name}.html`);
const outPath = () => join(scratch, `out-${tmpSeq++}.json`);
const rustPath = () => join(scratch, `private-${tmpSeq++}.rs`);

/**
 * A fixture the generator must refuse, the rule whose message proves the
 * refusal came from that rule rather than from a crash, and any extra flags the
 * run needs.
 *
 * The flags matter for exactly one rule. The closed private set is asserted on
 * every REAL run, but a fixture has to opt in with `--assert-private-set` —
 * otherwise every fixture here would have to name its private cards `cdwagent`
 * and `ucsfomopagent`, and the fixtures that exist to exercise *other* rules
 * (`suffixed-download`, `three-affiliations`, `private-no-affiliation`) could
 * not exist at all.
 */
const REJECTED = [
  ['invalid-privacy', /^registry: .*data-privacy must be "private" or "public"/m],
  ['private-no-name', /^registry: .*must declare data-extension-name/m],
  ['unknown-institution', /^registry: .*data-affiliation names "atlantis"/m],
  ['public-with-affiliation', /^registry: .*data-affiliation is meaningless/m],
  ['empty-affiliation', /^registry: .*data-affiliation is present but empty/m],

  // Attribute syntax HTML allows and the card-fragment substring search did
  // not. Each of these read as an un-annotated (public) card, which is the
  // fail-open direction.
  ['spaced-attribute', /^registry: .*must declare data-extension-name/m],
  ['single-quoted-attribute', /^registry: .*must declare data-extension-name/m],
  ['boolean-attribute', /^registry: .*data-privacy is present with no value/m],
  ['nested-metadata', /^registry: .*data-privacy.*must be declared on the card element itself/m],

  // The badge is the no-JS view's ONLY statement of the tier, and the attribute
  // is the generator's. Two sources for one fact drift; publishing while they
  // disagree ships a card that reads "Public" over a private connector.
  ['badge-contradicts-tier', /^registry: .*the privacy badge reads "Public" but the card declares data-privacy="private"/m],
  ['unbadged-card', /^registry: .*carries 0 \[data-privacy-badge\] chips, expected exactly one reading "Public"/m],

  // The same drift one axis over. The institution badge is the only place the
  // no-JS view states DR-26's third axis, and each of these three publishes a
  // card that says something the catalog does not.
  //
  //   undeclared  — a badge with no attribute behind it paints an institutional
  //                 constraint the daemon will never enforce. Absent means
  //                 unconstrained, so this is the fail-OPEN direction.
  //   contradicts — badge and attribute name different institutions, and only
  //                 the attribute reaches registry.json and the app.
  //   shows-the-id— the badge rendered from the slug. INSTITUTIONS carries a
  //                 display name precisely so DR-26's warning can NAME an
  //                 institution instead of shrugging a slug at the user.
  [
    'affiliation-badge-undeclared',
    /^registry: .*carries an institution badge \("ucsf"\) but the card declares no data-affiliation/m,
  ],
  [
    'affiliation-badge-contradicts-declaration',
    /^registry: .*badges \[stanford\] but the card declares data-affiliation="ucsf"/m,
  ],
  [
    'affiliation-badge-shows-the-id',
    /^registry: .*institution badge for "ucsf" reads "ucsf data", expected "UCSF data"/m,
  ],

  // A catalog of nothing is never a legitimate result — on a real run it
  // rewrites the compiled-in private set to empty.
  ['missing-section', /^registry: .*no element with id="extensions-section"/m],
  ['empty-section', /^registry: .*holds no ext-card/m],
  ['no-download-link', /^registry: .*no \.brxt download link/m],

  // The join key between the published catalog and the installed config entry.
  ['blank-extension-name', /^registry: .*data-extension-name.*only whitespace/m],
  ['punctuated-extension-name', /^registry: .*data-extension-name.*private_agent/m],
  ['duplicate-extension-name', /^registry: .*both declare data-extension-name "twinagent"/m],

  // The catalog's own id, which is a DIFFERENT field from the join key above.
  // A version in it means the id moves every release, so `install_extension`
  // only ever resolves the spelling that was current when the model last read
  // the catalog — the SPOKEAgent defect, caught at the page instead of three
  // layers away in a Rust snapshot test that can name no card.
  ['versioned-id', /^registry: .*ends with this card's version "v1\.2\.3"/m],
  ['punctuated-registry-id', /^registry: .*data-registry-id "suffix agent!" is not a bare key/m],

  // The private set is a CLOSED LIST of two, both UCSF — not a property a card
  // grants itself by writing data-privacy="private". Checked in both directions
  // and on the affiliation, because all three changes are ones nobody should be
  // able to make as a side effect of editing a card.
  [
    'private-set-third-entry',
    /^registry: .*"thirdprivateagent" is not in the closed private set/m,
    ['--assert-private-set'],
  ],
  [
    'private-set-missing-entry',
    /^registry: the closed private set names "ucsfomopagent", but no card/m,
    ['--assert-private-set'],
  ],
  [
    'private-set-foreign-affiliation',
    /^registry: .*"cdwagent" is private with affiliation \[stanford\], but the closed private set records \[ucsf\]/m,
    ['--assert-private-set'],
  ],
];

for (const [name, rule, args = []] of REJECTED) {
  test(`${name} is refused, by its own rule, writing nothing`, () => {
    const out = outPath();
    const r = run({ input: fixture(name), out, args });
    assert.equal(r.code, 1, `expected exit 1, got ${r.code}\n${r.both}`);
    assert.match(r.stderr, rule);
    assert.equal(
      existsSync(out),
      false,
      'a rejected run must leave the file it was asked to write absent'
    );
  });
}

test('the happy fixture parses, so the refusals above are not "zero cards"', () => {
  const out = outPath();
  const r = run({ input: fixture('happy'), out });
  assert.equal(r.code, 0, r.both);
  assert.match(r.stdout, /registry\.json written: 2 extensions, 0 skills/);

  const reg = JSON.parse(readFileSync(out, 'utf8'));
  assert.equal(reg.version, 2);
  assert.deepEqual(reg.institutions, { ucsf: 'UCSF' });
  assert.equal(reg.extensions.length, 2);

  // Positive assertions on the emitted document, not just on the exit status.
  // Every rule below has a wrong implementation that still exits 0.
  const pub = reg.extensions.find((e) => e.id === 'publicfixtureagent');
  assert.equal(pub.privacy, 'public', 'an un-annotated card is public by construction');
  assert.equal('extension_name' in pub, false);
  assert.equal('affiliation' in pub, false, 'absent affiliation must stay absent, not become []');

  const priv = reg.extensions.find((e) => e.id === 'privatefixtureagent');
  assert.equal(priv.privacy, 'private');
  assert.equal(priv.extension_name, 'privatefixtureagent', 'emitted in name_to_key form');
  assert.deepEqual(priv.affiliation, ['ucsf']);

  // The badge shares the .ext-tags row with the subject tags, and this row is
  // the tag source. Exact lists, not `includes` — a filter that dropped one tag
  // too many, or published "Private" as a topic, satisfies every other
  // assertion here.
  assert.deepEqual(pub.tags, ['MCP'], 'the Public badge must not be published as a subject tag');
  // Exact for the institution badge too: it shares this row, so an unfiltered
  // one publishes "UCSF data" as a TOPIC of the extension — and the shelf then
  // re-renders it as a third chip beside the badge it already prepends. The
  // plain "UCSF" subject tag beside it must survive, which is what separates
  // "skip the badge" from "delete anything that mentions the institution".
  assert.deepEqual(
    priv.tags,
    ['UCSF', 'MCP'],
    'neither the Private badge nor the institution badge may be published as a subject tag'
  );
});

test('a subject tag is not deleted for being spelled like the badge', () => {
  // The first version of the skip filter matched the chip's private/public
  // CLASS. `allTags` also harvests the skills shelf, where "Public" is an
  // ordinary topic, so that filter silently deleted legitimate tags from the
  // published catalog — with no fixture and no assertion on emitted tags, in a
  // generator whose whole job is to be trusted about what it publishes.
  const out = outPath();
  const r = run({ input: fixture('subject-tag-named-public'), out });
  assert.equal(r.code, 0, r.both);
  const reg = JSON.parse(readFileSync(out, 'utf8'));
  assert.deepEqual(reg.extensions[0].tags, ['MCP', 'Public Data', 'Public']);
  assert.equal(reg.extensions[0].privacy, 'public');
});

test('a public card is not refused for describing patients honestly', () => {
  // The false failure the deleted description-keyword heuristic produced. It
  // refused any un-annotated card whose description matched "patient", "ehr",
  // "phi", "clinical record", "medical record" or "de-identified clinical" —
  // inferring a security property from marketing prose. A public literature or
  // imaging tool that says a true thing about patients is not a private
  // extension, and refusing it teaches authors to write around a word list.
  //
  // Without this test, reinstating that heuristic breaks nothing.
  const out = outPath();
  const r = run({ input: fixture('public-description-mentions-patient'), out });
  assert.equal(r.code, 0, r.both);
  const reg = JSON.parse(readFileSync(out, 'utf8'));
  assert.equal(reg.extensions.length, 1);
  assert.equal(reg.extensions[0].privacy, 'public');
  assert.match(reg.extensions[0].description, /patient population/);
});

test('the closed private set is exactly the two UCSF extensions the operator named', () => {
  // The assertion inside the generator is checked against fixtures above; this
  // is the other half — that the list it holds is still the right list, read
  // off the committed security artifact rather than off the generator's own
  // constant. A run that widened both together would pass every fixture here.
  const rust = readFileSync(PROTECTED_PRIVATE_SET, 'utf8');
  assert.match(rust, /PRIVATE_EXTENSIONS: &\[&str\] =\s*&\["cdwagent", "ucsfomopagent"\];/);
  assert.match(
    rust,
    /EXTENSION_AFFILIATIONS: &\[\(&str, &\[&str\]\)\] =\s*&\[\("cdwagent", &\["ucsf"\]\), \("ucsfomopagent", &\["ucsf"\]\)\];/
  );
});

test('--assert-private-set is refused on a real run, which always asserts it', () => {
  // The flag is a fixture affordance, exactly like --emit-rust. Accepting it on
  // a real run would suggest the real run needs asking. Wrapped in
  // protectingArtifacts because a REGRESSION here is a real run: the guard is
  // what stops this test from rewriting the three published files.
  protectingArtifacts(() => {
    const r = run({ args: ['--assert-private-set'] });
    assert.equal(r.code, 1, r.both);
    assert.match(r.stderr, /--assert-private-set is for fixture runs/);
  });
});

test('a REAL run applies the closed private set, not only a run that asks for it', () => {
  // The half the flag cannot cover, and the half that matters.
  //
  // `assertClosedPrivateSet` runs under `IS_REAL_RUN || ASSERT_PRIVATE_SET`.
  // Delete the `IS_REAL_RUN ||` and every other gate in this file stays green:
  // the three closed-set fixtures pass the flag explicitly, `--check` on a
  // conforming page has nothing to report, and the test above asserts only that
  // the flag is REFUSED on a real run — never that the real run does the work.
  // Yet the real run is the only one that writes registry_private.rs, and
  // `--check` is the only invocation CI and `just check-everything` make. So the
  // one path that decides what the daemon withholds would have had no coverage
  // at all, and the gate could stop firing without a single test going red.
  //
  // Mutating the real catalog is the only way to exercise it, because "real run"
  // is defined as reading the real baam.html. `--check` is the safe mode for
  // that: it writes nothing, and here it exits before even the comparison
  // because the violation drain precedes it. The page is restored in a `finally`
  // regardless, and asserted restored afterwards — a test that leaves the
  // published catalog mutated is worse than no test.
  //
  // The mutation renames one private card's join key, which trips BOTH loops at
  // once: the renamed key is not in the closed list, and the listed key now has
  // no private card. The second is the dangerous direction — an extension that
  // keeps working and quietly stops being withheld from public sessions.
  const page = join(LANDING, 'baam.html');
  const original = readFileSync(page, 'utf8');
  protectingArtifacts(() => {
    try {
      const mutated = original.replace(
        'data-extension-name="CDWAgent"',
        'data-extension-name="NotListedAgent"'
      );
      assert.notEqual(mutated, original, 'the mutation must actually apply, or this proves nothing');
      writeFileSync(page, mutated);

      const r = run({ args: ['--check'] });
      assert.equal(r.code, 1, `a real run must refuse the widened set\n${r.both}`);
      assert.match(r.stderr, /"notlistedagent" is not in the closed private set/);
      assert.match(
        r.stderr,
        /the closed private set names "cdwagent", but no card in this catalog declares/
      );
      // The advice is part of the rule, not decoration: a gate that says only
      // "unexpected" is one people delete.
      assert.match(r.stderr, /EXPECTED_PRIVATE_EXTENSIONS in landing\/scripts\/build-registry\.mjs/);
    } finally {
      writeFileSync(page, original);
    }
  });
  assert.equal(readFileSync(page, 'utf8'), original, 'the real catalog must be restored');

  // And the restored page still passes, so the failure above was the mutation
  // and not something this test left behind.
  const clean = run({ args: ['--check'] });
  assert.equal(clean.code, 0, clean.both);
});

test('an institution nobody names is refused, and the refusal says how to keep it', () => {
  // Task 56 Step 3. `INSTITUTIONS` used to be pinned by cardinality on the Rust
  // side — "this build knows exactly one institution". That guard says nothing
  // about whether the one institution is the RIGHT one, and its only possible
  // repair is deletion: the day a second institution is genuinely added, the
  // assertion is simply wrong and the person adding it deletes it. Referential
  // integrity scales instead, and catches the error actually made — an orphan,
  // which is either a leftover from a deleted connector or the other half of a
  // typo in a card.
  //
  // Driven by mutating the real catalog, for the reason the closed-private-set
  // test beside it is: "used" is defined against the real cards, so a fixture
  // cannot exercise the real run. `--check` writes nothing, the page is restored
  // in a `finally`, and the restoration is asserted afterwards.
  const page = join(LANDING, 'baam.html');
  const original = readFileSync(page, 'utf8');
  protectingArtifacts(() => {
    try {
      // Strip the affiliation off every private card, which leaves `ucsf`
      // declared in INSTITUTIONS and named by nothing.
      const mutated = original.replaceAll(' data-affiliation="ucsf"', '');
      assert.notEqual(mutated, original, 'the mutation must actually apply, or this proves nothing');
      writeFileSync(page, mutated);

      const r = run({ args: ['--check'] });
      assert.equal(r.code, 1, `an orphaned institution must be refused\n${r.both}`);
      assert.match(r.stderr, /institution "ucsf" is declared but no card names it/);
      // The repair is part of the rule. A gate that says only "unexpected" is
      // one people delete rather than update — which is exactly how the count
      // this replaced would have ended.
      assert.match(r.stderr, /retainedUnused/);
    } finally {
      writeFileSync(page, original);
    }
  });
  assert.equal(readFileSync(page, 'utf8'), original, 'the real catalog must be restored');

  const clean = run({ args: ['--check'] });
  assert.equal(clean.code, 0, clean.both);
});

// ---- The other three institution rules ------------------------------------
// The orphan rule above was the only one of the four with a test. The three
// below are properties of the `INSTITUTIONS` literal alone — no catalog is
// involved — so each one is a hand-written row going wrong in a way that
// publishes something wrong rather than failing loudly:
//
//   * no display name  → the warning DR-26 requires to NAME an institution
//                        renders the raw slug at the user.
//   * un-normalised id → the Rust side reduces an id with name_to_key, so the
//                        row can never be matched by anything.
//   * duplicate id     → the later row silently wins in INSTITUTION_NAMES.
//
// They run on EVERY invocation, fixture included, which is what lets a fixture
// run exercise them at all. Under the previous shape they sat behind the
// real-run gate beside the orphan rule and no fixture could reach them.
const MALFORMED_INSTITUTIONS = [
  [
    'an institution with no display name',
    "[{ id: 'ucsf' }]",
    /institution "ucsf" has no display name/,
  ],
  [
    'an institution id the Rust side could never match',
    "[{ id: 'UCSF', name: 'UCSF' }]",
    /institution "UCSF" is not normalised/,
  ],
  [
    'the same institution declared twice',
    "[{ id: 'ucsf', name: 'UCSF' }, { id: 'ucsf', name: 'UC San Francisco' }]",
    /institution "ucsf" is declared twice/,
  ],
];

for (const [what, literal, rule] of MALFORMED_INSTITUTIONS) {
  test(`${what} is refused, by its own rule, writing nothing`, () => {
    const out = outPath();
    const r = withInstitutions(literal, (script) =>
      run({ input: fixture('happy'), out, script })
    );
    assert.equal(r.code, 1, `expected exit 1, got ${r.code}\n${r.both}`);
    assert.match(r.stderr, rule);
    assert.equal(
      existsSync(out),
      false,
      'a rejected run must leave the file it was asked to write absent'
    );
  });
}

test('the rewritten generator is otherwise the generator, so the refusals above mean something', () => {
  // Non-vacuity for the whole mechanism. A copy that failed for an unrelated
  // reason — a botched rewrite, a path that no longer resolves — would make
  // every test above pass while testing nothing. Rewriting the literal to what
  // it already says must leave a run that is clean to the byte, which is also
  // the strongest available statement that the copy renders identical output.
  withInstitutions("[{ id: 'ucsf', name: 'UCSF' }]", (script) => {
    const r = run({ args: ['--check'], script });
    assert.equal(r.code, 0, r.both);
    assert.match(r.stdout, /all three outputs are current/);
  });
});

test('an institution kept with a retainedUnused reason is not an orphan', () => {
  // The escape hatch, and the branch nothing exercised: no institution in the
  // real map carries the field, so it was dead code that could stop working
  // with nothing going red — leaving the orphan rule with no survivable answer
  // and turning "delete the row" into its only repair, which is how the
  // cardinality gate this replaced was going to end.
  //
  // Both runs exit 1, because an extra institution makes the regenerated
  // catalog differ from the committed one. The exit status is therefore not the
  // signal; the RULE is, and the rule must fire in the first run and stay
  // silent in the second.
  const orphanRule = /institution "stanford" is declared but no card names it/;
  const declared = (extra) =>
    `[{ id: 'ucsf', name: 'UCSF' }, { id: 'stanford', name: 'Stanford'${extra} }]`;

  protectingArtifacts(() => {
    const bare = withInstitutions(declared(''), (script) => run({ args: ['--check'], script }));
    assert.equal(bare.code, 1, bare.both);
    assert.match(bare.stderr, orphanRule);

    const kept = withInstitutions(
      declared(", retainedUnused: 'being onboarded; its cards land next release'"),
      (script) => run({ args: ['--check'], script })
    );
    assert.doesNotMatch(
      kept.stderr,
      /is declared but no card names it/,
      'a reason on the row is what makes an unused institution declarable rather than a failure'
    );
    // ...and it got past the violation drain to the comparison, so the silence
    // above is the rule not firing rather than the run dying earlier.
    assert.match(kept.stderr, /registry\.json/);
  });
});

test('attribute order does not decide whether a card exists', () => {
  // A card that reads perfectly in a browser must not be invisible to the
  // generator: an invisible private card is an empty compiled-in private set.
  const out = outPath();
  const r = run({ input: fixture('reordered-attributes'), out });
  assert.equal(r.code, 0, r.both);
  const reg = JSON.parse(readFileSync(out, 'utf8'));
  assert.equal(reg.extensions.length, 1, 'the card must be found with class last');
  assert.equal(reg.extensions[0].privacy, 'private');
  assert.equal(reg.extensions[0].extension_name, 'reorderedagent');
});

test('the compiled private set is keyed on the name, not on the download filename', () => {
  // The case the extension_name field exists for. Every private entry in the
  // real catalog happens to have id == extension_name, so a generator that
  // emitted the id into the Rust const passes every other test here.
  //
  // The fixture's id and name are deliberately UNRELATED strings
  // (`suffixcatalog` vs `suffixagent`) rather than a versioned pair. A shared
  // prefix would let a generator that truncated the id land on the right answer
  // by accident, which is the one outcome this test must not accept.
  const out = outPath();
  const rs = rustPath();
  const r = run({ input: fixture('suffixed-download'), out, args: ['--emit-rust', rs] });
  assert.equal(r.code, 0, r.both);

  const reg = JSON.parse(readFileSync(out, 'utf8'));
  assert.equal(reg.extensions[0].id, 'suffixcatalog');
  assert.equal(reg.extensions[0].extension_name, 'suffixagent');

  const rust = readFileSync(rs, 'utf8');
  assert.match(rust, /PRIVATE_EXTENSIONS: &\[&str\] = &\["suffixagent"\];/);
  assert.equal(
    rust.includes('1.2.3'),
    false,
    'the download-derived id must not reach the compiled-in private set'
  );
});

test('a private extension with no affiliation is unconstrained, not rejected', () => {
  // happy.html's only private card is affiliated, so without this a generator
  // that REQUIRED affiliation on every private entry passes the whole suite.
  const out = outPath();
  const rs = rustPath();
  const r = run({ input: fixture('private-no-affiliation'), out, args: ['--emit-rust', rs] });
  assert.equal(r.code, 0, r.both);

  const reg = JSON.parse(readFileSync(out, 'utf8'));
  assert.equal(reg.extensions[0].privacy, 'private');
  assert.equal(
    'affiliation' in reg.extensions[0],
    false,
    'absent means unconstrained and must stay absent, never [] '
  );
  const rust = readFileSync(rs, 'utf8');
  assert.match(rust, /&\["unaffiliatedagent"\];/);
  // ...and NO row in the affiliation table. A row with an empty list would be
  // read by `affiliation_for` as "permits nothing", turning the correct default
  // (unconstrained) into a permanent cross-institutional warning.
  assert.match(rust, /EXTENSION_AFFILIATIONS: &\[\(&str, &\[&str\]\)\] = &\[\];/);
});

test('the compiled-in snapshot carries affiliation and the institution names', () => {
  // DR-26 / Task 47. There is no network path to the registry from Rust, so the
  // compiled snapshot IS the registry there: an affiliation the generator does
  // not emit here is an affiliation the daemon and the CLI can never enforce.
  // The institution map has to come too, because DR-26 requires the warning to
  // NAME both institutions, and the display name lives only in this map.
  const out = outPath();
  const rs = rustPath();
  const r = run({ input: fixture('happy'), out, args: ['--emit-rust', rs] });
  assert.equal(r.code, 0, r.both);

  const rust = readFileSync(rs, 'utf8');
  assert.match(rust, /PRIVATE_EXTENSIONS: &\[&str\] = &\["privatefixtureagent"\];/);
  assert.match(
    rust,
    /EXTENSION_AFFILIATIONS: &\[\(&str, &\[&str\]\)\] =\s*&\[\("privatefixtureagent", &\["ucsf"\]\)\];/
  );
  assert.match(rust, /INSTITUTIONS: &\[\(&str, &str\)\] = &\[\("ucsf", "UCSF"\)\];/);

  // Keyed on the extension NAME, exactly like the private set beside it — the
  // download-derived id is a different string (`suffixed-download` proves it),
  // and an affiliation keyed on the wrong one silently never matches.
  assert.equal(
    rust.includes('"publicfixtureagent"'),
    false,
    'a public extension has no affiliation to declare and must not appear'
  );
});

/**
 * The repo's own rustfmt, or whatever is on PATH — `null` when neither answers.
 *
 * Not a hard dependency: this suite runs in the frontend CI job, where the Rust
 * toolchain is not installed. The assertion it guards is the *corroborating*
 * one; the golden shape beside it always runs.
 */
function findRustfmt() {
  for (const candidate of [join(REPO, 'bin', 'rustfmt'), 'rustfmt']) {
    const probe = spawnSync(candidate, ['--version'], { encoding: 'utf8' });
    if (!probe.error && probe.status === 0) return candidate;
  }
  return null;
}

test('a third affiliated extension wraps the way rustfmt wraps, not the way 100 columns would', () => {
  // The bug this exists to prevent, measured: rustfmt wraps a slice literal by
  // `array_width` (60 columns BETWEEN the brackets), not by the 100-column
  // `max_width`. Two affiliated rows fit either way, so every fixture here
  // passed while the generator emitted, for three rows, the hanging form
  //
  //     pub const EXTENSION_AFFILIATIONS: &[(&str, &[&str])] =
  //         &[("alphawideagent", &["ucsf"]), ("betawideagent", ...)];
  //
  // which `cargo fmt --check` rejects and `--check` below demands — two gates
  // in `just check-everything` with no state satisfying both.
  const out = outPath();
  const rs = rustPath();
  const r = run({ input: fixture('three-affiliations'), out, args: ['--emit-rust', rs] });
  assert.equal(r.code, 0, r.both);

  const rust = readFileSync(rs, 'utf8');
  assert.ok(
    rust.includes(
      'pub const EXTENSION_AFFILIATIONS: &[(&str, &[&str])] = &[\n' +
        '    ("alphawideagent", &["ucsf"]),\n' +
        '    ("betawideagent", &["ucsf"]),\n' +
        '    ("gammawideagent", &["ucsf"]),\n' +
        '];'
    ),
    `three rows must go one per line, not onto one indented line:\n${rust}`
  );

  // ...and that is not merely this generator agreeing with itself. Strip the
  // `#[rustfmt::skip]` attributes — which exist so a miss here can never wedge
  // the build — and rustfmt must leave the result alone.
  const rustfmt = findRustfmt();
  if (rustfmt === null) return;
  const unskipped = join(scratch, `unskipped-${tmpSeq++}.rs`);
  writeFileSync(
    unskipped,
    rust
      .split('\n')
      .filter((line) => line.trim() !== '#[rustfmt::skip]')
      .join('\n')
  );
  const fmt = spawnSync(rustfmt, ['--edition', '2021', '--check', unskipped], {
    encoding: 'utf8',
  });
  assert.equal(
    fmt.status,
    0,
    `rustfmt would rewrite the generated file:\n${fmt.stdout}${fmt.stderr}`
  );
});

test('the emitted consts are exempt from rustfmt, so the two gates cannot deadlock', () => {
  // The structural half of the fix above. Predicting rustfmt is best-effort —
  // its handling of a single very wide element follows no rule this generator
  // implements — so rustfmt is removed as a second authority over the layout of
  // a file whose own header forbids hand-editing it.
  const out = outPath();
  const rs = rustPath();
  const r = run({ input: fixture('three-affiliations'), out, args: ['--emit-rust', rs] });
  assert.equal(r.code, 0, r.both);

  const rust = readFileSync(rs, 'utf8');
  assert.equal(
    (rust.match(/^#\[rustfmt::skip\]$/gm) ?? []).length,
    3,
    `all three consts must carry the attribute:\n${rust}`
  );
});

test('--input without --out is refused rather than defaulted', () => {
  const r = run({ input: fixture('happy') });
  assert.equal(r.code, 1, r.both);
  assert.match(r.stderr, /--input requires an --out/);
});

// ---- The three published artifacts are unwritable by a fixture run ---------
// The guard compared raw path STRINGS, so any second spelling of a protected
// file slipped past it while resolving to the same inode. Each test below
// snapshots the real file and restores it in a `finally`, so a regression makes
// the test red without also rewriting the tracked catalog.
const PROTECTED = {
  'the published registry': join(LANDING, 'registry.json'),
  'the desktop fallback snapshot': join(
    REPO,
    'ui/desktop/src/components/baam/registry.fallback.json'
  ),
  'the compiled-in private set': PROTECTED_PRIVATE_SET,
};

/** Run `body`, and put every protected artifact back byte-for-byte afterwards. */
function protectingArtifacts(body) {
  const before = Object.values(PROTECTED).map((p) => [p, readFileSync(p)]);
  try {
    body();
  } finally {
    for (const [p, bytes] of before) writeFileSync(p, bytes);
  }
}

test('a relative --out that aliases the real registry is refused', () => {
  protectingArtifacts(() => {
    const real = PROTECTED['the published registry'];
    const before = readFileSync(real);
    // Textually different from the absolute default, same file.
    const r = run({ input: fixture('happy'), out: 'landing/registry.json', cwd: REPO });
    assert.equal(r.code, 1, r.both);
    assert.match(r.stderr, /--input requires an --out/);
    assert.deepEqual(readFileSync(real), before, 'the real catalog must not be rewritten');
  });
});

test('a symlink to the real registry is refused', () => {
  protectingArtifacts(() => {
    const real = PROTECTED['the published registry'];
    const before = readFileSync(real);
    const alias = join(scratch, `alias-${tmpSeq++}.json`);
    symlinkSync(real, alias);
    const r = run({ input: fixture('happy'), out: alias });
    assert.equal(r.code, 1, r.both);
    assert.match(r.stderr, /--input requires an --out/);
    assert.deepEqual(readFileSync(real), before, 'the real catalog must not be rewritten');
  });
});

for (const [what, path] of Object.entries(PROTECTED)) {
  if (path === PROTECTED['the published registry']) continue;
  test(`--out pointing at ${what} is refused`, () => {
    protectingArtifacts(() => {
      const before = readFileSync(path);
      const r = run({ input: fixture('happy'), out: path });
      assert.equal(r.code, 1, r.both);
      assert.deepEqual(readFileSync(path), before);
    });
  });
}

test('--emit-rust pointing at the compiled-in private set is refused', () => {
  protectingArtifacts(() => {
    const rs = PROTECTED['the compiled-in private set'];
    const before = readFileSync(rs);
    const r = run({ input: fixture('happy'), out: outPath(), args: ['--emit-rust', rs] });
    assert.equal(r.code, 1, r.both);
    assert.deepEqual(readFileSync(rs), before);
  });
});

test('a flag with no value is refused by name, not by a stack trace', () => {
  const r = run({ input: fixture('happy'), args: ['--out'] });
  assert.equal(r.code, 1, r.both);
  assert.match(r.stderr, /^registry: --out needs a value/m);
});

// ---- Drift between the three outputs is detectable ------------------------
// They are written by one command from one source, which is not the same as
// being in sync: an interrupted run, a hand edit or a rebase that took one side
// of a conflict all leave a published catalog the app does not enforce. Nothing
// could tell before `--check`.
test('--check passes on the committed tree', () => {
  const r = run({ args: ['--check'] });
  assert.equal(r.code, 0, r.both);
  assert.match(r.stdout, /all three outputs are current/);
});

for (const [what, path] of Object.entries(PROTECTED)) {
  test(`--check notices ${what} drifting`, () => {
    protectingArtifacts(() => {
      writeFileSync(path, readFileSync(path, 'utf8') + '\n// drifted\n');
      const r = run({ args: ['--check'] });
      assert.equal(r.code, 1, r.both);
      assert.match(r.stderr, new RegExp(path.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
    });
  });
}

test('--check refuses to pretend a fixture is the real input', () => {
  const r = run({ input: fixture('happy'), out: outPath(), args: ['--check'] });
  assert.equal(r.code, 1, r.both);
  assert.match(r.stderr, /--check/);
});

test('a held generation lock is refused by name', () => {
  // Two generators interleaving is how the three outputs end up describing
  // different worlds. The message has to name the file, because a crash is the
  // one way a stale lock can outlive its run.
  const lock = join(LANDING, '.registry-build.lock');
  writeFileSync(lock, 'held by the test suite\n');
  try {
    const r = run({ args: [] });
    assert.equal(r.code, 1, r.both);
    assert.match(r.stderr, /\.registry-build\.lock/);
  } finally {
    rmSync(lock, { force: true });
  }
});

// ---- The featured strip repeats grid cards -------------------------------
// Driven by mutating the real catalog, for the reason the closed-private-set and
// orphaned-institution tests above are: the fixtures carry extension cards and no
// skills shelf, so the rule is only reachable on a real run. `--check` writes
// nothing and exits on the violation drain before any comparison; the page is
// restored in a `finally` and the restoration asserted afterwards.

const FEATURED_PAGE = join(LANDING, 'baam.html');

/**
 * `original` with `from` → `to`, applied inside the `#skills-featured` strip ONLY.
 *
 * Every field in that strip now equals its grid twin's — that is the whole point
 * of the rule — so a whole-page `replace` would edit whichever copy came first
 * and prove nothing about the other. Windowing on the strip is what makes
 * "mutated the featured card" a true statement rather than a hopeful one.
 */
function mutateFeatured(original, from, to) {
  const start = original.indexOf('id="skills-featured"');
  const end = original.indexOf('id="core-skill-grid"');
  assert.ok(start !== -1 && end > start, 'the featured strip and the core grid must both be found');
  const strip = original.slice(start, end);
  assert.equal(
    strip.split(from).length - 1,
    1,
    `the mutation anchor must be unique in the strip: ${from}`
  );
  const mutated = original.slice(0, start) + strip.replace(from, to) + original.slice(end);
  assert.notEqual(mutated, original, 'the mutation must actually apply, or this proves nothing');
  return mutated;
}

/** Run `--check` over a mutated page, restoring the page whatever happens. */
function checkWithFeatured(mutate) {
  const original = readFileSync(FEATURED_PAGE, 'utf8');
  let r;
  protectingArtifacts(() => {
    try {
      writeFileSync(FEATURED_PAGE, mutate(original));
      r = run({ args: ['--check'] });
    } finally {
      writeFileSync(FEATURED_PAGE, original);
    }
  });
  assert.equal(readFileSync(FEATURED_PAGE, 'utf8'), original, 'the real catalog must be restored');
  return r;
}

test('the committed featured strip says exactly what its grid twins say', () => {
  // The green half: the four mutations below all assert a REFUSAL, and a refusal
  // means nothing if the unmutated page was already being refused for some
  // unrelated reason.
  //
  // Stated plainly, because it is the kind of assertion that reads stronger than
  // it is: this test is also green on a tree where the rule does not exist at all
  // — no rule, no complaint, no "featured" in stderr. It is the four mutations
  // that prove the rule fires; measured on origin/main, where the rule was
  // missing, two of them failed on `--check` exiting 0 where 1 was required and
  // two on the drifted copy no longer carrying the twin's text.
  const r = run({ args: ['--check'] });
  assert.equal(r.code, 0, r.both);
  assert.doesNotMatch(r.stderr, /featured/, r.both);
});

test('a featured card that reworded its grid twin is refused, naming the field', () => {
  // The defect this rule was written for: on 2026-09-12 the three copies differed
  // from their twins in 8 fields, and nothing in this file or in CI could see it,
  // because only the GRIDS reach registry.json. The strip is the first thing a
  // visitor reads and the one thing no gate was reading.
  const r = checkWithFeatured((original) =>
    mutateFeatured(
      original,
      'scRNA-seq clustering, annotation, trajectory, and integration.',
      'scRNA-seq analysis — QC, clustering, annotation, trajectory, and integration.'
    )
  );
  assert.equal(r.code, 1, `a reworded featured card must be refused\n${r.both}`);
  assert.match(r.stderr, /featured "single-cell" and its grid card disagree on description/);
  // The repair is part of the rule: which of the two copies is canonical is the
  // question a person hitting this has, and "they differ" does not answer it.
  assert.match(r.stderr, /the one registry\.json publishes, so it is the one to copy FROM/);
});

test('a featured card whose keywords drifted is refused, not only its prose', () => {
  // `data-tags` is the half a reader cannot see. It is what the card matches on
  // in the shelf's own text search, so a strip-only keyword answers queries the
  // registry row — and therefore the app's catalog and the model-facing search —
  // does not, for the very same download.
  const r = checkWithFeatured((original) =>
    mutateFeatured(
      original,
      'data-tags="single-cell scanpy seurat scvi"',
      'data-tags="single-cell scanpy seurat scvi clustering annotation"'
    )
  );
  assert.equal(r.code, 1, `drifted featured keywords must be refused\n${r.both}`);
  assert.match(r.stderr, /featured "single-cell" and its grid card disagree on keywords/);
});

test('a featured card filed under the wrong category chip is refused', () => {
  // `data-cat` has no twin to compare against on the card itself — a grid card
  // inherits its category from the grid — so this is the one field the rule has
  // to reach for deliberately. Wrong, the copy answers a category chip its twin
  // does not, which is the same skill appearing in two places and missing from one.
  const r = checkWithFeatured((original) =>
    mutateFeatured(original, 'data-cat="biomedical"', 'data-cat="core"')
  );
  assert.equal(r.code, 1, `a miscategorised featured card must be refused\n${r.both}`);
  assert.match(
    r.stderr,
    /featured "single-cell" declares data-cat="core", but its grid card sits in the Biomedical grid/
  );
});

test('a featured card that repeats nothing in the grids is refused by name', () => {
  // The strip is defined as repeats. A card only there is a download the registry
  // never publishes: the app's catalog cannot install it and the page's own search
  // is the only place it exists.
  const r = checkWithFeatured((original) =>
    mutateFeatured(
      original,
      'skill-single-cell/single-cell.zip',
      'skill-single-cell/not-a-published-skill.zip'
    )
  );
  assert.equal(r.code, 1, `a featured card with no twin must be refused\n${r.both}`);
  assert.match(r.stderr, /repeats no grid card/);
  assert.match(r.stderr, /not-a-published-skill\.zip/);
});
