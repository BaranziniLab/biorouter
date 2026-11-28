#!/usr/bin/env node
// Checks the BAAM catalogs in landing/catalog/ against the page and each other.
//
//   node landing/scripts/check-catalog.mjs
//   node landing/scripts/check-catalog.mjs --skills-repo <biorouter-skills checkout>
//   node landing/scripts/check-catalog.mjs --remote
//   node landing/scripts/check-catalog.mjs --json
//
// The catalogs are the tracking system of docs/extensions/baam-catalog.md:
// extensions.json (every BAAM extension, active or not), skills.json (a mirror
// of biorouter-skills catalog.json at the commit it records in source_commit)
// and jobs.json (the job vocabulary, Biorouter's own jobs and the gap list).
// This check reads them, the committed landing/registry.json and the parts of
// landing/baam.html that registry.json does not carry (skill card versions, the
// Moved and retired list, the hand-written counts), and fails when any of them
// disagree or when a duplicate rule (D1 to D4) or the gap rule (G1) fails.
//
// It also holds the catalog to Biorouter's extension id rename map,
// crates/biorouter/src/config/extension_renames.json (docs/extensions/
// extension-renames.md): a row whose id or manifest_name the map renames
// records the new id in renamed_locally_to and in aliases, and no row records
// a rename the map does not hold.
//
// build-registry.mjs runs the same checkCatalogs() on the registry it has just
// built in memory, before it writes or compares anything, so the published
// outputs can never be regenerated while a catalog disagrees with the page.
//
// --skills-repo also compares the mirror with that checkout's catalog.json at
// source_commit; --remote compares it with the copy on GitHub at that commit.
// No dependencies.

import { execFileSync } from 'node:child_process';
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  DEFAULT_COMPONENT_SIMILARITY_THRESHOLD,
  DEFAULT_GENERIC_WORDS,
  DEFAULT_SIMILARITY_THRESHOLD,
  applyBaseline,
  methodSkillWarnings,
  normalizeName,
  ruleD1,
  ruleD2,
  ruleD3,
  ruleD4,
  ruleG1,
} from './lib/catalog-rules.mjs';

const LANDING = join(dirname(fileURLToPath(import.meta.url)), '..');
export const REPO_ROOT = join(LANDING, '..');
export const CATALOG_DIR = join(LANDING, 'catalog');
const SKILLS_REPO = 'BaranziniLab/biorouter-skills';
const SKILL_DOWNLOAD = (id) => `https://github.com/${SKILLS_REPO}/releases/download/skill-${id}/${id}.zip`;
const SHA = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const SEMVER = /^\d+\.\d+\.\d+(\+[0-9A-Za-z.-]+)?$/;
const GROUPS = new Set(['ucsf', 'tools', 'adapters']);
const CATEGORY_LABEL = { core: 'Core', developer: 'Developer', biomedical: 'Biomedical' };

// ---------------------------------------------------------------- inputs

export function loadCatalogs(dir = CATALOG_DIR) {
  const read = (name) => JSON.parse(readFileSync(join(dir, name), 'utf8'));
  return { extensions: read('extensions.json'), skills: read('skills.json'), jobs: read('jobs.json') };
}

/** The table the Rust crate compiles in; its own tests validate it fully. */
export const RENAMES_FILE = 'crates/biorouter/src/config/extension_renames.json';

/**
 * The extension id rename map, [{ from, to }], read from this repository. Both
 * names are already name_to_key keys. The Rust crate refuses a malformed table
 * in its tests; this only refuses what would make the catalog rule meaningless.
 */
export function loadRenames(repo = REPO_ROOT) {
  const table = JSON.parse(readFileSync(join(repo, RENAMES_FILE), 'utf8'));
  if (table?.version !== 1 || !Array.isArray(table.renames)) {
    throw new Error(`${RENAMES_FILE}: expected "version": 1 and a "renames" list`);
  }
  return table.renames.map((r, i) => {
    if (typeof r?.from !== 'string' || typeof r?.to !== 'string' || !r.from || !r.to) {
      throw new Error(`${RENAMES_FILE}: rename ${i} needs a "from" and a "to"`);
    }
    return { from: r.from, to: r.to };
  });
}

/** Rust's name_to_key: whitespace stripped, lowercased. The rename map is keyed by it. */
const extensionKey = (name) => String(name ?? '').replace(/\s+/g, '').toLowerCase();

/**
 * Biorouter's own skill, bundle and extension names, read from this repository
 * the way biorouter-skills scripts/builtin-names.mjs reads them, so a BAAM item
 * can never take a name Biorouter ships (rule D2).
 */
export function readBuiltins(repo = REPO_ROOT) {
  const text = (p) => readFileSync(join(repo, p), 'utf8');
  const skillsRs = text('crates/biorouter/src/agents/skills_extension.rs');
  const soulRs = text('crates/biorouter/src/knowledge/soul.rs');
  const glyph = text('ui/desktop/src/utils/toolGlyph.ts');
  const arrayNames = (src, ident) => {
    const start = src.indexOf(`pub static ${ident}`);
    if (start === -1) throw new Error(`${ident} not found in skills_extension.rs`);
    const end = src.indexOf('];', start);
    return [...src.slice(start, end).matchAll(/\(\s*"([a-z0-9-]+)",\s*include_str!/g)].map((m) => m[1]);
  };
  const constValue = (src, ident) => {
    const m = src.match(new RegExp(`pub const ${ident}: &str = "([^"]+)"`));
    if (!m) throw new Error(`${ident} not found`);
    return m[1];
  };
  const skills = [
    ...arrayNames(skillsRs, 'BUILTIN_SKILLS'),
    ...arrayNames(skillsRs, 'KNOWLEDGE_SKILLS'),
    constValue(soulRs, 'SOUL_SKILL_DIR'),
  ];
  const dir = join(repo, 'crates/biorouter/src/agents/builtin_skills');
  for (const f of readdirSync(dir).filter((n) => !n.startsWith('.'))) {
    if (!skills.includes(f)) throw new Error(`builtin_skills/${f} is not in BUILTIN_SKILLS or KNOWLEDGE_SKILLS`);
  }
  const keysAt = glyph.indexOf('export const BUILT_IN_EXTENSION_KEYS');
  const labelsAt = glyph.indexOf('const DISPLAY_KEY_ALIASES');
  if (keysAt === -1 || labelsAt === -1) throw new Error('toolGlyph.ts no longer declares BUILT_IN_EXTENSION_KEYS and DISPLAY_KEY_ALIASES');
  const keys = [...glyph.slice(keysAt, glyph.indexOf(']);', keysAt)).matchAll(/'([a-z_]+)'/g)]
    .map((m) => m[1])
    .filter((k) => k !== 'platform' && k !== 'workflow');
  const labels = [...glyph.slice(labelsAt, glyph.indexOf('] as const', labelsAt)).matchAll(/\['([^']+)', '[a-z_]+'\]/g)].map((m) => m[1]);
  return {
    skills: [...new Set(skills)].sort(),
    bundles: [constValue(skillsRs, 'KNOWLEDGE_BUNDLE')],
    extensions: [...new Set([...keys, ...labels])].sort(),
  };
}

const attr = (tag, name) => {
  const m = tag.match(new RegExp(`\\s${name}="([^"]*)"`));
  return m ? m[1] : null;
};

/** data-version of every skill card, keyed by the id its download names. */
export function skillCardVersions(html) {
  const out = new Map();
  for (const m of html.matchAll(/<div class="skill-card(?: [^"]*)?"[^>]*>/g)) {
    const rest = html.slice(m.index, m.index + 6000);
    const dl = rest.match(/releases\/download\/skill-([a-z0-9-]+)\//);
    if (!dl) continue;
    const id = dl[1];
    const featured = /\bis-feat\b/.test(m[0]);
    const v = attr(m[0], 'data-version');
    if (!out.has(id)) out.set(id, []);
    out.get(id).push({ version: v, featured });
  }
  return out;
}

/** The Moved and retired list: one <li data-moved-id> per superseded or retired item. */
export function movedEntries(html) {
  const at = html.indexOf('id="baam-moved"');
  if (at === -1) return null;
  const end = html.indexOf('</details>', at);
  const block = html.slice(at, end === -1 ? undefined : end);
  return [...block.matchAll(/<li\b[^>]*>/g)].map((m) => ({
    id: attr(m[0], 'data-moved-id'),
    kind: attr(m[0], 'data-moved-kind'),
    to: attr(m[0], 'data-moved-to'),
  }));
}

/** The counts the page states before baam.js runs. */
export function pageCounts(html) {
  const num = (id) => {
    const m = html.match(new RegExp(`id="${id}">(\\d+)`));
    return m ? Number(m[1]) : null;
  };
  const packaged = html.match(/<span class="sub-note">(\d+) packaged skills<\/span>/);
  const all = html.match(/<h2>All extensions<\/h2><span class="n">(\d+)<\/span>/);
  return {
    'tab-count-extensions': num('tab-count-extensions'),
    'tab-count-skills': num('tab-count-skills'),
    'count-adapters': num('count-adapters'),
    'count-core': num('count-core'),
    'count-dev': num('count-dev'),
    'count-bio': num('count-bio'),
    'packaged skills note': packaged ? Number(packaged[1]) : null,
    'All extensions': all ? Number(all[1]) : null,
  };
}

// ---------------------------------------------------------------- rule items

export function skillRuleItems(catalog) {
  const items = [];
  for (const row of catalog?.items ?? []) {
    const ref = `skill:${row.id}`;
    items.push({
      ref, catalog: 'skills', kind: row.kind, id: row.id, display_name: row.display_name,
      status: row.status, aliases: row.aliases ?? [], job: row.job, distinct_from: row.distinct_from ?? [],
      text: [row.display_name, row.summary ?? '', ...(row.keywords ?? [])],
    });
    for (const [folder, entry] of Object.entries(row.components ?? {})) {
      items.push({
        ref: `component:${row.id}/${folder}`, catalog: 'skills', kind: 'component', id: `${row.id}/${folder}`,
        name: entry.name ?? folder, display_name: entry.name ?? folder,
        status: row.status === 'active' ? entry.status : row.status === 'retired' ? 'retired' : entry.status,
        aliases: [], job: entry.job, distinct_from: entry.distinct_from ?? [], parent: ref,
        text: [entry.name ?? folder],
      });
    }
  }
  return items;
}

export function extensionRuleItems(catalog) {
  const items = [];
  for (const row of catalog?.items ?? []) {
    const ref = `extension:${row.id}`;
    items.push({
      ref, catalog: 'extensions', kind: 'extension', id: row.id, display_name: row.display_name,
      status: row.status, aliases: row.aliases ?? [], job: row.job, distinct_from: row.distinct_from ?? [],
      text: [row.display_name, row.summary ?? '', ...(row.keywords ?? [])],
    });
    for (const b of row.bundled_skills ?? []) {
      items.push({
        ref: `bundled:${row.id}/${b.name}`, catalog: 'extensions', kind: 'bundled-skill', id: b.name,
        display_name: b.name, status: row.status, aliases: [], job: b.job, distinct_from: [], parent: ref,
        method_skill: b.method_skill ?? null, text: [b.name],
      });
    }
  }
  return items;
}

// ---------------------------------------------------------------- the check

const failure = (rule, subject, message, refs = []) => ({ rule, subject, message, refs });
const sameList = (a, b) => JSON.stringify([...(a ?? [])].sort()) === JSON.stringify([...(b ?? [])].sort());
const statusTarget = (status) => (status.startsWith('superseded_by:') ? status.slice('superseded_by:'.length) : '');

/**
 * The local id renames, held to the rows (docs/extensions/extension-renames.md).
 * A row is renamed when its id or its manifest_name is a rename's `from`; it
 * must then say so in renamed_locally_to and carry the new id in aliases,
 * whatever its status, because the record outlives the turn-on that supersedes
 * the row. A row that states renamed_locally_to must match a rename in the map.
 */
export function renameFailures(extRows, renames) {
  const out = [];
  const f = (sub, row, msg) => out.push(failure('rename', `${sub}:${row.id}`, `extension ${row.id}: ${msg}`, [`extension:${row.id}`]));
  const keysOf = (row) => [...new Set([extensionKey(row.id), extensionKey(row.manifest_name)].filter(Boolean))];
  for (const row of extRows) {
    const keys = keysOf(row);
    const mine = renames.filter((r) => keys.includes(r.from));
    if (mine.length > 1) {
      f('ambiguous', row, `the rename map renames both ${mine.map((r) => r.from).join(' and ')}, which name this one row`);
      continue;
    }
    const [rename] = mine;
    const claimed = row.renamed_locally_to;
    if (rename) {
      if (claimed !== rename.to) {
        f('renamed-to', row, `${RENAMES_FILE} renames ${rename.from} to ${rename.to}, but the row's renamed_locally_to is ${claimed === undefined ? '(absent)' : JSON.stringify(claimed)}; set it to "${rename.to}"`);
      }
      if (!(row.aliases ?? []).includes(rename.to)) {
        f('alias', row, `${RENAMES_FILE} renames ${rename.from} to ${rename.to}, but "${rename.to}" is not in the row's aliases`);
      }
    }
    if (claimed !== undefined && !renames.some((r) => keys.includes(r.from) && r.to === claimed)) {
      f('unmapped', row, `renamed_locally_to is ${JSON.stringify(claimed)}, but ${RENAMES_FILE} holds no rename of ${keys.join(' or ')} to it; add the rename there first`);
    }
  }
  return out;
}

/**
 * registry: { extensions, skills } as build-registry.mjs builds it.
 * html: landing/baam.html. catalogs: loadCatalogs(). builtins: readBuiltins().
 * renames: loadRenames(), read from this repository when left out.
 * Returns { errors, warnings, accepted, counts }.
 */
export function checkCatalogs({ registry, html, catalogs, builtins, renames = loadRenames() }) {
  const raw = [];
  const warnings = [];
  const { extensions: extCat, skills: skillCat, jobs } = catalogs;

  // Shapes.
  if (extCat?.format !== 1) raw.push(failure('shape', 'format:extensions', 'extensions.json must have "format": 1'));
  if (skillCat?.format !== 1) raw.push(failure('shape', 'format:skills', 'skills.json must have "format": 1'));
  if (jobs?.format !== 1) raw.push(failure('shape', 'format:jobs', 'jobs.json must have "format": 1'));
  if (!SHA.test(skillCat?.source_commit ?? '')) {
    raw.push(failure('mirror', 'source-commit', 'skills.json has no source_commit; write it with landing/scripts/sync-skill-catalog.mjs'));
  }
  if (jobs && (jobs.similarity_threshold !== DEFAULT_SIMILARITY_THRESHOLD ||
      jobs.component_similarity_threshold !== DEFAULT_COMPONENT_SIMILARITY_THRESHOLD ||
      JSON.stringify(jobs.generic_words) !== JSON.stringify([...DEFAULT_GENERIC_WORDS]))) {
    raw.push(failure('jobs', 'rules-drift', 'jobs.json similarity_threshold, component_similarity_threshold or generic_words differ from landing/scripts/lib/catalog-rules.mjs; change both together'));
  }
  const extRows = extCat?.items ?? [];
  const skillRows = skillCat?.items ?? [];
  const seen = new Set();
  for (const r of extRows) {
    const s = (sub, msg) => raw.push(failure('row', `${sub}:${r.id}`, `extension ${r.id}: ${msg}`, [`extension:${r.id}`]));
    if (seen.has(r.id)) s('duplicate', 'two rows have this id');
    seen.add(r.id);
    if (!GROUPS.has(r.group)) s('group', `group "${r.group}" is not ucsf, tools or adapters`);
    if (!SEMVER.test(r.version ?? '')) s('version', `version "${r.version}" is not semver`);
    if (!Array.isArray(r.history) || r.history.at(-1)?.version !== r.version) s('history', `the last history entry is not the row's version ${r.version}`);
    if (r.status === 'retired' && !r.retired?.reason) s('retired', 'a retired row says why in retired.reason');
    if (r.status === 'active') {
      const rel = r.release ?? {};
      if (!rel.legacy_tag && rel.tag !== `v${r.version}`) s('release-tag', `release.tag must be "v${r.version}" (tags are plain vX.Y.Z)`);
      if (!SHA256.test(rel.sha256 ?? '')) s('release-sha', 'release.sha256 is missing; take it from the release SHA256SUMS');
      if (r.privacy !== 'public' && r.privacy !== 'private') s('privacy', `privacy "${r.privacy}" is not public or private`);
    }
  }
  raw.push(...renameFailures(extRows, renames));

  // Registry ↔ catalog: extensions.
  const cards = new Map((registry?.extensions ?? []).map((e) => [e.id, e]));
  const activeExt = extRows.filter((r) => r.status === 'active');
  for (const e of registry?.extensions ?? []) {
    const r = extRows.find((x) => x.id === e.id);
    if (!r || r.status !== 'active') {
      raw.push(failure('card', `ext-unlisted:${e.id}`, `the card ${e.id} has no active row in landing/catalog/extensions.json`));
    }
  }
  for (const r of activeExt) {
    const e = cards.get(r.id);
    const f = (sub, msg) => raw.push(failure('card', `${sub}:${r.id}`, `extension ${r.id}: ${msg}`, [`extension:${r.id}`]));
    if (!e) {
      f('ext-missing', 'is active in the catalog but has no card in landing/baam.html');
      continue;
    }
    if (e.name !== r.display_name) f('name', `the card says "${e.name}", the catalog "${r.display_name}"`);
    const adapters = /BRXT/i.test(e.organization ?? '');
    const group = adapters ? 'adapters' : (e.affiliation ?? []).includes('ucsf') ? 'ucsf' : 'tools';
    if (group !== r.group) f('group', `the card sits in ${group}, the catalog says ${r.group}`);
    if (e.license !== r.licence) f('licence', `the card says ${e.license}, the catalog ${r.licence}`);
    if (e.privacy !== r.privacy) f('privacy', `the card says ${e.privacy}, the catalog ${r.privacy}`);
    const cardAff = e.affiliation ?? null;
    if (JSON.stringify(cardAff) !== JSON.stringify(r.affiliation ?? null)) {
      f('affiliation', `the card declares ${JSON.stringify(cardAff)}, the catalog ${JSON.stringify(r.affiliation ?? null)}`);
    }
    if (e.version !== `v${r.version}`) f('version', `the card shows ${e.version}, the catalog ${r.version}`);
    const rel = r.release ?? {};
    const want = `https://github.com/${r.repository}/releases/download/${encodeURIComponent(rel.tag ?? '')}/${rel.asset}`;
    if (e.download !== want) f('download', `the card downloads ${e.download}, the catalog names ${want}`);
    if ((r.summary ?? '') !== e.description) f('summary', 'summary differs from the card description (run the catalog sync in docs/extensions/baam-catalog.md)');
    if (!sameList(r.keywords, e.tags)) f('keywords', 'keywords differ from the card tags');
  }

  // Registry ↔ catalog: skills.
  const skillCards = new Map((registry?.skills ?? []).map((s) => [s.id, s]));
  const versions = skillCardVersions(html);
  for (const s of registry?.skills ?? []) {
    const r = skillRows.find((x) => x.id === s.id);
    if (!r || r.status !== 'active') {
      raw.push(failure('card', `skill-unlisted:${s.id}`, `the skill card ${s.id} has no active row in landing/catalog/skills.json`));
    }
  }
  for (const r of skillRows.filter((x) => x.status === 'active')) {
    const s = skillCards.get(r.id);
    const f = (sub, msg) => raw.push(failure('card', `${sub}:${r.id}`, `skill ${r.id}: ${msg}`, [`skill:${r.id}`]));
    if (!s) {
      f('skill-missing', 'is active in the catalog but has no card in landing/baam.html');
      continue;
    }
    if (normalizeName(s.name) !== normalizeName(r.display_name) && !(r.aliases ?? []).includes(s.name)) {
      f('name', `the card says "${s.name}", the catalog "${r.display_name}"`);
    }
    if (s.category !== CATEGORY_LABEL[r.category]) f('category', `the card sits in ${s.category}, the catalog says ${r.category}`);
    if (s.license !== r.licence) f('licence', `the card says ${s.license}, the catalog ${r.licence}`);
    if (s.download !== SKILL_DOWNLOAD(r.id)) f('download', `the card downloads ${s.download}, expected ${SKILL_DOWNLOAD(r.id)}`);
    for (const v of versions.get(r.id) ?? []) {
      if (v.version !== r.version) {
        f(v.featured ? 'featured-version' : 'version', `the ${v.featured ? 'featured ' : ''}card's data-version is ${v.version ?? '(absent)'}, the catalog ${r.version}`);
      }
    }
  }

  // Redirects on the page (D4's page half).
  const byExt = new Map(extRows.map((r) => [r.id, r]));
  for (const r of extRows.filter((x) => x.status.startsWith('superseded_by:'))) {
    const t = cards.get(statusTarget(r.status));
    if (t && !t.description.includes(`Replaces ${r.display_name}`)) {
      raw.push(failure('D4', `card-names-old:${r.id}`, `the card of ${t.id} replaces ${r.display_name} but its description does not say "Replaces ${r.display_name}"`, [`extension:${r.id}`]));
    }
  }
  for (const r of skillRows.filter((x) => x.status.startsWith('superseded_by:') && !x.status.startsWith('superseded_by:builtin:'))) {
    const t = skillCards.get(statusTarget(r.status));
    if (t && !(t.keywords ?? []).includes(r.id)) {
      raw.push(failure('D4', `card-tags-old:${r.id}`, `the card of ${t.id} replaces ${r.id} but its data-tags do not carry "${r.id}", so a search for the old name misses it`, [`skill:${r.id}`]));
    }
  }

  // The Moved and retired list.
  const moved = movedEntries(html);
  const inactive = [
    ...extRows.filter((r) => r.status !== 'active').map((r) => ({ kind: 'extension', r })),
    ...skillRows.filter((r) => r.status !== 'active').map((r) => ({ kind: 'skill', r })),
  ];
  if (moved === null) {
    raw.push(failure('moved', 'missing', 'landing/baam.html has no <details id="baam-moved"> list'));
  } else {
    const key = (kind, id) => `${kind}:${id}`;
    const listed = new Map();
    for (const m of moved) {
      const k = key(m.kind, m.id);
      listed.set(k, (listed.get(k) ?? 0) + 1);
    }
    for (const { kind, r } of inactive) {
      const k = key(kind, r.id);
      const n = listed.get(k) ?? 0;
      if (n !== 1) raw.push(failure('moved', `count:${k}`, `${k} is ${r.status} and must appear exactly once in the Moved and retired list (it appears ${n} times)`));
      const entry = moved.find((m) => m.kind === kind && m.id === r.id);
      if (entry && (entry.to ?? '') !== statusTarget(r.status)) {
        raw.push(failure('moved', `target:${k}`, `${k}: the moved list points at "${entry.to}", the catalog at "${statusTarget(r.status)}"`));
      }
    }
    const want = new Set(inactive.map(({ kind, r }) => key(kind, r.id)));
    for (const m of moved) {
      if (!want.has(key(m.kind, m.id))) raw.push(failure('moved', `extra:${key(m.kind, m.id)}`, `the Moved and retired list names ${key(m.kind, m.id)}, which the catalogs do not list as superseded or retired`));
    }
  }

  // The hand-written counts.
  const adapters = activeExt.filter((r) => r.group === 'adapters').length;
  const activeSkills = skillRows.filter((r) => r.status === 'active');
  const count = (cat) => activeSkills.filter((r) => r.category === cat).length;
  const packaged = activeSkills
    .filter((r) => r.category === 'biomedical')
    .reduce((n, r) => n + Object.values(r.components ?? {}).filter((c) => c.status === 'active').length, 0);
  const counts = {
    'tab-count-extensions': activeExt.length,
    'All extensions': activeExt.length,
    'tab-count-skills': activeSkills.length,
    'count-adapters': adapters,
    'count-core': count('core'),
    'count-dev': count('developer'),
    'count-bio': count('biomedical'),
    'packaged skills note': packaged,
  };
  const onPage = pageCounts(html);
  for (const [k, v] of Object.entries(counts)) {
    if (onPage[k] !== v) raw.push(failure('count', k, `landing/baam.html states ${k} = ${onPage[k]}, the catalogs give ${v}`));
  }

  // The duplicate rules and the gap rule, across both catalogs.
  const all = [...skillRuleItems(skillCat), ...extensionRuleItems(extCat)];
  raw.push(...ruleD1(all, { builtinJobs: jobs?.builtin ?? [] }));
  const ms = methodSkillWarnings(all);
  raw.push(...ms.failures);
  warnings.push(...ms.warnings.filter((w) => w.startsWith('bundled:')));
  // A retired item's id stays reserved (D2), with one written exception: a
  // retired row may pass its name to the active item in the other catalog that
  // took over its job (UCSFHPCAgent's name to the ucsf-hpc skill), stated as
  // retired.name_passes_to on the retired row.
  const rowOf = (ref) => {
    const [kind, id] = ref.split(':');
    return kind === 'extension' ? extRows.find((r) => r.id === id) : kind === 'skill' ? skillRows.find((r) => r.id === id) : null;
  };
  raw.push(
    ...ruleD2(all, builtins).filter((f) => {
      const owners = [...new Set(f.refs)];
      if (owners.length !== 2) return true;
      const [a, b] = owners.map(rowOf);
      if (!a || !b) return true;
      const passes = (old, ref) => old.status !== 'active' && old.retired?.name_passes_to === ref;
      return !(passes(a, owners[1]) && b.status === 'active') && !(passes(b, owners[0]) && a.status === 'active');
    })
  );
  // D3 within skills belongs to biorouter-skills, which reads each SKILL.md;
  // the extensions are screened here.
  raw.push(...ruleD3(all.filter((i) => i.kind === 'extension'), {
    threshold: jobs?.similarity_threshold ?? DEFAULT_SIMILARITY_THRESHOLD,
    genericWords: jobs?.generic_words ?? DEFAULT_GENERIC_WORDS,
  }));
  raw.push(...ruleD4(all, builtins));
  const g = ruleG1(all, jobs?.required ?? []);
  raw.push(...g.failures);
  warnings.push(...g.warnings);
  for (const r of extRows) {
    for (const b of r.overlaps_builtin ?? []) {
      if (!b?.builtin || !String(b.boundary ?? '').trim()) {
        raw.push(failure('D1', `overlap-boundary:${r.id}`, `extension ${r.id}: an overlaps_builtin entry needs a builtin and a boundary`));
      }
    }
  }
  if (!byExt.size) raw.push(failure('shape', 'empty:extensions', 'extensions.json lists no extensions'));

  const baseline = [...(extCat?.baseline_violations ?? []), ...(skillCat?.baseline_violations ?? [])];
  const done = [...(extCat?.done_work_packages ?? []), ...(skillCat?.done_work_packages ?? [])];
  const result = applyBaseline(raw, baseline, done);
  return { errors: result.errors, warnings, accepted: result.accepted, counts };
}

/** The catalog.json bytes skills.json mirrors: skills.json without its source_commit line. */
export function mirroredBytes(skillsJsonText) {
  return skillsJsonText.replace(/^ {2}"source_commit": "[0-9a-f]{40}",\n/m, '');
}

function compareMirror(label, upstream, local) {
  if (upstream === mirroredBytes(local)) return [];
  return [failure('mirror', label, `landing/catalog/skills.json differs from ${label}; run landing/scripts/sync-skill-catalog.mjs`)];
}

// ---------------------------------------------------------------- CLI

async function main(argv) {
  const opt = (name) => {
    const i = argv.indexOf(name);
    return i === -1 ? null : argv[i + 1];
  };
  const dir = resolve(opt('--catalog') ?? CATALOG_DIR);
  const registry = JSON.parse(readFileSync(resolve(opt('--registry') ?? join(LANDING, 'registry.json')), 'utf8'));
  const html = readFileSync(resolve(opt('--html') ?? join(LANDING, 'baam.html')), 'utf8');
  const catalogs = loadCatalogs(dir);
  const res = checkCatalogs({ registry, html, catalogs, builtins: readBuiltins(REPO_ROOT) });
  const skillsText = readFileSync(join(dir, 'skills.json'), 'utf8');
  const commit = catalogs.skills.source_commit;
  const repo = opt('--skills-repo');
  if (repo) {
    if (!existsSync(join(repo, '.git'))) throw new Error(`${repo} is not a git checkout`);
    const upstream = execFileSync('git', ['-C', repo, 'show', `${commit}:catalog.json`], { encoding: 'utf8' });
    res.errors.push(...compareMirror(`${repo} catalog.json at ${commit}`, upstream, skillsText));
  }
  if (argv.includes('--remote')) {
    const url = `https://raw.githubusercontent.com/${SKILLS_REPO}/${commit}/catalog.json`;
    const resp = await fetch(url);
    if (!resp.ok) res.errors.push(failure('mirror', 'remote', `could not fetch ${url}: HTTP ${resp.status}`));
    else res.errors.push(...compareMirror(url, await resp.text(), skillsText));
  }
  if (argv.includes('--json')) {
    console.log(JSON.stringify({ ok: res.errors.length === 0, ...res }, null, 2));
  } else {
    const ext = catalogs.extensions.items;
    const sk = catalogs.skills.items;
    console.log(
      `catalog: ${ext.length} extension rows (${ext.filter((r) => r.status === 'active').length} active), ` +
        `${sk.length} skill rows (${sk.filter((r) => r.status === 'active').length} active), skills at ${commit?.slice(0, 7)}`
    );
    if (res.accepted.length) console.log(`baseline violations still open: ${res.accepted.length}`);
    for (const w of res.warnings) console.log(`warning: ${w}`);
    for (const e of res.errors) console.log(`FAIL [${e.rule}] ${e.message}`);
    console.log(res.errors.length ? `${res.errors.length} failure(s)` : 'ok');
  }
  return res.errors.length ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (e) => {
      console.error(e.stack || e.message);
      process.exit(2);
    }
  );
}
