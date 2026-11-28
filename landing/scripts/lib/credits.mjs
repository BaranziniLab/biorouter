// The credits contract: the shape of landing/credits.json, the function that
// builds it from the two BAAM catalogs, and the parts of landing/baam.html that
// carry credit text (one line on every card, one curator line per shelf, one in
// the footer).
//
// Shared by build-registry.mjs (writes credits.json and the desktop app's copy
// under the registry's generation lock, and compares both in --check),
// write-credits.mjs (writes the lines into baam.html), check-catalog.mjs (PV14)
// and the browser test. No file system and no network: callers pass parsed
// catalogs and page text. Every credit text comes from catalog-rules.mjs
// (creditLine, originSentence, CURATED_BY), the file biorouter-skills shares
// verbatim, so the page, the app and the skills repository print the same words.
//
// credits.json, version 1. registry.json cannot grow a field (registry v2 is
// parsed with deny_unknown_fields by every daemon since v1.90.0), so credits
// travel beside it, keyed by the same ids:
//
//   { version: 1, source, curated_by, made_by_lab, skills_commit,
//     extensions: { <id>: Entry }, skills: { <id>: Entry } }
//
// with one Entry for every catalog row, active or not, in catalog order:
//
//   { status, origin, line, summary, licence,
//     sources: [Source], ideas: [Idea], data_and_services: [Data],
//     people: [Person], written_for_biorouter: [name], earlier: [Earlier],
//     moved_to, licence_files, record }
//
//   Source  { name, url, pin, licence, credit, people, use, what, owner, via }
//   Idea    { name, url, credit, people, what, cite }
//   Data    { kind, name, url, terms, credit, cite, how }
//   Person  { name, affiliation, role }
//   Earlier { kind: "version", version, origin, licence }
//         | { kind: "idea", name, url, credit, people, what, cite }
//
// `line` is the card and app line, `summary` the sentence a full record opens
// with. `sources` is every upstream entry in record order; `ideas` the ideas
// that shape the item as it ships now (applies_to "current"); `earlier` the
// history entries whose origin differs from today's and the ideas that shaped
// only an earlier version. `url` is an http(s) URL or null, never anything a
// page could execute. `licence_files` links the item's notice file (a skill's
// LICENSE-UPSTREAM, else the skills repository LICENSE; an extension's NOTICE at
// its release tag). `record` links the full record: the skills README section
// at the mirrored commit; null for extensions until their repositories carry
// the generated section. A reader allows unknown fields and checks `version`.

import {
  CARD_CREDIT_PATTERN,
  CURATED_BY,
  EXTENSION_BY_LAB,
  MADE_BY_LAB,
  ORIGIN_SENTENCES,
  creditLine,
  currentIdeas,
  isPublication,
  originSentence,
  sourceName,
  sourceUrl,
  usesOf,
  viaOf,
} from './catalog-rules.mjs';

export const CREDITS_VERSION = 1;
export const CREDITS_SOURCE = 'https://biorouter.ucsf.edu/baam';
/** The two copies build-registry.mjs writes, relative to the repository root. Same bytes. */
export const CREDITS_PATHS = Object.freeze([
  'landing/credits.json',
  'ui/desktop/src/components/baam/credits.fallback.json',
]);
export const SKILLS_REPO_URL = 'https://github.com/BaranziniLab/biorouter-skills';

/** The card line: <p class="item-credit">, the next element after a card's description. */
export const CARD_CREDIT_CLASS = 'item-credit';
/** One <p class="shelf-curator"> per shelf, the shelf's last child. */
export const SHELF_CURATOR_CLASS = 'shelf-curator';
/** One <span class="foot-curator"> in the footer's brand column. */
export const FOOT_CURATOR_CLASS = 'foot-curator';
/** The shelves that each carry one curator line, by element id, in page order. */
export const CURATED_SHELVES = Object.freeze(['extensions-section', 'workflows-section', 'skills-section']);
/** How many sources (or ideas, or data entries) an opened panel lists before "Show all N". */
export const PANEL_SOURCES_SHOWN = 3;
/** The label of the detail panel row. */
export const PANEL_ROW_LABEL = 'Provenance and credits';

const text = (x) => (typeof x === 'string' && x.trim() ? x.trim() : null);
const names = (x) => (Array.isArray(x) ? x.filter((n) => typeof n === 'string' && n.trim()).map((n) => n.trim()) : []);
const list = (x) => (Array.isArray(x) ? x : []);
const HTTP = /^https?:\/\/[^\s"'<>]+$/i;
/** `x` when it is an http(s) URL, else null: a value from a record never becomes a javascript: or data: link. */
export const httpUrl = (x) => (typeof x === 'string' && HTTP.test(x.trim()) ? x.trim() : null);

/**
 * "version 0.9.3", "commit 7e9d133", "DOI 10.x/y", or null: how the panel pins
 * a source. A guideline or paper is pinned by its DOI (its name already says
 * which edition), else by nothing the panel prints.
 */
export function pinText(u) {
  if (isPublication(u)) return text(u?.doi) ? `DOI ${text(u.doi)}` : null;
  const v = text(u?.version);
  if (v) return `version ${v.replace(/^v(?=\d)/, '')}`;
  if (typeof u?.commit === 'string' && /^[0-9a-f]{7,40}$/.test(u.commit)) return `commit ${u.commit.slice(0, 7)}`;
  return null;
}

export function sourceEntry(u) {
  return {
    name: sourceName(u),
    url: httpUrl(sourceUrl(u)),
    pin: pinText(u),
    licence: text(u?.licence),
    credit: text(u?.credit),
    people: names(u?.people),
    use: usesOf(u),
    what: text(u?.what),
    owner: u?.owner === 'lab' ? 'lab' : 'external',
    via: viaOf(u).filter((v) => typeof v === 'string'),
  };
}

export function ideaEntry(i) {
  return {
    name: text(i?.name),
    url: httpUrl(i?.url),
    credit: text(i?.credit),
    people: names(i?.people),
    what: text(i?.what),
    cite: text(i?.cite),
  };
}

export function dataEntry(d) {
  return {
    kind: text(d?.kind),
    name: text(d?.name),
    url: httpUrl(d?.url),
    terms: text(d?.terms),
    credit: text(d?.credit),
    cite: text(d?.cite),
    how: text(d?.how),
  };
}

export function personEntry(c) {
  return { name: text(c?.name), affiliation: text(c?.affiliation), role: text(c?.role) };
}

/** History entries whose origin differs from today's, then the ideas that shaped only an earlier version. */
export function earlierEntries(row) {
  const origin = row?.source?.origin;
  const versions = list(row?.history)
    .filter((h) => text(h?.origin) && h.origin !== origin)
    .map((h) => ({ kind: 'version', version: text(h.version), origin: h.origin, licence: text(h.licence) }));
  const ideas = list(row?.source?.ideas_from)
    .filter((i) => i?.applies_to === 'earlier')
    .map((i) => ({ kind: 'idea', ...ideaEntry(i) }));
  return [...versions, ...ideas];
}

/** A bundle's active components written for Biorouter (in-house or ideas-derived), by name. */
export function writtenForBiorouter(row) {
  if (row?.kind !== 'bundle' || !row.components || row.source?.origin === 'in-house') return [];
  return Object.entries(row.components)
    .filter(([, c]) => c?.status === 'active' && (c.origin === 'in-house' || c.origin === 'ideas-derived'))
    .map(([folder, c]) => text(c.name) ?? folder);
}

const encPath = (p) => String(p).split('/').map(encodeURIComponent).join('/');
const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

function skillLinks(row, commit) {
  if (!text(commit)) return { licence_files: null, record: null };
  const base = `${SKILLS_REPO_URL}/blob/${commit}`;
  const notice = text(row?.notice_file) && text(row?.path) ? `${base}/${encPath(row.path)}/${encPath(row.notice_file)}` : `${base}/LICENSE`;
  return { licence_files: notice, record: `${base}/README.md#provenance-and-credits` };
}

/**
 * Extensions whose NOTICE is not at the root of their repository, by registry
 * id. The row names the repository and the release tag but not the folder the
 * extension is built from. gptr Agent moved into the gptr package repository
 * at 1.3.1 and is built from mcp-bridge/ (the tree at v1.3.1 has no root
 * NOTICE; read 2026-11-28).
 */
export const EXTENSION_NOTICE_PATHS = Object.freeze({ gptragent: 'mcp-bridge/NOTICE' });

function extensionLinks(row) {
  const tag = text(row?.release?.tag);
  const repo = text(row?.repository);
  if (!tag || !repo || !REPO.test(repo)) return { licence_files: null, record: null };
  const notice = Object.hasOwn(EXTENSION_NOTICE_PATHS, row?.id) ? EXTENSION_NOTICE_PATHS[row.id] : 'NOTICE';
  return { licence_files: `https://github.com/${repo}/blob/${encodeURIComponent(tag)}/${encPath(notice)}`, record: null };
}

/** The credits entry of one row. `catalog` is "extensions" or "skills". */
export function creditsEntry(row, catalog, { skillsCommit = null } = {}) {
  const src = row?.source ?? {};
  const links = catalog === 'skills' ? skillLinks(row, skillsCommit) : extensionLinks(row);
  return {
    status: row?.status ?? null,
    origin: src.origin ?? null,
    line: creditLine(row, { catalog }),
    summary: originSentence(row, { catalog }),
    licence: text(row?.licence),
    sources: list(src.upstreams).map(sourceEntry),
    ideas: currentIdeas(row).map(ideaEntry),
    data_and_services: list(src.data_and_services).map(dataEntry),
    people: list(row?.contributors).map(personEntry),
    written_for_biorouter: writtenForBiorouter(row),
    earlier: earlierEntries(row),
    moved_to: row?.status === 'active' ? null : (src.content_moved_to ?? null),
    licence_files: links.licence_files,
    record: links.record,
  };
}

/** credits.json from loadCatalogs()'s extensions and skills (the mirror, with its source_commit). */
export function buildCredits({ extensions, skills }) {
  const skillsCommit = text(skills?.source_commit);
  const out = {
    version: CREDITS_VERSION,
    source: CREDITS_SOURCE,
    curated_by: CURATED_BY,
    made_by_lab: MADE_BY_LAB,
    skills_commit: skillsCommit,
    extensions: {},
    skills: {},
  };
  for (const row of list(extensions?.items)) out.extensions[row.id] = creditsEntry(row, 'extensions');
  for (const row of list(skills?.items)) out.skills[row.id] = creditsEntry(row, 'skills', { skillsCommit });
  return out;
}

/** The bytes both copies hold. */
export function serializeCredits(credits) {
  return JSON.stringify(credits, null, 2) + '\n';
}

// ---------------------------------------------------------------------------
// The page: landing/baam.html.
//
// Cards. Every .ext-card in #extensions-grid (the no-JS view, which is also the
// registry's input) and every .skill-card in #skills-section (the featured strip
// included) carries exactly one <p class="item-credit">LINE</p> as the next
// element after its description paragraph, LINE being creditLine's text with &,
// < and > escaped. The element is the generator's marker: write-credits.mjs
// owns it and nothing else in the card. A card's id is the registry's: the
// download file name without .brxt or .zip.
//
// Curator lines. Each shelf in CURATED_SHELVES ends with one
// <p class="shelf-curator">CURATED_BY</p>, and the footer's brand column holds
// one <span class="foot-curator">CURATED_BY</span>. They are placed once by
// hand; write-credits.mjs keeps their text equal to the constant.

/** & < > escaped: the text of an element, never markup. */
export function escapeText(s) {
  return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

const slugFromUrl = (url) => (String(url).split('/').pop() || String(url)).replace(/\.(zip|brxt)$/i, '');

function landmark(html, needle) {
  const at = html.indexOf(needle);
  if (at === -1) throw new Error(`landing/baam.html has no ${needle}; the credits generator needs it`);
  return at;
}

/** [start, end) of the parts of the page each rule reads. Throws when a landmark is gone. */
export function pageRanges(html) {
  const ext = landmark(html, 'id="extensions-section"');
  const extGrid = landmark(html, 'id="extensions-grid"');
  const wf = landmark(html, 'id="workflows-section"');
  const skills = landmark(html, 'id="skills-section"');
  const moved = landmark(html, 'id="baam-moved"');
  const footer = landmark(html, '<footer class="site-footer">');
  const footerEnd = html.indexOf('</footer>', footer);
  if (!(ext < extGrid && extGrid < wf && wf < skills && skills < moved && moved < footer && footerEnd > footer)) {
    throw new Error('landing/baam.html: the shelves are not in the order extensions, workflows, skills, moved list, footer');
  }
  return {
    cards: { extensions: [extGrid, wf], skills: [skills, moved] },
    shelves: { 'extensions-section': [ext, wf], 'workflows-section': [wf, skills], 'skills-section': [skills, moved] },
    footer: [footer, footerEnd],
  };
}

const CARD_START = { extensions: /<div class="ext-card(?:\s[^"]*)?"[^>]*>/g, skills: /<div class="skill-card(?:\s[^"]*)?"[^>]*>/g };
const DESC = { extensions: /<p class="ext-desc">[\s\S]*?<\/p>/, skills: /<p class="skill-desc">[\s\S]*?<\/p>/ };
const DOWNLOAD = { extensions: /<a href="([^"]+)"[^>]*class="brxt-chip"/, skills: /<a href="([^"]+)"[^>]*class="skill-dl-btn"/ };
const CREDIT_EL = /<p class="item-credit">([\s\S]*?)<\/p>/g;
/**
 * A class attribute holding one of `tokens`, as a browser reads it: the value
 * in double quotes, single quotes or none, the attribute name in any case, the
 * token a whole class name (so "item-credits" is not "item-credit").
 */
function classAttr(...tokens) {
  const t = `(?:${tokens.join('|')})`;
  return new RegExp(
    `\\s[cC][lL][aA][sS][sS]\\s*=\\s*(?:"(?:[^"]*\\s)?${t}(?:\\s[^"]*)?"|'(?:[^']*\\s)?${t}(?:\\s[^']*)?'|${t}(?=[\\s/>]))`,
    'g',
  );
}
const ANY_CREDIT_CLASS = classAttr(CARD_CREDIT_CLASS);
const ANY_CURATOR_CLASS = classAttr(SHELF_CURATOR_CLASS, FOOT_CURATOR_CLASS);
const REGISTRY_ID = /\sdata-registry-id="([^"]*)"/;

/**
 * [closeStart, closeEnd) of the end tag that closes the element whose start
 * tag begins at `open` (a card's or a shelf's <div>), counting nested elements
 * of the same name and skipping comments and scripts. [length, length] when
 * it never closes.
 */
export function elementClose(html, open) {
  const name = /^<([a-zA-Z][a-zA-Z0-9]*)\b/.exec(html.slice(open, open + 40))?.[1];
  if (!name) return [html.length, html.length];
  const re = new RegExp(`<!--[\\s\\S]*?-->|<script\\b[\\s\\S]*?<\\/script\\s*>|<(\\/?)${name}(?=[\\s/>])[^>]*>`, 'gi');
  re.lastIndex = open;
  let depth = 0;
  for (let m = re.exec(html); m; m = re.exec(html)) {
    if (m[1] === undefined) continue;
    depth += m[1] ? -1 : 1;
    if (depth === 0) return [m.index, m.index + m[0].length];
  }
  return [html.length, html.length];
}

/**
 * A card's id as build-registry.mjs and baam.js read it: an extension card's
 * `data-registry-id` when its start tag states a non-empty one, else the
 * download file name without its extension. Skill cards have only the file name.
 */
function cardId(kind, startTag, download) {
  if (kind === 'extensions') {
    const declared = REGISTRY_ID.exec(startTag)?.[1].trim();
    if (declared) return declared;
  }
  return download ? slugFromUrl(download) : null;
}

/**
 * Every card the credit rule covers, in page order:
 * { kind, id, start, end, close, descEnd, indent, credits: [{ start, end, inner }], stray }
 * Offsets are into `html`. [start, end) runs to the next card (for a shelf's
 * last card, to the end of the shelf's cards range); `close` is the offset just
 * past the card's own end tag. `descEnd` is null for a card with no description,
 * `id` null for one with no download link. `stray` counts item-credit classes
 * that are not the exact <p class="item-credit"> element.
 */
export function cardCreditSites(html) {
  const ranges = pageRanges(html);
  const sites = [];
  for (const kind of ['extensions', 'skills']) {
    const [from, to] = ranges.cards[kind];
    const re = new RegExp(CARD_START[kind].source, 'g');
    re.lastIndex = from;
    const starts = [];
    const tags = [];
    for (let m = re.exec(html); m && m.index < to; m = re.exec(html)) {
      starts.push(m.index);
      tags.push(m[0]);
    }
    starts.forEach((start, k) => {
      const end = k + 1 < starts.length ? starts[k + 1] : to;
      const span = html.slice(start, end);
      const desc = DESC[kind].exec(span);
      const dl = DOWNLOAD[kind].exec(span);
      const credits = [...span.matchAll(CREDIT_EL)].map((m) => ({ start: start + m.index, end: start + m.index + m[0].length, inner: m[1] }));
      const any = (span.match(ANY_CREDIT_CLASS) ?? []).length;
      let indent = '';
      if (desc) {
        const lineStart = span.lastIndexOf('\n', desc.index) + 1;
        indent = /^[ \t]*/.exec(span.slice(lineStart, desc.index))[0];
      }
      sites.push({
        kind,
        id: cardId(kind, tags[k], dl ? dl[1] : null),
        start,
        end,
        close: elementClose(html, start)[1],
        descEnd: desc ? start + desc.index + desc[0].length : null,
        indent,
        credits,
        stray: any - credits.length,
      });
    });
  }
  return sites;
}

/**
 * { shelves: { <shelf id>: [{ start, end, inner }] }, footer: [...], stray,
 * shelfClose: { <shelf id>: offset } } for the curator lines. A curator element
 * on a card is never a shelf's line (it is counted in `stray`), and
 * `shelfClose` is where the shelf's own end tag starts, so a caller can tell
 * whether its line is the shelf's last element.
 */
export function curatorSites(html, sites = cardCreditSites(html)) {
  const ranges = pageRanges(html);
  const onCard = (at) => sites.some((s) => at >= s.start && at < s.close);
  const find = (re, [from, to]) =>
    [...html.slice(from, to).matchAll(re)]
      .map((m) => ({ start: from + m.index, end: from + m.index + m[0].length, inner: m[1] }))
      .filter((c) => !onCard(c.start));
  const shelves = {};
  const shelfClose = {};
  for (const id of CURATED_SHELVES) {
    shelves[id] = find(/<p class="shelf-curator">([\s\S]*?)<\/p>/g, ranges.shelves[id]);
    shelfClose[id] = elementClose(html, html.lastIndexOf('<', ranges.shelves[id][0]))[0];
  }
  const footer = find(/<span class="foot-curator">([\s\S]*?)<\/span>/g, ranges.footer);
  const placed = Object.values(shelves).reduce((n, s) => n + s.length, 0) + footer.length;
  const all = (html.match(ANY_CURATOR_CLASS) ?? []).length;
  return { shelves, footer, stray: all - placed, shelfClose };
}

/**
 * The number of item-credit classes anywhere on the page that no card covers:
 * one in a shelf header, a group head, the moved list, the footer or a detail
 * panel. Page-wide, as curatorSites counts the curator classes: a credit
 * element is the generator's only where a card holds it.
 */
export function strayCardCredits(html, sites = cardCreditSites(html)) {
  const all = (String(html).match(ANY_CREDIT_CLASS) ?? []).length;
  const inCards = sites.reduce((n, s) => n + s.credits.length + s.stray, 0);
  return all - inCards;
}

/**
 * [start, end) of every generated element where it belongs: each card's
 * <p class="item-credit">, each shelf's <p class="shelf-curator"> and the
 * footer's <span class="foot-curator">. An element of one of these classes
 * anywhere else is not generated. On a page without the landmarks the finders
 * need, which PV14 already fails as "page", every such element is taken, so
 * the scan still names only hand-written wording there.
 */
export function generatedCreditRanges(html) {
  let sites;
  let cur;
  try {
    sites = cardCreditSites(html);
    cur = curatorSites(html, sites);
  } catch {
    const text = String(html);
    return [/<p class="item-credit">[\s\S]*?<\/p>/g, /<p class="shelf-curator">[\s\S]*?<\/p>/g, /<span class="foot-curator">[\s\S]*?<\/span>/g]
      .flatMap((re) => [...text.matchAll(re)])
      .map((m) => [m.index, m.index + m[0].length]);
  }
  return [
    ...sites.flatMap((s) => s.credits),
    ...Object.values(cur.shelves).flat(),
    ...cur.footer,
  ].map((c) => [c.start, c.end]);
}

/** The credits entry a card shows, or null. */
export function entryFor(credits, kind, id) {
  const map = credits?.[kind];
  return map && Object.hasOwn(map, id) ? map[id] : null;
}

/**
 * PV14, the page half: [{ subject, message }] for every card, shelf or footer
 * line that is missing, doubled, misplaced or different from what `credits`
 * (buildCredits' output) says, and for credit wording written by hand
 * elsewhere on the page. Subjects: "card:<kind>:<id>",
 * "card-id:<kind>:<offset>", "shelf:<shelf id>" (also when its one line is
 * not the shelf's last element), "footer", "stray" (a curator class on a card
 * or outside a shelf or the footer, or an item-credit class outside a card,
 * both counted page-wide), "text:<excerpt>" (handWrittenCredit, which reads
 * `credits` for the lines it looks for).
 */
export function creditPageProblems(html, credits) {
  const out = [];
  const sites = cardCreditSites(html);
  for (const s of sites) {
    const where = `${s.kind === 'skills' ? 'skill' : 'extension'} card ${s.id ?? `at offset ${s.start}`}`;
    if (!s.id) {
      out.push({ subject: `card-id:${s.kind}:${s.start}`, message: `${where} has no download link, so no credits entry can be found for it` });
      continue;
    }
    const subject = `card:${s.kind}:${s.id}`;
    const entry = entryFor(credits, s.kind, s.id);
    if (!entry) {
      out.push({ subject, message: `${where} has no entry in credits.json (no ${s.kind} row with this id)` });
      continue;
    }
    const want = escapeText(entry.line);
    if (s.stray > 0) out.push({ subject, message: `${where} carries an item-credit class on an element other than <p class="item-credit">` });
    if (s.credits.length !== 1) {
      out.push({ subject, message: `${where} carries ${s.credits.length} credit lines; it needs exactly one (run node landing/scripts/write-credits.mjs)` });
      continue;
    }
    const c = s.credits[0];
    // Order first: slice(descEnd, start) is empty when the line sits before the description.
    if (s.descEnd === null || c.start < s.descEnd || html.slice(s.descEnd, c.start).trim() !== '') {
      out.push({ subject, message: `${where}: the credit line is not the next element after the description` });
    }
    if (c.inner !== want) out.push({ subject, message: `${where} says "${c.inner}", the catalog gives "${want}" (run node landing/scripts/write-credits.mjs)` });
  }
  const cur = curatorSites(html, sites);
  const want = escapeText(CURATED_BY);
  for (const id of CURATED_SHELVES) {
    const found = cur.shelves[id];
    if (found.length !== 1) out.push({ subject: `shelf:${id}`, message: `the shelf #${id} has ${found.length} curator lines outside its cards; it needs exactly one <p class="shelf-curator">` });
    else if (found[0].inner !== want) out.push({ subject: `shelf:${id}`, message: `the curator line of #${id} says "${found[0].inner}", not "${want}"` });
    else if (html.slice(found[0].end, cur.shelfClose[id]).trim() !== '') {
      out.push({ subject: `shelf:${id}`, message: `the curator line of #${id} is not the shelf's last element; it goes after the shelf's no-results line, right before the shelf closes` });
    }
  }
  if (cur.footer.length !== 1) out.push({ subject: 'footer', message: `the footer has ${cur.footer.length} curator lines; it needs exactly one <span class="foot-curator">` });
  else if (cur.footer[0].inner !== want) out.push({ subject: 'footer', message: `the footer's curator line says "${cur.footer[0].inner}", not "${want}"` });
  if (cur.stray > 0) out.push({ subject: 'stray', message: `${cur.stray} curator class(es) sit on a card, outside a shelf or the footer, or on another element` });
  const strayCredits = strayCardCredits(html, sites);
  if (strayCredits > 0) {
    out.push({ subject: 'stray', message: `${strayCredits} item-credit class(es) sit outside a card (a shelf header, the moved list, the footer or a detail panel); a credit line belongs on a card, written by node landing/scripts/write-credits.mjs` });
  }
  for (const hit of handWrittenCredit(html, credits)) {
    out.push({ subject: `text:${hit}`, message: `the page says "${hit}" outside a generated credit line; credit lives in the generated lines only (take it out of the card or heading text)` });
  }
  return out;
}

/** The named references a careless author could hide a credit phrase behind. */
const NAMED_REFERENCES = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'",
  nbsp: ' ', ensp: ' ', emsp: ' ', thinsp: ' ',
  shy: '­', zwnj: '‌', zwj: '‍',
};

/** Text with its character references read, in one pass; unknown ones are kept. */
function decodeReferences(text) {
  return text.replace(/&(?:#(\d+)|#[xX]([0-9a-fA-F]+)|([a-zA-Z]+));?/g, (whole, dec, hex, name) => {
    if (name !== undefined) return Object.hasOwn(NAMED_REFERENCES, name) && whole.endsWith(';') ? NAMED_REFERENCES[name] : whole;
    const cp = dec !== undefined ? Number.parseInt(dec, 10) : Number.parseInt(hex, 16);
    return Number.isFinite(cp) && cp > 0 && cp <= 0x10ffff ? String.fromCodePoint(cp) : whole;
  });
}

// ---------------------------------------------------------------- credit wording

const reEscape = (s) => String(s).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
/** A fixed phrase as a pattern source: its full stop dropped, any run of spaces one space. */
const phraseSource = (s) => reEscape(String(s).trim().replace(/[.\s]+$/, '')).replace(/\s+/g, '\\s+');

/**
 * The clauses of the origin sentences (the sentence a full record and an
 * opened panel start with) that say who made an item: the ones naming the lab
 * or Biorouter ("Original work of the Baranzini Lab.", "the Baranzini Lab
 * wrote the extension around it."). The other clauses ("The program below does
 * the work") credit no one.
 */
export const ORIGIN_CREDIT_CLAUSES = Object.freeze(
  Object.values(ORIGIN_SENTENCES)
    .flatMap((s) => s.split(/(?<=\.)\s+|;\s+/))
    .filter((c) => /baranzini lab|biorouter/i.test(c)),
);

/**
 * Credit wording a page may carry only in a generated element (PV14): the
 * phrases PV13 keeps out of card text (CARD_CREDIT_PATTERN, from the rules
 * file biorouter-skills shares, which knows no extension line), the clause
 * every "Runs" and "Bridges to" extension line ends with (EXTENSION_BY_LAB)
 * and the crediting clauses of the origin sentences. Built from the rules
 * file's own constants, so the words it looks for are the words the
 * generator writes. Case-insensitive.
 */
export const PAGE_CREDIT_PATTERN = new RegExp(
  [CARD_CREDIT_PATTERN.source, ...[EXTENSION_BY_LAB, ...ORIGIN_CREDIT_CLAUSES].map(phraseSource)].join('|'),
  'i',
);

/**
 * The part of a credit line that says what an item comes from and who made
 * it, without the lab's clause, "and N more" or the full stop: "Runs Playwright
 * MCP 0.0.86 by Microsoft Corporation (Apache-2.0)", "Bridges to IDC MCP
 * server by Imaging Data Commons", "From superpowers by Jesse Vincent (MIT)".
 */
export function lineLead(line) {
  return String(line ?? '')
    .split(/;\s+|,\s+packaged for /i)[0]
    .replace(/\s+and \d+ more\.?$/i, '')
    .replace(/[.\s]+$/, '')
    .trim();
}

/**
 * The lead of every generated line in `credits` (lineLead) that the fixed
 * phrases do not already find, lower-cased and deduplicated: a "Runs X by Y"
 * or "From X by Y" written out without the lab's clause still credits X's
 * makers, so it is credit wording too.
 */
export function creditLinePhrases(credits) {
  const out = new Set();
  for (const kind of ['extensions', 'skills']) {
    for (const entry of Object.values(credits?.[kind] ?? {})) {
      const lead = lineLead(entry?.line);
      if (lead.length >= 12 && !PAGE_CREDIT_PATTERN.test(lead)) out.add(lead.replace(/\s+/g, ' ').toLowerCase());
    }
  }
  return [...out].sort();
}

/** PAGE_CREDIT_PATTERN and the given line leads as one global, case-insensitive pattern. */
export function creditWordingPattern(phrases = []) {
  return new RegExp([PAGE_CREDIT_PATTERN.source, ...phrases.map(phraseSource)].join('|'), 'gi');
}

/** Characters a browser shows nothing for: soft hyphens and zero-width characters. */
const INVISIBLE = /[­​-‍⁠﻿]/g;
const shown = (t) => String(t).replace(INVISIBLE, '').replace(/\s+/g, ' ');

/** [{ key, excerpt }] for every match of `re` (global) in the shown text `t`. */
function hitsIn(t, re) {
  const out = [];
  for (const m of t.matchAll(re)) {
    out.push({ key: m[0].replace(/\s+/g, ' ').toLowerCase(), excerpt: t.slice(Math.max(0, m.index - 30), m.index + m[0].length + 40).trim() });
  }
  return out;
}

/** Credit wording in one text as a reader sees it (spaces as one, invisible characters dropped), as excerpts. */
export function creditWording(text, phrases = []) {
  return hitsIn(shown(text), creditWordingPattern(phrases)).map((h) => h.excerpt);
}

/** The elements whose tags join the text either side of them, as a browser lays them out: no space is shown for them. */
const INLINE_TAG = /<\/?(?:a|abbr|b|bdi|bdo|cite|code|data|dfn|em|i|kbd|mark|q|s|samp|small|span|strong|sub|sup|time|u|var|wbr)(?=[\s/>])[^>]*>/gi;
/** Attributes a browser shows or a screen reader announces. */
const SHOWN_ATTRIBUTES = new Set(['title', 'alt', 'aria-label', 'aria-description', 'aria-placeholder', 'placeholder']);
const START_TAG = /<([a-zA-Z][a-zA-Z0-9-]*)((?:"[^"]*"|'[^']*'|[^'">])*)>/g;
const ATTRIBUTE = /([^\s"'<>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;

/**
 * The shown attribute values of every start tag (SHOWN_ATTRIBUTES, and the
 * content of a <meta> whose name or property is a description or a title, which
 * search results and link previews show), references read.
 */
function shownAttributeValues(page) {
  const out = [];
  for (const tag of page.matchAll(START_TAG)) {
    const attrs = {};
    for (const a of tag[2].matchAll(ATTRIBUTE)) {
      const key = a[1].toLowerCase();
      if (!Object.hasOwn(attrs, key)) attrs[key] = a[2] ?? a[3] ?? a[4] ?? '';
    }
    for (const [key, value] of Object.entries(attrs)) if (SHOWN_ATTRIBUTES.has(key)) out.push(value);
    const meta = attrs.name ?? attrs.property ?? '';
    if (tag[1].toLowerCase() === 'meta' && /(?:^|:)(?:description|title)$/i.test(meta)) out.push(attrs.content ?? '');
  }
  return out.map((v) => shown(decodeReferences(v))).filter((v) => v.trim());
}

/**
 * Credit wording (PAGE_CREDIT_PATTERN, and with `credits` the lead of every
 * generated line, creditLinePhrases) anywhere on the page outside the
 * generated elements, as short excerpts. Only an element where the generator
 * puts it is exempt (generatedCreditRanges): an item-credit outside a card, or
 * a curator element outside a shelf or the footer, is read as page text like
 * any other.
 *
 * The page is read as a browser shows it. Comments and scripts are not text.
 * The text is read twice, once with every tag a space and once with inline
 * elements (b, span, a, wbr and the like) joining the text either side, so
 * "Adap<b>ted</b> from" is the phrase it shows; a phrase found only the second
 * way is reported once. The attributes a browser shows or announces (title,
 * alt, aria-label, placeholder, a meta description) are read as well; others
 * (class, data-*) are not. Character references are read as the characters a
 * browser shows ("Adapted&#32;from", "Adapted&nbsp;from" and "Ad&#97;pted
 * from" all show the phrase), in one pass, so "&amp;#32;" stays the literal
 * text "&#32;". Soft hyphens and zero-width characters show nothing and are
 * dropped.
 */
export function handWrittenCredit(html, credits = null) {
  let page = String(html);
  // Overlapping ranges (only on a malformed page) merged, then blanked back to
  // front, so each range's offsets still hold when it is blanked.
  const merged = [];
  for (const [start, end] of generatedCreditRanges(page).sort((a, b) => a[0] - b[0])) {
    const last = merged[merged.length - 1];
    if (last && start < last[1]) last[1] = Math.max(last[1], end);
    else merged.push([start, end]);
  }
  for (const [start, end] of merged.reverse()) page = page.slice(0, start) + ' ' + page.slice(end);
  page = page.replace(/<!--[\s\S]*?-->/g, ' ').replace(/<script\b[\s\S]*?<\/script\s*>/gi, ' ');
  const re = creditWordingPattern(credits ? creditLinePhrases(credits) : []);
  const spaced = hitsIn(shown(decodeReferences(page.replace(/<[^>]+>/g, ' '))), re);
  const joined = hitsIn(shown(decodeReferences(page.replace(INLINE_TAG, '').replace(/<[^>]+>/g, ' '))), re);
  // A phrase found both ways is one phrase: the joined reading adds only the
  // matches beyond what the spaced reading found for the same words.
  const seen = new Map();
  for (const h of spaced) seen.set(h.key, (seen.get(h.key) ?? 0) + 1);
  const extra = joined.filter((h) => {
    const n = seen.get(h.key) ?? 0;
    if (n > 0) seen.set(h.key, n - 1);
    return n === 0;
  });
  const attrs = shownAttributeValues(page).flatMap((v) => hitsIn(v, re));
  return [...spaced, ...extra, ...attrs].map((h) => h.excerpt);
}

/**
 * The string literals of a script, decoded as JavaScript reads them
 * (\n, \xHH, \uHHHH, \u{H...}, a line continuation), comments skipped. Meant
 * for a data file such as landing/baam-details.js, which holds object literals
 * of strings and no regular expressions.
 */
export function scriptStrings(source) {
  const s = String(source ?? '');
  const out = [];
  const SIMPLE = { n: '\n', t: '\t', r: '\r', b: '\b', f: '\f', v: '\v', 0: '\0' };
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === '/' && s[i + 1] === '/') {
      const e = s.indexOf('\n', i);
      i = e === -1 ? s.length : e;
    } else if (c === '/' && s[i + 1] === '*') {
      const e = s.indexOf('*/', i + 2);
      i = e === -1 ? s.length : e + 2;
    } else if (c === '"' || c === "'" || c === '`') {
      let j = i + 1;
      let v = '';
      while (j < s.length && s[j] !== c) {
        if (s[j] === '\\') {
          const n = s[j + 1];
          let m;
          if (n === '\n') j += 2;
          else if (n === '\r') j += s[j + 2] === '\n' ? 3 : 2;
          else if ((m = /^x([0-9a-fA-F]{2})/.exec(s.slice(j + 1, j + 4)))) { v += String.fromCodePoint(Number.parseInt(m[1], 16)); j += 4; }
          else if ((m = /^u([0-9a-fA-F]{4})/.exec(s.slice(j + 1, j + 6)))) { v += String.fromCharCode(Number.parseInt(m[1], 16)); j += 6; }
          else if ((m = /^u\{([0-9a-fA-F]{1,6})\}/.exec(s.slice(j + 1, j + 10))) && Number.parseInt(m[1], 16) <= 0x10ffff) { v += String.fromCodePoint(Number.parseInt(m[1], 16)); j += 1 + m[0].length; }
          else { v += Object.hasOwn(SIMPLE, n) ? SIMPLE[n] : (n ?? ''); j += 2; }
        } else if (c === '`' && s[j] === '$' && s[j + 1] === '{') {
          // An expression in a template literal is code, not text: skipped.
          let depth = 1;
          j += 2;
          while (j < s.length && depth > 0) {
            if (s[j] === '{') depth += 1;
            else if (s[j] === '}') depth -= 1;
            j += 1;
          }
          v += ' ';
        } else if (c !== '`' && s[j] === '\n') {
          break;
        } else {
          v += s[j];
          j += 1;
        }
      }
      out.push(v);
      i = j + 1;
    } else {
      i += 1;
    }
  }
  return out;
}

/**
 * Credit wording in the panel data (landing/baam-details.js), as excerpts:
 * every string there reaches an opened extension panel as text (baam.js
 * escapes it), next to the generated "Provenance and credits" row, so a credit
 * written into one would say who made the item a second time, by hand. Read
 * with the same words as the page (creditWordingPattern); a string shows as
 * written, so no character reference is read.
 */
export function detailsCredit(source, credits = null) {
  const re = creditWordingPattern(credits ? creditLinePhrases(credits) : []);
  return scriptStrings(source).flatMap((v) => hitsIn(shown(v), re)).map((h) => h.excerpt);
}

/**
 * The page with every card's credit line, every shelf curator line and the
 * footer curator line written from `credits`. Returns { html, problems }:
 * problems are sites it cannot write (a card with no description, no id or no
 * entry; a shelf or footer without its one curator element), which it leaves
 * untouched. Writing twice gives the same page.
 */
export function writeCreditsHtml(html, credits) {
  const problems = [];
  const edits = [];
  for (const s of cardCreditSites(html)) {
    const entry = s.id ? entryFor(credits, s.kind, s.id) : null;
    if (!s.id || !entry || s.descEnd === null) {
      problems.push({ subject: `card:${s.kind}:${s.id ?? s.start}`, message: !s.id ? 'no download link' : !entry ? 'no credits entry' : 'no description paragraph' });
      continue;
    }
    // Small, disjoint edits only (a card's span can reach past its own markup,
    // for the last card of a shelf up to the shelf's curator line): the line
    // after the description is written or replaced in place, and any other
    // credit element of the card is removed with its line break and indent.
    // The line kept in place is the one right after the description; one that
    // sits before it (start < descEnd) is never kept, since [descEnd, end)
    // would then run backwards and repeat text.
    const line = `\n${s.indent}<p class="${CARD_CREDIT_CLASS}">${escapeText(entry.line)}</p>`;
    const kept = s.credits.find((c) => c.start >= s.descEnd && html.slice(s.descEnd, c.start).trim() === '');
    edits.push(kept ? [s.descEnd, kept.end, line] : [s.descEnd, s.descEnd, line]);
    for (const c of s.credits) {
      if (c === kept) continue;
      const lineStart = html.lastIndexOf('\n', c.start);
      const from = lineStart !== -1 && html.slice(lineStart, c.start).trim() === '' ? lineStart : c.start;
      edits.push([from, c.end, '']);
    }
  }
  const cur = curatorSites(html);
  const want = escapeText(CURATED_BY);
  for (const id of CURATED_SHELVES) {
    const found = cur.shelves[id];
    if (found.length !== 1) {
      problems.push({ subject: `shelf:${id}`, message: `needs exactly one <p class="shelf-curator"> placed by hand (found ${found.length})` });
      continue;
    }
    edits.push([found[0].start, found[0].end, `<p class="${SHELF_CURATOR_CLASS}">${want}</p>`]);
  }
  if (cur.footer.length !== 1) problems.push({ subject: 'footer', message: `needs exactly one <span class="foot-curator"> placed by hand (found ${cur.footer.length})` });
  else edits.push([cur.footer[0].start, cur.footer[0].end, `<span class="${FOOT_CURATOR_CLASS}">${want}</span>`]);
  // Back to front; at one offset the wider edit goes first, so an insertion
  // there never lands inside text a later edit removes.
  edits.sort((a, b) => b[0] - a[0] || b[1] - a[1]);
  let out = html;
  for (const [start, end, replacement] of edits) out = out.slice(0, start) + replacement + out.slice(end);
  return { html: out, problems };
}
