#!/usr/bin/env node
// Generate registry.json from baam.html.
//
// The BAAM page keeps static cards as the no-JS fallback, and registry.json is
// the machine-readable catalog consumed by the page at runtime and by Biorouter.
// This script regenerates that catalog from the fallback cards so both surfaces
// stay in sync.
//
//   node scripts/build-registry.mjs
//
// Re-run this whenever baam.html's cards change.
//
// A real run writes THREE files from one source, so the privacy annotations on
// the cards cannot drift from what the app enforces:
//
//   landing/registry.json                                  — the published catalog
//   ui/desktop/src/components/baam/registry.fallback.json  — the bundled snapshot
//   crates/biorouter/src/privacy/registry_private.rs       — the compiled-in set
//
// They are written together, staged and renamed into place under a lock, so a
// reader never sees a half-written catalog and two generators cannot interleave.
//
//   node scripts/build-registry.mjs --check
//
// regenerates all three in memory and fails if any committed copy differs,
// which is how CI and `just check-everything` establish that they agree.
//
// Flags exist so the validations below can be exercised against the fixtures in
// scripts/fixtures/ without touching any of that. `--out` is mandatory
// alongside `--input`, and no run but the full one may write a published
// artifact — under any spelling of its path:
//
//   node scripts/build-registry.mjs --input scripts/fixtures/happy.html \
//                                   --out /tmp/happy-registry.json \
//                                   --emit-rust /tmp/happy-private.rs \
//                                   --assert-private-set
//
// `--assert-private-set` is the fixture opt-in for the closed-list assertion a
// real run always applies — see EXPECTED_PRIVATE_EXTENSIONS below.

import { closeSync, openSync, readFileSync, realpathSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { basename, dirname, join, resolve } from 'node:path';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = join(ROOT, '..');
const SITE_BASE = 'https://biorouter.ucsf.edu/';

// --- argv, so the validations can be exercised against fixtures -------------
// Both flags are needed. `--input` alone still WRITES to landing/registry.json,
// so a fixture run would overwrite the real 37-extension catalog with the
// fixture's two entries — which is why passing one without the other is an
// error rather than a default.
const argv = process.argv.slice(2);
const flag = (name, fallback) => {
  const i = argv.indexOf(name);
  if (i === -1) return fallback;
  const value = argv[i + 1];
  if (value === undefined || value.startsWith('--')) {
    // Otherwise `--out` as the last argument reaches writeFileSync(undefined),
    // which exits non-zero with a stack trace — indistinguishable from a
    // validation failure to anything checking only the status.
    console.error(`registry: ${name} needs a value`);
    process.exit(1);
  }
  return value;
};

// Path comparisons here decide whether a run may overwrite a published
// artifact, so they compare what the FILESYSTEM sees, never what was typed.
// `landing/registry.json` from the repo root is a different string from the
// absolute default and the same file; so is a symlink to it. A raw string
// comparison let both through, and a two-card fixture then replaced the
// 37-extension catalog while skipping its two siblings entirely.
function canonical(p) {
  const abs = resolve(p);
  try {
    return realpathSync(abs);
  } catch {
    // Not created yet — canonicalise the directory it will be created in, which
    // is what resolves any symlinked parent.
    try {
      return join(realpathSync(dirname(abs)), basename(abs));
    } catch {
      return abs;
    }
  }
}
const samePath = (a, b) => canonical(a) === canonical(b);

const DEFAULT_INPUT = join(ROOT, 'baam.html');
const DEFAULT_OUTPUT = join(ROOT, 'registry.json');
const FALLBACK_OUTPUT = join(REPO, 'ui/desktop/src/components/baam/registry.fallback.json');
const RUST_OUTPUT = join(REPO, 'crates/biorouter/src/privacy/registry_private.rs');
// The three files a real run publishes. Nothing else may write any of them.
const PUBLISHED = [DEFAULT_OUTPUT, FALLBACK_OUTPUT, RUST_OUTPUT];
const isPublished = (p) => PUBLISHED.some((q) => samePath(p, q));

const INPUT = flag('--input', DEFAULT_INPUT);
const OUTPUT = flag('--out', DEFAULT_OUTPUT);
// A real run is the full one: the real page in, all three artifacts out. Its
// two sibling outputs belong to it alone — a two-card fixture must never leave
// the compiled-in private set holding two invented names.
const IS_REAL_RUN = samePath(INPUT, DEFAULT_INPUT) && samePath(OUTPUT, DEFAULT_OUTPUT);

if (isPublished(OUTPUT) && !IS_REAL_RUN) {
  console.error(
    `registry: --input requires an --out of its own — "${OUTPUT}" resolves to ` +
      `${canonical(OUTPUT)}, which is one of the three published artifacts, and only a ` +
      `full run from ${DEFAULT_INPUT} may write those`
  );
  process.exit(1);
}

// `--emit-rust <path>` renders the compiled-in private set for a FIXTURE run.
// Without it, the one output that is a security artifact could only ever be
// produced from the real baam.html, where every private entry happens to have
// id == extension_name — so a generator keyed on the wrong field looked
// identical from outside. A real run always writes the canonical path, so the
// flag would only mean "somewhere else", which is not a thing anyone wants.
const EMIT_RUST = flag('--emit-rust', null);
if (EMIT_RUST !== null && IS_REAL_RUN) {
  console.error('registry: --emit-rust is for fixture runs; a real run always writes the crate path');
  process.exit(1);
}
if (EMIT_RUST !== null && isPublished(EMIT_RUST)) {
  console.error(
    `registry: --emit-rust may not target "${EMIT_RUST}" — it resolves to ` +
      `${canonical(EMIT_RUST)}, one of the three published artifacts`
  );
  process.exit(1);
}

// `--assert-private-set` applies the closed-list assertion to a FIXTURE run. A
// real run always applies it; the flag exists for exactly the reason
// `--emit-rust` does. Without it the rule could only ever be exercised against
// the real baam.html — which passes it — so a gate that had quietly stopped
// firing would be indistinguishable from a gate that works. And every fixture
// beside this one would have to name its private cards `cdwagent` and
// `ucsfomopagent`, which would delete `suffixed-download`, `three-affiliations`
// and `private-no-affiliation` as tests of anything.
const ASSERT_PRIVATE_SET = argv.includes('--assert-private-set');
if (ASSERT_PRIVATE_SET && IS_REAL_RUN) {
  console.error(
    'registry: --assert-private-set is for fixture runs; a real run always asserts the closed private set'
  );
  process.exit(1);
}

// `--check` regenerates in memory and compares, writing nothing. It is what
// makes "these three files agree with baam.html" a fact CI can establish rather
// than a claim the generated header used to make on its own behalf.
const CHECK = argv.includes('--check');
if (CHECK && !IS_REAL_RUN) {
  console.error('registry: --check reads the real baam.html and the three real outputs; it takes no --input/--out');
  process.exit(1);
}

// Two generators that read baam.html at different moments can interleave their
// writes and leave the three outputs from two different generations. Held only
// for the writes of a real run.
const LOCK = join(ROOT, '.registry-build.lock');
function withGenerationLock(body) {
  let fd;
  try {
    fd = openSync(LOCK, 'wx');
  } catch (err) {
    if (err.code !== 'EEXIST') throw err;
    console.error(
      `registry: another generator holds ${LOCK}. ` +
        `If none is running (a crash can outlive its run), delete that file.`
    );
    process.exit(1);
  }
  try {
    writeFileSync(fd, `${process.pid}\n`);
    body();
  } finally {
    closeSync(fd);
    rmSync(LOCK, { force: true });
  }
}

// Collect every violation and report them together, then exit non-zero. One
// `throw` per violation would hide the second and third problems behind the
// first, which is how a publisher ends up fixing this file three times.
const violations = [];
const fail = (msg) => violations.push(msg);

// ---- Institutions are DECLARED, one row each ------------------------------
// Institution id → display name. The warning copy for a cross-institutional
// flow renders from this, so an affiliation naming an id that is absent here
// has no name to render with and is a build failure rather than a silent pass.
//
// ⚠ **A list of rows, not an object literal, because adding an institution is a
// reviewed decision.** DR-26's third axis is not a UCSF feature: a future
// private provider or connector may be covered by another institution's
// agreements, and the code has to be able to say so. What must NOT be possible
// is an institution appearing by accident — a typo in one card's
// `data-affiliation` that quietly becomes a real institution nobody approved,
// or a row left behind after the last card naming it was deleted. Both are
// caught by `assertInstitutionRowsAreWellFormed` and
// `assertInstitutionsAreNamedByACard` below, which are referential rather than a
// count: "there should be N" is a gate people delete instead of update.
//
//   id             the normalised slug. `name_to_key` on the Rust side
//                  lowercases and strips whitespace, so write it that way here.
//   name           the display name DR-26 requires the warning to NAME. "This
//                  may be a compliance risk" is a shrug; "UCSF (ucsf)" is
//                  something a user can act on.
//   retainedUnused optional prose. Present ONLY for an institution deliberately
//                  kept in the map while no card names it — a connector being
//                  onboarded, or one just removed whose grants should still
//                  render. Absent means "some card must name this", which is
//                  what catches an orphan.
const INSTITUTIONS = [{ id: 'ucsf', name: 'UCSF' }];

// The published `id -> name` map, derived so there is exactly one place an
// institution is declared. `registry.json` and the compiled Rust snapshot both
// render from this, and the app reads the display name out of it.
const INSTITUTION_NAMES = Object.fromEntries(INSTITUTIONS.map((i) => [i.id, i.name]));

// ---- The private set is a CLOSED LIST -------------------------------------
// Every private extension, and the affiliation each one's data is under.
//
// ⚠ **A card cannot make itself private.** Writing `data-privacy="private"` on
// a new card is refused unless that extension is named here — and an entry named
// here with no matching card is refused too. Both directions, plus the
// affiliation, because all three are changes to what the daemon withholds from a
// public session and none of them should happen as a side effect of editing a
// marketing page.
//
// This is deliberately two edits (this list AND landing/baam.html), and that is
// the feature: the second edit is what makes somebody review the first.
//
// What this REPLACED, and what that trades away. The generator used to refuse a
// card whose *description* matched a keyword list ('patient', 'ehr', 'phi',
// 'clinical record', 'medical record', 'de-identified clinical') while declaring
// no tier — inferring a security property from marketing prose. It could only
// produce false failures: SPOKE describes diseases, and an imaging or literature
// tool can honestly say "patient" while touching nothing sensitive, so the rule
// punished authors for writing accurately. Its one real use was a *future*
// clinical extension whose author forgets to tag it, and the closed list does
// NOT catch that case — the set simply stays at two. Operator ruling,
// 2026-08-04: prose is not the place to catch it. If that case ever needs
// covering, the answer is an explicit field on the card, not a return to
// guessing from the description.
const EXPECTED_PRIVATE_EXTENSIONS = [
  ['cdwagent', ['ucsf']],
  ['ucsfomopagent', ['ucsf']],
];

// Every closed-list failure carries this. A gate that says only "unexpected"
// teaches people to delete the gate; one that names the file and says why it is
// two edits teaches them to make the decision.
const CLOSED_SET_ADVICE =
  'the private set is a closed list, not something a card grants itself — changing it means ' +
  'editing EXPECTED_PRIVATE_EXTENSIONS in landing/scripts/build-registry.mjs as well as the ' +
  'card in landing/baam.html, deliberately two edits so that adding, removing or re-affiliating ' +
  'a private extension is a reviewed decision rather than a side effect of writing a card';

// A declared extension name has to survive TWO reductions, in two different
// crates, and land on the same key both times. Both are written out here so the
// generator can assert they agree on each name rather than assume it:
//
//   name_to_key   (crates/biorouter/src/config/extensions.rs) — the reduction
//                 `classify_extension` applies before the private-set lookup:
//                 whitespace stripped, lowercased.
//   normalize     (crates/biorouter/src/agents/extension_manager.rs) — the
//                 reduction the manager applies to the config entry's name
//                 before storing it: whitespace stripped, every character
//                 outside [A-Za-z0-9_-] replaced by "_", lowercased.
//
// They agree exactly on ASCII letters, digits, "_" and "-". Outside that set
// they diverge — `Private.Agent` becomes "private.agent" here and
// "private_agent" in the running app — so the compiled-in set would hold a key
// the app never produces and the extension would classify Public. Where they
// disagree the name is REFUSED, not guessed at: a suffix-stripping or
// punctuation-folding heuristic in a security path is right until it isn't.
const nameToKey = (s) => s.replace(/\s+/g, '').toLowerCase();
const normalizeLikeTheManager = (s) =>
  [...s]
    .filter((c) => !/\s/.test(c))
    .map((c) => (/[A-Za-z0-9_-]/.test(c) ? c : '_'))
    .join('')
    .toLowerCase();

const html = readFileSync(INPUT, 'utf8');

const stripTags = (s) =>
  s
    .replace(/<[^>]+>/g, '')
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/\s+/g, ' ')
    .trim();

const slugFromUrl = (url) => {
  const file = url.split('/').pop() || url;
  return file.replace(/\.(zip|brxt)$/i, '');
};

const absolutize = (url) =>
  /^https?:\/\//i.test(url) ? url : SITE_BASE + url.replace(/^\.?\//, '');

// ---- A small, strict reader for element start tags ------------------------
// Not a full HTML parser, and it does not need to be. Everything the privacy
// rules depend on is (1) which element is the card and (2) that element's OWN
// attributes, so this reads start tags the way HTML actually defines them: any
// attribute order, optional whitespace around `=`, single or double quotes,
// unquoted values, valueless attributes, case-insensitive names.
//
// The previous reader searched the card's whole fragment for the literal string
// `data-privacy="…"`. Three legal spellings therefore read as an un-annotated —
// i.e. PUBLIC — card: `data-privacy = "private"`, `data-privacy='private'`, and
// a `class` attribute that was not written first (which hid the card entirely).
// A fourth, a `data-privacy` on a CHILD of the card, read as the card's own.
// Every one of those fails in the open direction.

const ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", '#39': "'" };
const decodeEntities = (s) => s.replace(/&(amp|lt|gt|quot|apos|#39);/g, (_, e) => ENTITIES[e]);

const isSpace = (c) => c === ' ' || c === '\t' || c === '\n' || c === '\r' || c === '\f';

/** Read the tag beginning at `src[at]` (which must be `<`), or null if none. */
function readTag(src, at) {
  if (src[at] !== '<') return null;
  let i = at + 1;
  const closing = src[i] === '/';
  if (closing) i++;
  const nameStart = i;
  while (i < src.length && /[a-zA-Z0-9:_-]/.test(src[i])) i++;
  if (i === nameStart) return null; // `<!--`, `<!doctype`, or a stray `<`
  const name = src.slice(nameStart, i).toLowerCase();
  const attrs = Object.create(null);
  while (i < src.length) {
    while (i < src.length && isSpace(src[i])) i++;
    if (src[i] === '>') return { name, attrs, closing, selfClosing: false, end: i + 1 };
    if (src[i] === '/' && src[i + 1] === '>') {
      return { name, attrs, closing, selfClosing: true, end: i + 2 };
    }
    const attrStart = i;
    while (i < src.length && !isSpace(src[i]) && src[i] !== '/' && src[i] !== '>' && src[i] !== '=') {
      i++;
    }
    if (i === attrStart) {
      i++; // a character that cannot start an attribute name; step over it
      continue;
    }
    const attrName = src.slice(attrStart, i).toLowerCase();
    let j = i;
    while (j < src.length && isSpace(src[j])) j++;
    if (src[j] !== '=') {
      // A valueless (boolean) attribute. `null`, never `''` — "declared with no
      // value" and "declared empty" are different mistakes and get different
      // messages.
      attrs[attrName] = null;
      continue;
    }
    i = j + 1;
    while (i < src.length && isSpace(src[i])) i++;
    const quote = src[i];
    if (quote === '"' || quote === "'") {
      const close = src.indexOf(quote, i + 1);
      if (close === -1) return null; // unterminated value: not a tag we can trust
      attrs[attrName] = decodeEntities(src.slice(i + 1, close));
      i = close + 1;
    } else {
      const valueStart = i;
      while (i < src.length && !isSpace(src[i]) && src[i] !== '>') i++;
      attrs[attrName] = decodeEntities(src.slice(valueStart, i));
    }
  }
  return null; // ran off the end of the source mid-tag
}

/**
 * Every start/end tag in `src`, with comments, doctypes and the raw-text
 * elements skipped. A `<div id="extensions-section">` written inside a comment
 * or a script string is text, not a section — the old reader's `indexOf` could
 * not tell the difference, which is why happy.html carries a paragraph warning
 * its own author not to spell the wrapper out in its comment.
 */
function* scanTags(src) {
  let i = 0;
  while (i < src.length) {
    const lt = src.indexOf('<', i);
    if (lt === -1) return;
    if (src.startsWith('<!--', lt)) {
      const close = src.indexOf('-->', lt + 4);
      i = close === -1 ? src.length : close + 3;
      continue;
    }
    if (src.startsWith('<!', lt) || src.startsWith('<?', lt)) {
      const close = src.indexOf('>', lt);
      i = close === -1 ? src.length : close + 1;
      continue;
    }
    const tag = readTag(src, lt);
    if (!tag) {
      i = lt + 1;
      continue;
    }
    yield { ...tag, start: lt };
    if (!tag.closing && !tag.selfClosing && (tag.name === 'script' || tag.name === 'style')) {
      const m = new RegExp(`</${tag.name}\\s*>`, 'i').exec(src.slice(tag.end));
      i = m ? tag.end + m.index + m[0].length : src.length;
      continue;
    }
    i = tag.end;
  }
}

const tagCache = new Map();
const tagsOf = (src) => {
  if (!tagCache.has(src)) tagCache.set(src, [...scanTags(src)]);
  return tagCache.get(src);
};

/**
 * The inner HTML of the element whose start tag is `all[k]`, or null when that
 * element is never closed. Depth is counted over elements of the same name, so
 * nested `<div>`s inside a card do not end it early.
 */
function innerOf(src, all, k) {
  const open = all[k];
  if (open.selfClosing) return '';
  let depth = 1;
  for (let n = k + 1; n < all.length; n++) {
    const t = all[n];
    if (t.name !== open.name || t.selfClosing) continue;
    depth += t.closing ? -1 : 1;
    if (depth === 0) return src.slice(open.end, t.start);
  }
  return null;
}

/**
 * The first element carrying `id`, as `{ attrs, inner }` — `inner: null` when
 * the element is never closed. `null` when there is no such element at all.
 * Both are failures at the call site; they are distinguished because the fix
 * differs.
 */
function elementById(src, id) {
  const all = tagsOf(src);
  const k = all.findIndex((t) => !t.closing && t.attrs.id === id);
  if (k === -1) return null;
  return { attrs: all[k].attrs, inner: innerOf(src, all, k) };
}

const classList = (tag) => String(tag.attrs.class ?? '').split(/\s+/).filter(Boolean);

/** Every element in `src` whose class list contains `cardClass`, in order. */
function pickCards(src, cardClass) {
  const all = tagsOf(src);
  const cards = [];
  for (let k = 0; k < all.length; k++) {
    const t = all[k];
    if (t.closing || !classList(t).includes(cardClass)) continue;
    cards.push({ attrs: t.attrs, inner: innerOf(src, all, k) ?? '' });
  }
  return cards;
}

// Metadata a card declares about itself. Read from the card element's own
// attributes and nowhere else; found on a descendant, it is a build failure
// rather than a value, because the two readings disagree and resolving that
// silently — in either direction — is how a tier gets decided by accident.
const CARD_METADATA_ATTRS = ['data-privacy', 'data-extension-name', 'data-affiliation'];
function nestedMetadataAttr(inner) {
  for (const t of scanTags(inner)) {
    if (t.closing) continue;
    const hit = CARD_METADATA_ATTRS.find((a) => a in t.attrs);
    if (hit) return hit;
  }
  return null;
}

const first = (re, s) => {
  const m = re.exec(s);
  return m ? m[1] : '';
};

// A `[data-privacy-badge]` chip is the PRIVACY BADGE, not a subject tag. The
// badge lives inside .ext-tags so it reads as the first chip of the row, and the
// row is also this generator's tag source — so without this filter the word
// "Private" would be published as a topic of the two private extensions, and
// then re-rendered as a second chip beside the badge the shelf already prepends.
// The card's real annotation is `data-privacy`; the badge is only its picture.
//
// The marker is an ATTRIBUTE, not the `private`/`public` class. `allTags` also
// harvests the skills shelf, where "Public" is a perfectly ordinary subject tag
// — filtering on the class would delete it, silently, from the published
// catalog. An attribute nothing else uses cannot collide with a topic name.
const PRIVACY_BADGE_ATTR = 'data-privacy-badge';

// The INSTITUTION badge, DR-26's third axis rendered on the card, and it is
// filtered out of the harvested tags for exactly the reason the privacy badge is
// — it shares the .ext-tags row, so without this "UCSF data" would be published
// as a subject topic of the two private extensions and then re-rendered as a
// second chip beside the badge the shelf already prepends.
//
// It carries its institution ID as its VALUE, not merely a marker, because the
// label a reader sees is a display name and the card's declaration is a slug:
// with only a marker there would be no way to tell whether the chip on the card
// is the picture of the affiliation the card declares, or of some other one.
const AFFILIATION_BADGE_ATTR = 'data-affiliation-badge';

/** Every `<span class="tag …">` in `block`, as `{ label, badge, institution }`. */
function tagSpans(block) {
  const all = tagsOf(block);
  const out = [];
  for (let k = 0; k < all.length; k++) {
    const t = all[k];
    if (t.closing || t.name !== 'span' || !classList(t).includes('tag')) continue;
    const inner = innerOf(block, all, k);
    if (inner === null) continue; // an unclosed span has no label to read
    out.push({
      label: stripTags(inner),
      badge: PRIVACY_BADGE_ATTR in t.attrs,
      // null = not an institution badge. '' = one that names no institution,
      // which is a chip claiming an affiliation it cannot name and is caught
      // below rather than folded into "not a badge".
      institution:
        AFFILIATION_BADGE_ATTR in t.attrs ? String(t.attrs[AFFILIATION_BADGE_ATTR] ?? '') : null,
    });
  }
  return out;
}

const allTags = (containerRe, s) => {
  const block = first(containerRe, s);
  if (!block) return [];
  return tagSpans(block)
    .filter((t) => !t.badge && t.institution === null)
    .map((t) => t.label);
};

/** The labels of the badge chips in a card's tag row, in document order. */
const privacyBadges = (containerRe, s) => {
  const block = first(containerRe, s);
  if (!block) return [];
  return tagSpans(block)
    .filter((t) => t.badge)
    .map((t) => t.label);
};

/** The institution badge chips in a card's tag row, in document order. */
const affiliationBadges = (containerRe, s) => {
  const block = first(containerRe, s);
  if (!block) return [];
  return tagSpans(block)
    .filter((t) => t.institution !== null)
    .map((t) => ({ id: t.institution, label: t.label }));
};

// The badge label, in one place, so the page and this generator cannot spell it
// differently. The DISPLAY NAME, never the id — a badge reading "ucsf data" is a
// slug that escaped onto the marketplace, and DR-26 asks for a name a reader can
// act on. The " data" suffix is not decoration either: it is what keeps this chip
// from being read as the ordinary "UCSF" org tag sitting beside it on the row.
const institutionBadgeLabel = (name) => `${name} data`;

// ---- Extensions ----------------------------------------------------------
// A missing or unclosed section, or a section with no cards, is a FAILURE and
// not an empty catalog. On a real run an empty catalog rewrites the compiled-in
// private set to `&[]`, which classifies every private extension as Public —
// the largest possible fail-open, produced by markup that merely got renamed.
const extSection = elementById(html, 'extensions-section');
if (!extSection) {
  fail(`no element with id="extensions-section" in ${INPUT} — there is nothing to read`);
} else if (extSection.inner === null) {
  fail(`the element with id="extensions-section" is never closed`);
}
const extScope = extSection && extSection.inner !== null ? extSection.inner : '';
const extCards = extScope ? pickCards(extScope, 'ext-card') : [];
if (extScope && extCards.length === 0) {
  fail(
    `the element with id="extensions-section" holds no ext-card elements — ` +
      `a catalog of nothing is a parse failure, not a result`
  );
}

const extensions = extCards.map(({ attrs, inner: card }, index) => {
  const name = stripTags(first(/<h3>([\s\S]*?)<\/h3>/, card));
  const org = stripTags(first(/<div class="ext-org">([\s\S]*?)<\/div>/, card));
  const description = stripTags(first(/<p class="ext-desc">([\s\S]*?)<\/p>/, card));
  const github = first(/<a href="([^"]+)"[^>]*class="ext-gh-link"/, card);
  const download = first(/<a href="([^"]+)"[^>]*class="brxt-chip"/, card);
  const tags = allTags(/<div class="ext-tags">([\s\S]*?)<\/div>/, card);
  const license = attrs['data-license'] || 'Apache-2.0';
  const id = slugFromUrl(download);
  // Every message below is prefixed with something a human can find in the
  // page. `id` is derived from the download filename, so it is exactly the
  // field that is missing in the one case where it would be most needed.
  const label = id || name || `ext-card #${index + 1}`;

  if (!download) {
    fail(`${label}: no .brxt download link, so there is no id to publish it under`);
  }

  // The DEFAULT matters more than the extraction: an un-annotated card is
  // public by construction, so R11(ii)'s fail-open direction is enforced by the
  // tool rather than by reviewer discipline. An annotation that is PRESENT but
  // unparseable is a different thing and must not collapse into the default —
  // hence reading `in attrs` rather than the value's truthiness.
  const declaresPrivacy = 'data-privacy' in attrs;
  const privacy = declaresPrivacy ? (attrs['data-privacy'] ?? '') : 'public';
  const extensionName = attrs['data-extension-name'] ?? '';
  // null = absent = unconstrained, which is the right default: most extensions
  // carry no institutional constraint and must not all become mismatches on the
  // day this ships. An empty list is NOT the same thing — see below.
  const affiliation =
    'data-affiliation' in attrs
      ? String(attrs['data-affiliation'] ?? '')
          .split(/\s+/)
          .filter(Boolean)
      : null;

  const nested = nestedMetadataAttr(card);
  if (nested) {
    fail(
      `${label}: ${nested} is set on an element inside the card, not on the card — ` +
        `card metadata (${CARD_METADATA_ATTRS.join(', ')}) must be declared on ` +
        `the card element itself, or the card and its contents disagree about the tier`
    );
  }
  if (declaresPrivacy && attrs['data-privacy'] === null) {
    fail(`${label}: data-privacy is present with no value; it must be "private" or "public"`);
  } else if (privacy !== 'private' && privacy !== 'public') {
    fail(`${label}: data-privacy must be "private" or "public", got "${privacy}"`);
  } else {
    // The badge is what a visitor with JavaScript disabled actually reads: the
    // shelf re-renders from registry.json, but the static cards it renders over
    // are the whole page for that visitor, and nothing there trims, filters or
    // corrects anything. So the badge is held to the card's declared tier rather
    // than trusted to have been kept in step by hand.
    //
    // BOTH directions are enforced. A card with no badge at all is the gap this
    // rule closes — an unbadged card reads as "not yet reviewed" rather than
    // "public", which is precisely the ambiguity labelling both states removes.
    const badges = privacyBadges(/<div class="ext-tags">([\s\S]*?)<\/div>/, card);
    const want = privacy === 'private' ? 'Private' : 'Public';
    if (badges.length !== 1) {
      fail(
        `${label}: the tag row carries ${badges.length} [${PRIVACY_BADGE_ATTR}] chips, expected ` +
          `exactly one reading "${want}" — without JavaScript this card is all a visitor sees`
      );
    } else if (badges[0] !== want) {
      fail(
        `${label}: the privacy badge reads "${badges[0]}" but the card declares ` +
          `data-privacy="${privacy}" — the two views would disagree about the tier`
      );
    }
  }
  // The join key between this catalog and the installed config entry. Validate
  // the KEY, not the raw attribute: `data-extension-name="   "` is truthy and
  // reduces to "", which would be published as extension_name:"" and compiled
  // into the private set as an empty string — leaving the real extension
  // classified Public, which is the outcome this rule exists to prevent.
  const extensionKey = nameToKey(extensionName);
  if (extensionName && !extensionKey) {
    fail(
      `${label}: data-extension-name is only whitespace, which reduces to an empty key — ` +
        `the tier lookup would be keyed on a name nobody can type`
    );
  } else if (privacy === 'private' && !extensionKey) {
    // No suffix-stripping heuristic. `spokeagent-0.4.1` proves ids and
    // manifest names diverge, and a heuristic in a security path is right
    // until it isn't.
    fail(`${label}: a private extension must declare data-extension-name`);
  }
  if (extensionKey && normalizeLikeTheManager(extensionName) !== extensionKey) {
    fail(
      `${label}: data-extension-name "${extensionName}" reduces to "${extensionKey}" here but ` +
        `"${normalizeLikeTheManager(extensionName)}" in the extension manager — a name must be ` +
        `ASCII letters, digits, "_" or "-" only, or the registry and the installed ` +
        `extension disagree about which extension this is`
    );
  }
  // Nothing here reads the description. What a card SAYS is prose; which
  // extensions are private is EXPECTED_PRIVATE_EXTENSIONS, asserted below.

  // DR-26 — affiliation is a third axis. HIPAA compliance does not transfer
  // between institutions, so a private connector may name whose agreements
  // cover its data.
  if (affiliation !== null) {
    if (privacy !== 'private') {
      fail(
        `${label}: data-affiliation is meaningless on a ${privacy} extension — ` +
          `affiliation asks under whose agreements, which only arises once data is private`
      );
    }
    if (affiliation.length === 0) {
      fail(
        `${label}: data-affiliation is present but empty — absent means unconstrained, ` +
          `so an empty list would turn a typo into a granted flow`
      );
    }
    for (const inst of affiliation) {
      if (!Object.prototype.hasOwnProperty.call(INSTITUTION_NAMES, inst)) {
        fail(`${label}: data-affiliation names "${inst}", which is not in the institutions map`);
      }
    }
  }

  // The institution badge is held to the declaration the same way the privacy
  // badge is, and for the same reason: without JavaScript the chip is the whole
  // of what a visitor is told, and nothing on that path trims, filters or
  // corrects it. Two sources for one fact drift.
  //
  // Both directions. A badge on a card that declares NO affiliation is the
  // dangerous one — it paints an institutional constraint the catalog does not
  // record, so the page and the daemon disagree about a flow the daemon is the
  // one enforcing.
  const instBadges = affiliationBadges(/<div class="ext-tags">([\s\S]*?)<\/div>/, card);
  const badgeIds = instBadges.map((b) => b.id);
  if (affiliation === null) {
    if (instBadges.length > 0) {
      fail(
        `${label}: the tag row carries an institution badge (${badgeIds.map((i) => `"${i}"`).join(', ')}) ` +
          `but the card declares no data-affiliation — absent means unconstrained, and a badge ` +
          `nobody declared paints a constraint the app will not enforce`
      );
    }
  } else if (
    privacy === 'private' &&
    affiliation.length > 0 &&
    affiliation.every((i) => Object.prototype.hasOwnProperty.call(INSTITUTION_NAMES, i))
  ) {
    // Only once the declaration itself is sound. A card that already failed one
    // of the rules above would otherwise be reported twice, the second time
    // pointing at the badge — the wrong half of the problem to fix.
    if (badgeIds.join(' ') !== affiliation.join(' ')) {
      fail(
        `${label}: the tag row badges [${badgeIds.join(', ')}] but the card declares ` +
          `data-affiliation="${affiliation.join(' ')}" — without JavaScript this card is all a ` +
          `visitor sees, so the badge and the attribute must name the same institutions in the ` +
          `same order`
      );
    } else {
      for (const b of instBadges) {
        const want = institutionBadgeLabel(INSTITUTION_NAMES[b.id]);
        if (b.label !== want) {
          fail(
            `${label}: the institution badge for "${b.id}" reads "${b.label}", expected "${want}" — ` +
              `the badge renders the institutions map's DISPLAY NAME, never the raw id`
          );
        }
      }
    }
  }

  // org looks like "BaranziniLab · UCSF · v0.4.3"
  const parts = org.split('·').map((p) => p.trim());
  const version = parts.find((p) => /^v?\d/.test(p)) || '';
  const organization = parts.filter((p) => p !== version).join(' · ');
  // ⚠ The PUBLISHED id is `data-registry-id` when the card states one, and the
  // download slug otherwise. It is deliberately NOT `extensionKey`.
  //
  // The slug alone was wrong for exactly one card. SPOKEAgent's release asset
  // is `spokeagent-0.4.1.brxt`, so its id became `spokeagent-0.4.1` — a VERSION
  // baked into a stable name, which the app then advertised as the extension's
  // name while the extension that installs is `spokeagent`, and which changes
  // with every release.
  //
  // The obvious repair — publish `extensionKey` as the id — is wrong twice
  // over, and both reasons are already written down in this file. It would
  // assert that ids and manifest names are the same thing, which the
  // extensionKey block above rejects in as many words ("`spokeagent-0.4.1`
  // proves ids and manifest names diverge"). And it would make id ==
  // extension_name for EVERY card, which is precisely the condition that makes
  // a generator keyed on the wrong field indistinguishable from a correct one —
  // the reason `suffixed-download` is a fixture at all. The security test would
  // have gone vacuous while still passing.
  //
  // So the id gets its own declaration. Two fields, stated separately, because
  // they are two different things that merely coincide on 36 of 37 cards.
  const declaredId = String(attrs['data-registry-id'] ?? '').trim();
  const publishedId = declaredId || id;
  if (attrs['data-registry-id'] !== undefined && !publishedId) {
    fail(`${label}: data-registry-id is empty, which would publish the card under no id at all`);
  }
  if (declaredId && normalizeLikeTheManager(declaredId) !== declaredId) {
    fail(
      `${label}: data-registry-id "${declaredId}" is not a bare key — an id is typed into ` +
        `install_extension, so it must be ASCII letters, digits, "_" or "-" only`
    );
  }
  // The rule the Rust snapshot test enforces, moved to the source. An id is a
  // STABLE NAME; a version belongs in `version` and in the asset filename, both
  // of which keep it. Catching it here tells whoever edits the page, at the
  // moment they edit it, instead of failing a Rust test three layers away that
  // names no card.
  const bareVersion = version.replace(/^v/, '');
  if (bareVersion && publishedId.endsWith(bareVersion)) {
    fail(
      `${label}: the published id "${publishedId}" ends with this card's version ` +
        `"${version}" — an id is a name and must not move when the version does. ` +
        `The download filename may carry the version; add data-registry-id to give ` +
        `the catalog a stable id.`
    );
  }
  return {
    id: publishedId,
    name,
    organization,
    version,
    description,
    tags,
    github,
    download: absolutize(download),
    filename: download.split('/').pop(),
    license,
    privacy,
    ...(extensionKey ? { extension_name: extensionKey } : {}),
    ...(affiliation !== null ? { affiliation } : {}),
  };
});

// Two cards claiming the same join key means the tier lookup cannot tell the
// two extensions apart, and whichever one a user installed they get the other
// one's answer. Duplicate ids are the same failure one field over.
const seenKey = new Map();
const seenId = new Map();
for (const e of extensions) {
  if (e.extension_name) {
    if (seenKey.has(e.extension_name)) {
      fail(
        `${seenKey.get(e.extension_name)} and ${e.id} both declare data-extension-name ` +
          `"${e.extension_name}" — the tier lookup cannot tell them apart`
      );
    } else {
      seenKey.set(e.extension_name, e.id);
    }
  }
  if (e.id) {
    if (seenId.has(e.id)) fail(`two cards publish the same id "${e.id}"`);
    else seenId.set(e.id, true);
  }
}

/**
 * The catalog's private set must be EXACTLY `EXPECTED_PRIVATE_EXTENSIONS` —
 * same keys, same affiliations — or the run fails naming the difference.
 *
 * Three failures, because three different edits reach the same bad end:
 *
 *   * a card nobody listed declares itself private (the set grows by accident);
 *   * a listed extension has no private card (the set SHRINKS by accident,
 *     which is the dangerous direction — the extension keeps working and simply
 *     stops being withheld from public sessions);
 *   * a listed extension is re-affiliated (which flows count as
 *     cross-institutional changes underneath the warning copy).
 *
 * A private card with no `data-extension-name` is skipped here: it has already
 * failed a rule that names the real problem, and reporting it a second time as
 * "not in the closed set" would point at the wrong file.
 */
function assertClosedPrivateSet(exts) {
  const key = (insts) => [...insts].sort().join(', ');
  const expected = new Map(EXPECTED_PRIVATE_EXTENSIONS.map(([k, insts]) => [k, key(insts)]));
  const seen = new Set();

  for (const e of exts) {
    if (e.privacy !== 'private' || !e.extension_name) continue;
    const label = e.id || e.name || e.extension_name;
    seen.add(e.extension_name);
    if (!expected.has(e.extension_name)) {
      fail(
        `${label}: declares data-privacy="private", but "${e.extension_name}" is not in the ` +
          `closed private set {${[...expected.keys()].join(', ')}} — ${CLOSED_SET_ADVICE}`
      );
      continue;
    }
    // Absent affiliation means unconstrained, which is a different answer from
    // ["ucsf"], not a milder one — so it is a mismatch, spelled out as such
    // rather than rendered as an empty list a reader would misread.
    const declared = Array.isArray(e.affiliation) ? `[${key(e.affiliation)}]` : '(absent)';
    const want = `[${expected.get(e.extension_name)}]`;
    if (declared !== want) {
      fail(
        `${label}: "${e.extension_name}" is private with affiliation ${declared}, but the ` +
          `closed private set records ${want} — ${CLOSED_SET_ADVICE}`
      );
    }
  }

  for (const k of expected.keys()) {
    if (!seen.has(k)) {
      fail(
        `the closed private set names "${k}", but no card in this catalog declares ` +
          `data-privacy="private" with that data-extension-name — ${CLOSED_SET_ADVICE}`
      );
    }
  }
}

/**
 * Referential integrity for the institution map — the check that replaces a
 * count (issue #56, Task 56 Step 3).
 *
 * The map used to be pinned by *cardinality*: the Rust side asserted this build
 * knew exactly one institution. That is the wrong shape of guard twice over. It
 * says nothing about whether the one institution is the RIGHT one, and it is a
 * gate whose only possible repair is deletion — the day a second institution is
 * genuinely added, "there should be one" is simply wrong, and the person adding
 * it deletes the assertion rather than learning anything from it.
 *
 * Referential integrity scales instead, and catches the two errors that are
 * actually made:
 *
 *   * an ORPHAN — a declared institution no card names, which is either a
 *     leftover from a deleted connector or a typo whose sibling typo is in the
 *     card. Silence here means the map slowly fills with ids that mean nothing,
 *     and a warning renders a display name for an institution that no longer
 *     exists.
 *   * the reverse — a card naming an institution the map does not declare — is
 *     caught per card above, where the label points at the card.
 *
 * ⚠ **An orphan is legitimate sometimes, so it is declarable rather than
 * forbidden.** `retainedUnused` is the escape hatch, and it is prose: an
 * institution kept for a connector being onboarded, or one whose grants should
 * still render after its card was pulled. Requiring a *reason* rather than a
 * boolean is what makes the row survivable review.
 *
 * ⚠ **Only the ORPHAN half is gated to a real run**, because only it asks a
 * question about the catalog: "used" is a property of the real cards, and a
 * fixture page legitimately names no institution at all. The row-shape half
 * below asks nothing about the catalog — it is a property of the `INSTITUTIONS`
 * literal alone — so gating it too meant a malformed row could not be caught by
 * any fixture run, and the only coverage it had was CI noticing eventually.
 */
function assertInstitutionsAreNamedByACard(exts) {
  const used = new Set();
  for (const e of exts) {
    for (const inst of Array.isArray(e.affiliation) ? e.affiliation : []) used.add(inst);
  }
  for (const i of INSTITUTIONS) {
    if (used.has(i.id)) continue;
    if (typeof i.retainedUnused === 'string' && i.retainedUnused.length > 0) continue;
    fail(
      `institution "${i.id}" is declared but no card names it. Delete the row, or record why ` +
        `it is kept with retainedUnused: '…' in INSTITUTIONS in ` +
        `landing/scripts/build-registry.mjs — an institution nobody references is either a ` +
        `leftover or the other half of a typo, and both render a display name for a flow ` +
        `that no longer exists`
    );
  }
}

/**
 * The shape of each declared row, which is a property of the literal and of
 * nothing else — so it is checked on EVERY run, fixture included.
 *
 * Each rule is one of the three ways a hand-written row goes wrong silently:
 *
 *   * no display name — DR-26 requires the warning to NAME both institutions,
 *     and a nameless row renders the raw slug at the user.
 *   * an un-normalised id — the Rust side reduces an id with `name_to_key`, so
 *     `UCSF` or `u csf` here is a row that can never be matched by anything.
 *   * a duplicate id — the later row wins in `INSTITUTION_NAMES` while the
 *     earlier one still reads as declared, so the published display name is
 *     whichever was written last.
 */
function assertInstitutionRowsAreWellFormed() {
  const seen = new Set();
  for (const i of INSTITUTIONS) {
    if (typeof i.id !== 'string' || i.id.length === 0) {
      fail(`an institution row has no id: ${JSON.stringify(i)}`);
      continue;
    }
    if (typeof i.name !== 'string' || i.name.length === 0) {
      fail(
        `institution "${i.id}" has no display name — DR-26 requires a cross-institutional ` +
          `warning to NAME both institutions, and a nameless row renders as a raw slug`
      );
    }
    if (i.id !== i.id.toLowerCase() || /\s/.test(i.id)) {
      fail(
        `institution "${i.id}" is not normalised — the Rust side reduces an institution id ` +
          `with name_to_key (lowercase, whitespace stripped), so this row can never be matched`
      );
    }
    if (seen.has(i.id)) fail(`institution "${i.id}" is declared twice`);
    seen.add(i.id);
  }
}

assertInstitutionRowsAreWellFormed();
if (IS_REAL_RUN || ASSERT_PRIVATE_SET) {
  assertClosedPrivateSet(extensions);
  assertInstitutionsAreNamedByACard(extensions);
}

// ---- The compiled-in private set -----------------------------------------
// Rendered as Rust source rather than JSON because there is no network path to
// the registry from the CLI or the daemon: the only fetch lives in Electron's
// `registry:fetch` handler. Without a compiled-in copy, Rust could enforce
// nothing at all.
//
// This generator is the ONLY authority on how the file is laid out: the three
// consts are emitted with `#[rustfmt::skip]`, and `--check` below is what holds
// them to their expected text. Two authorities is a deadlock, not a nuisance —
// see `renderSliceConst`, where the layout rule is still kept faithful to
// rustfmt so the generated file reads like the rest of the tree.
//
// Three consts, not one, and all three are security artifacts:
//
//   PRIVATE_EXTENSIONS      which extensions a public session may not reach
//   EXTENSION_AFFILIATIONS  whose agreements each private extension's data is
//                           under (DR-26's third axis)
//   INSTITUTIONS            id -> display name, which is what lets a
//                           cross-institutional warning NAME the two
//                           institutions rather than shrug at the user
//
// They are emitted together because they are one catalog. An affiliation that
// reached registry.json but not this file would be an affiliation the daemon
// and the CLI can never enforce, and a second hand-maintained list beside
// PRIVATE_EXTENSIONS is exactly what Task 47 forbids.

// rustfmt's two relevant defaults. `max_width` is the one everybody knows;
// `array_width` is 60 and applies to what sits BETWEEN the brackets, and it is
// the one that bites.
const RUSTFMT_MAX_WIDTH = 100;
const RUSTFMT_ARRAY_WIDTH = 60;

/**
 * A slice-literal const, wrapped the way rustfmt wraps one: on one line while
 * the item fits `max_width` AND the bracket contents fit `array_width`, then
 * with the value moved to its own indented line, then one element per line.
 *
 * ⚠ **Checking only the 100 columns is wrong, and it wedges the build rather
 * than merely producing an ugly file.** Measured by bisection against the
 * repo's rustfmt (1.92, `bin/rustfmt`): with 60 characters between the brackets
 * rustfmt keeps the hanging form; at 61 it demands one element per line. A
 * predictor that consulted `max_width` alone emitted the hanging form for any
 * contents 61 characters or wider — which a third affiliated extension reaches
 * — so `cargo fmt --check` rejected exactly the text `--check` below required.
 * Running either formatter to satisfy its own gate broke the other, and no
 * state satisfied both.
 *
 * That deadlock is now structurally impossible, not merely avoided: the consts
 * are emitted with `#[rustfmt::skip]`, so rustfmt no longer holds an opinion
 * about them at all and a miss here costs readability instead of a red CI. The
 * widths are still honoured because the file is read by humans as a security
 * artifact and should look like the code around it — and because rustfmt's
 * handling of a *single* very wide element does not follow this rule at all,
 * which is a shape no predictor short of rustfmt itself gets right.
 */
function renderSliceConst(head, cells) {
  const contents = cells.join(', ');
  const fitsArray = contents.length <= RUSTFMT_ARRAY_WIDTH;
  const oneLine = `${head} = &[${contents}];`;
  if (fitsArray && oneLine.length <= RUSTFMT_MAX_WIDTH) return oneLine;
  const hanging = `${head} =\n    &[${contents}];`;
  if (fitsArray && hanging.split('\n').every((line) => line.length <= RUSTFMT_MAX_WIDTH)) {
    return hanging;
  }
  return `${head} = &[\n${cells.map((c) => `    ${c},`).join('\n')}\n];`;
}

/**
 * `#[rustfmt::skip]`, and why every generated const carries it.
 *
 * Layout needs exactly one owner. With rustfmt also holding an opinion, any
 * shape `renderSliceConst` predicts wrongly is a deadlock between two gates
 * that both run in `just check-everything` — and the file they are fighting
 * over is the compiled-in privacy snapshot, so the tempting way out is to hand
 * edit it, which is the one thing its header forbids.
 */
const SKIP_FMT = '#[rustfmt::skip]';

function renderPrivateSet(exts) {
  const privateExts = exts.filter((e) => e.privacy === 'private');
  const keys = privateExts.map((e) => e.extension_name).sort();
  const body = renderSliceConst(
    'pub const PRIVATE_EXTENSIONS: &[&str]',
    keys.map((k) => `"${k}"`)
  );

  // Only entries that DECLARE an affiliation get a row. An extension with no
  // declaration must produce no row at all, never a row with an empty list:
  // absent means unconstrained, an empty allowlist permits nothing, and the two
  // are opposite answers.
  const affiliationRows = privateExts
    .filter((e) => Array.isArray(e.affiliation) && e.affiliation.length > 0)
    .map((e) => [e.extension_name, [...e.affiliation].sort()])
    .sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
    .map(([key, insts]) => `("${key}", &[${insts.map((i) => `"${i}"`).join(', ')}])`);
  const affiliations = renderSliceConst(
    'pub const EXTENSION_AFFILIATIONS: &[(&str, &[&str])]',
    affiliationRows
  );

  const institutions = renderSliceConst(
    'pub const INSTITUTIONS: &[(&str, &str)]',
    [...INSTITUTIONS]
      .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
      .map(({ id, name }) => `("${id}", "${name}")`)
  );

  return (
    [
      '//! **Generated file — do not edit by hand.**',
      '//!',
      '//! `landing/scripts/build-registry.mjs` writes this in the same run as',
      '//! `landing/registry.json` and the desktop fallback snapshot, from the',
      '//! `data-privacy` / `data-extension-name` / `data-affiliation` annotations on',
      '//! the extension cards in `landing/baam.html`. Regenerate all three with:',
      '//!',
      '//! ```text',
      '//! node landing/scripts/build-registry.mjs',
      '//! ```',
      '//!',
      '//! The set has to live in Rust: there is no network path to the registry from',
      '//! here (the only fetch is the Electron `main.ts` `registry:fetch` handler), so',
      '//! without this file the CLI and the daemon can enforce nothing.',
      '//!',
      '//! Drift between the three is detectable, not merely discouraged:',
      '//!',
      '//! ```text',
      '//! node landing/scripts/build-registry.mjs --check',
      '//! ```',
      '//!',
      '//! regenerates all three in memory and fails if any committed copy differs. It',
      '//! runs in CI (the Frontend workflow) and in `just check-everything`, so a hand',
      '//! edit here — or an interrupted run that updated only some of the three — is',
      '//! caught rather than trusted not to happen.',
      '//!',
      '//! Each const carries `#[rustfmt::skip]` so that `--check` above is the only',
      '//! authority on its layout. rustfmt wraps a slice literal by `array_width`',
      '//! (60 columns between the brackets), not by `max_width`, and a generator that',
      '//! predicted the wrong one produced text `cargo fmt --check` rejected and',
      '//! `--check` demanded — two gates in `just check-everything` with no state',
      '//! satisfying both, over the file whose header forbids the obvious way out.',
      '',
      '/// The BAAM extensions whose cards declare `data-privacy="private"`, and which',
      '/// so must never be admitted to a public session.',
      '///',
      '/// Values are `name_to_key` **keys** — whitespace-stripped and lowercased —',
      '/// which is the form `classify_extension` reduces its argument to before the',
      '/// lookup. That makes the entry match either spelling the registry publishes:',
      '/// the id (`cdwagent`) or the bundle `manifest.name` (`CDWAgent`).',
      SKIP_FMT,
      body,
      '',
      '/// Whose agreements each private extension\'s data is under — DR-26\'s third',
      '/// axis, keyed by the same `name_to_key` key as the set above.',
      '///',
      '/// A private extension with NO institutional constraint has **no row here**.',
      '/// Absent means unconstrained (`ExtensionAffiliation::Any`, reachable from any',
      '/// private model); an empty allowlist would mean the opposite — permits nothing',
      '/// — so the generator refuses `data-affiliation=""` rather than emitting one.',
      SKIP_FMT,
      affiliations,
      '',
      '/// Institution id → display name, for the warning copy.',
      '///',
      '/// DR-26 requires a cross-affiliation warning to NAME both institutions:',
      '/// "this may be a compliance risk" is a shrug, not a warning a user can act on.',
      '/// An id absent from this map has no display name, so the warning surfaces the',
      '/// raw id — and, being absent from every allowlist that does not literally spell',
      '/// it, it is a mismatch rather than a silent pass.',
      SKIP_FMT,
      institutions,
    ].join('\n') + '\n'
  );
}

// ---- Skills --------------------------------------------------------------
// The three grids are required on a REAL run only: the fixtures beside this
// script carry extension cards and no skills, and a fixture that had to restate
// the whole page would stop being readable at a glance.
function parseSkillGrid(id, category) {
  const section = elementById(html, id);
  if (!section || section.inner === null) {
    if (IS_REAL_RUN) fail(`no usable element with id="${id}" — the catalog is missing a skill grid`);
    return [];
  }
  const cards = pickCards(section.inner, 'skill-card');
  if (IS_REAL_RUN && cards.length === 0) {
    fail(`the element with id="${id}" holds no skill-card elements`);
  }
  return cards.map((card) => readSkillCard(card, category));
}

/**
 * One skill card, read as the registry row it becomes.
 *
 * Split out of `parseSkillGrid` so the featured strip below is read by the SAME
 * code that builds the row, rather than by a second parser that could disagree
 * with this one about what a card says — a differential whose two sides are
 * measured differently reports its own skew as drift.
 *
 * `category` is not on the card: a grid card inherits it from the grid it sits
 * in, and a featured card declares its own `data-cat`.
 */
function readSkillCard({ attrs, inner: card }, category) {
  const name = stripTags(first(/<h3>([\s\S]*?)<\/h3>/, card));
  const type = stripTags(first(/<div class="skill-type">([\s\S]*?)<\/div>/, card));
  const description = stripTags(first(/<p class="skill-desc">([\s\S]*?)<\/p>/, card));
  const download = first(/<a href="([^"]+)"[^>]*class="skill-dl-btn"/, card);
  const tags = allTags(/<div class="skill-tags">([\s\S]*?)<\/div>/, card);
  const license = attrs['data-license'] || 'Apache-2.0';
  const dataTags = String(attrs['data-tags'] ?? '')
    .split(/\s+/)
    .filter(Boolean);
  return {
    id: slugFromUrl(download),
    name,
    category,
    type,
    description,
    tags,
    keywords: dataTags,
    download: absolutize(download),
    filename: download.split('/').pop(),
    license,
  };
}

const skills = [
  ...parseSkillGrid('core-skill-grid', 'Core'),
  ...parseSkillGrid('dev-skill-grid', 'Developer'),
  ...parseSkillGrid('bio-skill-grid', 'Biomedical'),
];

// ---- The featured strip repeats grid cards, so it must repeat them exactly --
// `#skills-featured` is three hand-written copies of cards that also live in the
// grids, and ONLY the grids reach the registry. A divergence is therefore
// invisible to every other gate in this file while being the first thing a
// visitor reads: the strip promises one thing, and the catalog the app installs
// from — and the model searches — describes another.
//
// Measured on 2026-09-12, before this check existed, the three copies differed
// from their twins in 8 fields. The worst was not prose. The featured `ggplot2
// Visualization` advertised "User-invocable · /ggplot-visualization" for a skill
// whose SKILL.md declares `user-invocable: false` — a slash command that does not
// exist — and the shelf derives the type facet from that very text, so one skill
// answered the "User-invocable" chip as a featured card and the "Auto-applied"
// chip as a grid card.
//
// Real-run only, for the reason the grids above are: the fixtures beside this
// script carry extension cards and no skills shelf at all.
function assertFeaturedRepeatsItsGridTwin(rows) {
  const section = elementById(html, 'skills-featured');
  if (!section || section.inner === null) {
    fail('no usable element with id="skills-featured" — the catalog lost its featured strip');
    return;
  }
  const byId = new Map(rows.map((r) => [r.id, r]));
  for (const card of pickCards(section.inner, 'skill-card')) {
    const featured = readSkillCard(card, String(card.attrs['data-cat'] ?? ''));
    const twin = byId.get(featured.id);
    if (!twin) {
      fail(
        `featured card "${featured.name || featured.id}" repeats no grid card: nothing in ` +
          `the three grids downloads "${featured.filename || featured.id}". Every card in ` +
          '#skills-featured must be a copy of one in the grids.'
      );
      continue;
    }
    for (const field of ['name', 'type', 'description', 'tags', 'keywords']) {
      if (JSON.stringify(featured[field]) === JSON.stringify(twin[field])) continue;
      fail(
        `featured "${twin.id}" and its grid card disagree on ${field}: the strip says ` +
          `${JSON.stringify(featured[field])}, the grid says ${JSON.stringify(twin[field])}. ` +
          'The grid card is the one registry.json publishes, so it is the one to copy FROM.'
      );
    }
    // Compared case-insensitively on purpose: the grid supplies the display name
    // the registry row carries ("Core") and the strip its own chip slug ("core").
    // Same axis, two spellings — and a copy filed under the wrong one is reachable
    // from a category chip its twin is not.
    if (featured.category.toLowerCase() !== twin.category.toLowerCase()) {
      fail(
        `featured "${twin.id}" declares data-cat="${featured.category}", but its grid card ` +
          `sits in the ${twin.category} grid`
      );
    }
  }
}
if (IS_REAL_RUN) assertFeaturedRepeatsItsGridTwin(skills);

const registry = {
  version: 2,
  source: 'https://biorouter.ucsf.edu/baam',
  institutions: INSTITUTION_NAMES,
  extensions,
  skills,
};

if (violations.length) {
  for (const v of violations) console.error(`registry: ${v}`);
  console.error(`registry: ${violations.length} validation failure(s); nothing written`);
  process.exit(1);
}

// Render EVERYTHING before writing anything. The three outputs are one catalog
// in three forms, and a failure between them leaves a published registry the
// app does not enforce.
const out = JSON.stringify(registry, null, 2) + '\n';
const rustSource = renderPrivateSet(extensions);
const tally =
  `${extensions.length} extensions, ${skills.length} skills ` +
  `(${skills.filter((s) => s.category === 'Core').length} core, ` +
  `${skills.filter((s) => s.category === 'Developer').length} developer, ` +
  `${skills.filter((s) => s.category === 'Biomedical').length} biomedical)`;

// ---- --check: are the three committed outputs what baam.html generates? ----
// "Written by one command from one source" is not the same as "in sync": an
// interrupted run, a hand edit, or a rebase that took one side of a conflict
// all leave a published catalog the app does not enforce, and until this mode
// existed nothing could tell.
if (CHECK) {
  const drift = [];
  const compare = (path, want) => {
    let have = null;
    try {
      have = readFileSync(path, 'utf8');
    } catch {
      /* absent counts as drift */
    }
    if (have !== want) drift.push(path);
  };
  compare(DEFAULT_OUTPUT, out);
  compare(FALLBACK_OUTPUT, out);
  compare(RUST_OUTPUT, rustSource);
  if (drift.length) {
    for (const d of drift) console.error(`registry: ${d} is not what baam.html generates`);
    console.error('registry: run `node landing/scripts/build-registry.mjs` to regenerate');
    process.exit(1);
  }
  console.log(`registry: all three outputs are current (${tally})`);
  process.exit(0);
}

if (!IS_REAL_RUN) {
  writeFileSync(OUTPUT, out);
  if (EMIT_RUST !== null) {
    // A fixture run asked to see the security artifact. Same renderer, same
    // input, so a test can assert what the compiled-in set would hold for a
    // catalog the real baam.html does not contain.
    writeFileSync(EMIT_RUST, rustSource);
  }
} else {
  // Stage all three beside their destinations, then rename them into place.
  // A rename is atomic, so no reader ever sees a half-written catalog, and a
  // failure while writing (a full disk, a permission fault) leaves all three
  // real files untouched instead of one of them replaced.
  //
  // The lock closes the remaining window: two generators that read baam.html at
  // different moments could otherwise interleave their renames and leave the
  // three outputs from two different generations.
  withGenerationLock(() => {
    const staged = [
      // The published catalog.
      [DEFAULT_OUTPUT, out],
      // The desktop app bundles a snapshot so Browse works offline. It is the
      // same document; writing it here is what keeps "verified in sync, by
      // luck" from being how that stays true.
      [FALLBACK_OUTPUT, out],
      // And the compiled-in private set, which is the only form of this catalog
      // the CLI and the daemon can read at all — there is no network path to
      // the registry from Rust.
      [RUST_OUTPUT, rustSource],
    ].map(([path, contents]) => {
      const tmp = `${path}.tmp-${process.pid}`;
      writeFileSync(tmp, contents);
      return [tmp, path];
    });
    try {
      for (const [tmp, path] of staged) renameSync(tmp, path);
    } finally {
      for (const [tmp] of staged) rmSync(tmp, { force: true });
    }
  });
}

console.log(`registry.json written: ${tally}`);
