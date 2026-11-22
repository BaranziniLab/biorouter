// The BAAM duplicate rules (D1 to D6) and the gap rule (G1).
//
// This module is shared by two checks and must stay free of file system and
// network access: biorouter-skills `scripts/check-catalog.mjs` (skills, their
// bundle components and, when given the extension catalog and jobs.json, the
// cross-kind rules) and the biorouter repository's `landing/scripts/` checks,
// which copy this file verbatim. Every rule takes plain items and returns
// failures; the callers decide where the items come from.
//
// An item is
//   { ref, catalog, kind, id, display_name, status, aliases, job, distinct_from,
//     text, parent, method_skill }
// where `ref` is unique ("skill:<id>", "extension:<id>",
// "component:<bundle>/<folder>", "bundled:<extension>/<skill>"), `catalog` is
// "skills" or "extensions", `kind` is skill | bundle | extension | component |
// bundled-skill, `text` is the list of strings D3 compares, and `parent` is the
// ref of the bundle or extension that holds a component or bundled skill.
//
// A failure is { rule, subject, message, refs }. `rule` and `subject` together
// are the key a `baseline_violations` entry names.

export const RULES_VERSION = 1;

// Calibrated against the baseline catalogs in November 2026 (see
// scripts/calibration.test.mjs in biorouter-skills). Card text is short, so
// token-set Jaccard scores are low: the fifth-highest of the seven known
// look-alike pairs scores 2/17 (0.118), and the plan's starting value of 0.40
// flagged one of them. 0.115 flags five of the seven; the two it misses share a
// job with their partner in the baseline, so D1 catches them. jobs.json freezes
// the same number, and the check fails when the two differ.
export const DEFAULT_SIMILARITY_THRESHOLD = 0.115;

// D5 screens bundle components with the plan's starting value. Component
// descriptions are longer and written from one template, so at 0.115 the
// screen flags several hundred sibling pairs (627 at 0.15 in the baseline);
// at 0.40 it flags the three cross-bundle pairs that are worth a sentence.
export const DEFAULT_COMPONENT_SIMILARITY_THRESHOLD = 0.4;

// Words too common in BAAM card text to tell two items apart. The first eight
// come from the plan; the rest are the words that appear in more than about
// ten percent of baseline items, frozen here so the screen does not drift as
// the catalog grows.
export const DEFAULT_GENERIC_WORDS = Object.freeze([
  'skill', 'agent', 'biorouter', 'analysis', 'workflow', 'data', 'research', 'tool',
  'apache', 'mcp', 'plan', 'planning', 'review', 'style', 'user', 'check', 'access',
  'guarded', 'metadata', 'scientific', 'design', 'model', 'report', 'provenance',
  'structure', 'api', 'adapter', 'file', 'read', 'call', 'run', 'project', 'across',
  'work', 'task', 'ask', 'command', 'request', 'handle', 'system', 'result', 'support',
  'help', 'need', 'want', 'make', 'build', 'create', 'write', 'include', 'provide',
]);

// English stopwords (the common NLTK list plus a few connective words that
// card text uses). Words shorter than three letters are dropped before this.
export const STOPWORDS = Object.freeze(new Set(`
about above after again against all also and any are aren arent because been before being below
between both but can cannot could couldn did didn does doesn doing don down during each either
else etc every few for from further had hadn has hasn have haven having her here hers herself him
himself his how into isn its itself just let like may might mightn more most must mustn myself
needn neither nor not now off once one only onto other ought our ours ourselves out over own per
same shan she should shouldn since some such than that the their theirs them themselves then there
these they this those though through thus too two under until upon very via was wasn were weren
what when where whether which while who whom whose why will with within without won would wouldn
yet you your yours yourself yourselves use used uses using
`.trim().split(/\s+/)));

/** Lowercase, letters and digits only, a trailing "agent" removed (rule D2). */
export function normalizeName(value) {
  const flat = String(value ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');
  return flat.length > 5 && flat.endsWith('agent') ? flat.slice(0, -5) : flat;
}

/** Plural folding only; deterministic and documented, so a score can be reproduced by hand. */
export function foldPlural(word) {
  if (word.length > 4 && word.endsWith('ies')) return `${word.slice(0, -3)}y`;
  if (word.length > 4 && /(ss|x|ch|sh)es$/.test(word)) return word.slice(0, -2);
  if (word.length > 3 && word.endsWith('s') && !/(ss|us|is)$/.test(word)) return word.slice(0, -1);
  return word;
}

export function tokenSet(parts, genericWords = DEFAULT_GENERIC_WORDS) {
  const generic = new Set([...genericWords].map(foldPlural));
  const out = new Set();
  for (const part of parts) {
    for (const raw of String(part ?? '').toLowerCase().match(/[a-z]+/g) ?? []) {
      if (raw.length < 3 || STOPWORDS.has(raw)) continue;
      const w = foldPlural(raw);
      if (generic.has(w) || STOPWORDS.has(w)) continue;
      out.add(w);
    }
  }
  return out;
}

export function jaccard(a, b) {
  let inter = 0;
  for (const x of a) if (b.has(x)) inter++;
  const union = a.size + b.size - inter;
  return union === 0 ? 0 : inter / union;
}

const isActive = (item) => item.status === 'active';
const pairKey = (a, b) => [a, b].sort().join('|');

function lists(item, otherId) {
  return (item.distinct_from ?? []).some(
    (d) => d && d.id === otherId && typeof d.boundary === 'string' && d.boundary.trim().length > 0,
  );
}

/** True when either item records a written boundary with the other. */
export function hasBoundary(a, b) {
  return lists(a, b.id) || lists(b, a.id);
}

// ---------------------------------------------------------------------------
// D1, one job, one owner.

export const JOB_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*(?:\.[a-z0-9]+(?:-[a-z0-9]+)*)+$/;

export function ruleD1(items, { builtinJobs = [] } = {}) {
  const failures = [];
  const builtin = new Map(builtinJobs.map((b) => [b.job, b.owner]));
  const byJob = new Map();
  for (const item of items.filter(isActive)) {
    if (!item.job || !JOB_PATTERN.test(item.job)) {
      failures.push({ rule: 'D1', subject: `job-format:${item.ref}`, refs: [item.ref],
        message: `${item.ref} has no job key, or "${item.job}" is not <domain>.<task> in lowercase` });
      continue;
    }
    if (item.kind === 'bundled-skill') {
      if (!item.job.endsWith('.tool-usage')) {
        failures.push({ rule: 'D1', subject: `tool-usage:${item.ref}`, refs: [item.ref],
          message: `${item.ref} is a bundled adapter skill, so its job must end in ".tool-usage" (it is "${item.job}")` });
      }
    }
    if (builtin.has(item.job)) {
      failures.push({ rule: 'D1', subject: `builtin:${item.job}`, refs: [item.ref],
        message: `${item.ref} claims "${item.job}", which Biorouter itself owns (${builtin.get(item.job)})` });
    }
    if (!byJob.has(item.job)) byJob.set(item.job, []);
    byJob.get(item.job).push(item.ref);
  }
  for (const [job, refs] of byJob) {
    if (refs.length > 1) {
      failures.push({ rule: 'D1', subject: `job:${job}`, refs,
        message: `${refs.length} active items share the job "${job}": ${refs.join(', ')}. One of them merges into the other.` });
    }
  }
  return failures;
}

/** Bundled adapter skills must name an active method skill (or null, which warns). */
export function methodSkillWarnings(items) {
  const active = new Set(items.filter((i) => isActive(i) && (i.kind === 'skill' || i.kind === 'bundle')).map((i) => i.id));
  const failures = [];
  const warnings = [];
  for (const item of items.filter((i) => isActive(i) && i.kind === 'bundled-skill')) {
    if (item.method_skill == null) {
      warnings.push(`${item.ref} names no method_skill`);
    } else if (!active.has(item.method_skill)) {
      failures.push({ rule: 'D1', subject: `method-skill:${item.ref}`, refs: [item.ref],
        message: `${item.ref} names method_skill "${item.method_skill}", which is not an active BAAM skill` });
    }
  }
  return { failures, warnings };
}

// ---------------------------------------------------------------------------
// D2, one name.

/**
 * builtins: { skills: [...names], bundles: [...], extensions: [...names or labels] }
 * The namespace holds every row's id (whatever its status, so a retired id is
 * never reused), and for active items their display names and aliases, plus
 * bundled skill names of active extensions and every Biorouter built-in name.
 */
export function ruleD2(items, builtins = {}) {
  const entries = new Map();
  const add = (name, owner, what) => {
    const n = normalizeName(name);
    if (!n) return;
    if (!entries.has(n)) entries.set(n, []);
    entries.get(n).push({ owner, what, raw: name });
  };
  const byRef = new Map(items.map((i) => [i.ref, i]));
  for (const item of items) {
    if (item.kind === 'component') continue; // D5 covers component names.
    if (item.kind === 'bundled-skill') {
      if (isActive(item)) add(item.id, item.ref, 'bundled skill name');
      continue;
    }
    add(item.id, item.ref, 'id');
    if (!isActive(item)) continue;
    add(item.display_name, item.ref, 'display name');
    for (const alias of item.aliases ?? []) add(alias, item.ref, 'alias');
  }
  for (const n of builtins.skills ?? []) add(n, `builtin-skill:${n}`, 'built-in skill');
  for (const n of builtins.bundles ?? []) add(n, `builtin-skill:${n}`, 'built-in skill bundle');
  for (const n of builtins.extensions ?? []) add(n, `builtin-extension:${normalizeName(n)}`, 'built-in extension');

  const failures = [];
  for (const [name, list] of entries) {
    const owners = [...new Set(list.map((e) => e.owner))];
    if (owners.length < 2) continue;
    const ownerItems = owners.map((o) => byRef.get(o)).filter(Boolean);
    const builtinOwners = owners.filter((o) => o.startsWith('builtin-'));
    // A built-in skill and a built-in extension of one name are Biorouter's own business.
    if (ownerItems.length === 0) continue;
    // An extension and the skill it bundles may share a name: they install together.
    if (ownerItems.length === 2 && !builtinOwners.length) {
      const [a, b] = ownerItems;
      if (a.parent === b.ref || b.parent === a.ref) continue;
    }
    // The one exception in the plan: an alias that equals the id of an item
    // superseded by the item carrying the alias.
    const aliasException = (() => {
      if (ownerItems.length !== 2 || builtinOwners.length) return false;
      const [a, b] = ownerItems;
      const check = (carrier, old) =>
        list.some((e) => e.owner === carrier.ref && e.what === 'alias') &&
        list.some((e) => e.owner === old.ref && e.what === 'id') &&
        old.status === `superseded_by:${carrier.id}`;
      return check(a, b) || check(b, a);
    })();
    if (aliasException) continue;
    // A retired or superseded row keeps its id reserved; that id may equal a
    // built-in (the reason develop-biorouter-extension was retired).
    if (ownerItems.length === 1 && !isActive(ownerItems[0]) && builtinOwners.length) continue;
    // An inactive row's display name was never registered, so only ids clash here.
    // Two catalogs: a skill and an extension that share a normalized name may
    // stand together only with a written boundary (see the plan's D2 note).
    if (ownerItems.length === 2 && !builtinOwners.length) {
      const [a, b] = ownerItems;
      if (a.catalog !== b.catalog && isActive(a) && isActive(b) && a.id !== b.id && hasBoundary(a, b)) continue;
    }
    failures.push({ rule: 'D2', subject: `name:${name}`, refs: owners,
      message: `the name "${name}" is used by ${list.map((e) => `${e.owner} (${e.what} "${e.raw}")`).join(', ')}` });
  }
  return failures;
}

// ---------------------------------------------------------------------------
// D3, look-alike screen (and D5's component screen, which uses the same code).

export function lookalikeScores(items, { genericWords = DEFAULT_GENERIC_WORDS } = {}) {
  const active = items.filter(isActive);
  const sets = new Map(active.map((i) => [i.ref, tokenSet(i.text ?? [], genericWords)]));
  const out = [];
  for (let x = 0; x < active.length; x++) {
    for (let y = x + 1; y < active.length; y++) {
      const a = active[x];
      const b = active[y];
      if (a.catalog !== b.catalog || (a.kind === 'component') !== (b.kind === 'component')) continue;
      out.push({ a, b, score: jaccard(sets.get(a.ref), sets.get(b.ref)) });
    }
  }
  return out.sort((p, q) => q.score - p.score || pairKey(p.a.ref, p.b.ref).localeCompare(pairKey(q.a.ref, q.b.ref)));
}

export function ruleD3(items, { threshold = DEFAULT_SIMILARITY_THRESHOLD, genericWords, rule = 'D3' } = {}) {
  return lookalikeScores(items, { genericWords })
    .filter((p) => p.score >= threshold && !hasBoundary(p.a, p.b))
    .map((p) => ({
      rule,
      subject: `pair:${pairKey(p.a.id, p.b.id)}`,
      refs: [p.a.ref, p.b.ref],
      score: p.score,
      message: `${p.a.ref} and ${p.b.ref} look alike (Jaccard ${p.score.toFixed(3)} >= ${threshold}); merge them, or record a distinct_from boundary`,
    }));
}

// ---------------------------------------------------------------------------
// D4, redirects are complete.

export const STATUS_PATTERN = /^(active|retired|superseded_by:(builtin:)?[a-z0-9][a-z0-9-]*(\/[a-z0-9][a-z0-9-]*)?)$/;
// A bundle component may also be "removed" (dropped upstream, nothing replaces it).
export const COMPONENT_STATUS_PATTERN = /^(active|removed|superseded_by:[a-z0-9][a-z0-9-]*(\/[a-z0-9][a-z0-9-]*)?)$/;

export function ruleD4(items, builtins = {}) {
  const failures = [];
  const rows = items.filter((i) => i.kind !== 'component' && i.kind !== 'bundled-skill');
  const byId = new Map(rows.map((i) => [`${i.catalog}:${i.id}`, i]));
  // A component is addressed as <bundle>/<folder> or as <bundle>/<component name>.
  const components = new Map();
  for (const i of items.filter((x) => x.kind === 'component')) {
    components.set(i.id, i);
    if (i.name) components.set(`${i.id.split('/')[0]}/${i.name}`, i);
  }
  const builtinSkills = new Set([...(builtins.skills ?? []), ...(builtins.bundles ?? [])]);
  const builtinExtensions = new Set((builtins.extensions ?? []).map(normalizeName));
  for (const item of items) {
    const pattern = item.kind === 'component' ? COMPONENT_STATUS_PATTERN : STATUS_PATTERN;
    if (!pattern.test(item.status ?? '')) {
      failures.push({ rule: 'D4', subject: `status:${item.ref}`, refs: [item.ref],
        message: `${item.ref} has status "${item.status}", which is not active, retired or superseded_by:<id>` });
      continue;
    }
    if (!item.status.startsWith('superseded_by:')) continue;
    const target = item.status.slice('superseded_by:'.length);
    if (target.startsWith('builtin:')) {
      const name = target.slice('builtin:'.length);
      const ok = item.catalog === 'skills' ? builtinSkills.has(name) : builtinExtensions.has(normalizeName(name));
      if (!ok) {
        failures.push({ rule: 'D4', subject: `target:${item.ref}`, refs: [item.ref],
          message: `${item.ref} is superseded by built-in "${name}", which Biorouter does not ship` });
      }
      continue;
    }
    if (item.kind === 'component') {
      // A removed component points at another component (<bundle>/<folder>) or a whole item.
      const t = target.includes('/') ? components.get(target) : byId.get(`skills:${target}`);
      if (!t || !isActive(t)) {
        failures.push({ rule: 'D4', subject: `target:${item.ref}`, refs: [item.ref],
          message: `${item.ref} is superseded by "${target}", which is not an active component or skill` });
      }
      continue;
    }
    const t = byId.get(`${item.catalog}:${target}`);
    if (!t) {
      failures.push({ rule: 'D4', subject: `target:${item.ref}`, refs: [item.ref],
        message: `${item.ref} is superseded by "${target}", which has no row in the ${item.catalog} catalog` });
      continue;
    }
    if (!isActive(t)) {
      failures.push({ rule: 'D4', subject: `chain:${item.ref}`, refs: [item.ref, t.ref],
        message: `${item.ref} is superseded by ${t.ref}, which is not active (no chains, no cycles)` });
      continue;
    }
    if (!(t.aliases ?? []).includes(item.id)) {
      failures.push({ rule: 'D4', subject: `alias:${item.ref}`, refs: [item.ref, t.ref],
        message: `${t.ref} replaces ${item.ref} but does not list "${item.id}" in aliases` });
    }
  }
  return failures;
}

// ---------------------------------------------------------------------------
// D5, components: unique names across bundles and single skills, and the look-alike screen.

export function ruleD5(items, { threshold = DEFAULT_COMPONENT_SIMILARITY_THRESHOLD, genericWords, builtins = {}, screen = true } = {}) {
  const failures = [];
  const names = new Map();
  const add = (name, owner) => {
    const n = normalizeName(name);
    if (!names.has(n)) names.set(n, new Set());
    names.get(n).add(owner);
  };
  for (const item of items) {
    if (item.catalog !== 'skills' && item.kind !== 'bundled-skill') continue;
    if (!isActive(item)) continue;
    if (item.kind === 'component') add(item.name ?? item.id, item.ref);
    else if (item.kind === 'skill') add(item.id, item.ref);
    else if (item.kind === 'bundled-skill') add(item.id, item.ref);
  }
  for (const n of [...(builtins.skills ?? [])]) add(n, `builtin-skill:${n}`);
  for (const [name, owners] of names) {
    const list = [...owners];
    if (list.length > 1 && list.some((o) => o.startsWith('component:'))) {
      failures.push({ rule: 'D5', subject: `name:${name}`, refs: list,
        message: `the component name "${name}" is also used by ${list.join(', ')}` });
    }
  }
  if (screen) {
    const comps = items.filter((i) => i.kind === 'component');
    failures.push(...ruleD3(comps, { threshold, genericWords, rule: 'D5' }));
  }
  return failures;
}

// ---------------------------------------------------------------------------
// D6, no empty shells.

/** The lines the detector compares: trimmed, non-empty, and not a level-one heading. */
export function shellLines(body) {
  return String(body ?? '')
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('# '));
}

/**
 * bodies: [{ ref, id, body }] for every active single skill. A skill fails when
 * more than half of its lines appear in five or more other skills.
 *
 * isTemplate(line), when given, also counts a line as shared. The check passes
 * the template lines frozen from the baseline, because the frequency alone
 * forgets a template once enough of the shells built from it become stubs: a
 * shell must keep failing until it is rebuilt, not pass because its siblings
 * were merged away.
 */
export function ruleD6(bodies, { minOthers = 5, maxShare = 0.5, isTemplate = () => false } = {}) {
  const lines = new Map(bodies.map((b) => [b.ref, shellLines(b.body)]));
  const freq = new Map();
  for (const ls of lines.values()) for (const l of new Set(ls)) freq.set(l, (freq.get(l) ?? 0) + 1);
  const failures = [];
  for (const b of bodies) {
    const ls = lines.get(b.ref);
    if (!ls.length) {
      failures.push({ rule: 'D6', subject: `shell:${b.id}`, refs: [b.ref], share: 1, message: `${b.ref} has an empty body` });
      continue;
    }
    const shared = ls.filter((l) => freq.get(l) >= minOthers + 1 || isTemplate(l)).length;
    const share = shared / ls.length;
    if (share > maxShare) {
      failures.push({ rule: 'D6', subject: `shell:${b.id}`, refs: [b.ref], share,
        message: `${b.ref} is a template shell: ${shared} of its ${ls.length} lines (${Math.round(share * 100)}%) appear in ${minOthers} or more other skills or in the baseline template` });
    }
  }
  return failures;
}

// ---------------------------------------------------------------------------
// G1, every required job has an active owner or a plan.

/** required: [{ job, why, owner, planned }]. Owner is an item id, "<bundle>/<component>" or "<extension>/<bundled skill>". */
export function ruleG1(items, required = []) {
  const failures = [];
  const warnings = [];
  const activeById = new Map();
  for (const i of items.filter(isActive)) {
    const key = i.kind === 'component' || i.kind === 'bundled-skill' ? i.id : i.id;
    activeById.set(key, i);
  }
  const jobOwners = new Map();
  for (const i of items.filter(isActive)) jobOwners.set(i.job, i);
  for (const r of required) {
    if (!r || !JOB_PATTERN.test(r.job ?? '')) {
      failures.push({ rule: 'G1', subject: `format:${r?.job}`, refs: [], message: `required job "${r?.job}" is not a valid job key` });
      continue;
    }
    const owner = r.owner ? activeById.get(r.owner) : null;
    const claimant = jobOwners.get(r.job);
    if (claimant && (!owner || claimant.ref !== owner.ref)) {
      failures.push({ rule: 'G1', subject: `owner:${r.job}`, refs: [claimant.ref],
        message: `${claimant.ref} claims required job "${r.job}", but jobs.json names ${r.owner ?? 'no owner'}` });
      continue;
    }
    if (owner) continue;
    if (r.planned) {
      warnings.push(`required job "${r.job}" has no active owner yet; planned: ${r.planned}${r.owner ? ` (intended owner ${r.owner})` : ''}`);
      continue;
    }
    failures.push({ rule: 'G1', subject: `gap:${r.job}`, refs: [],
      message: `required job "${r.job}" has no active owner and no planned entry` });
  }
  return { failures, warnings };
}

// ---------------------------------------------------------------------------
// Baseline violations: failures that exist today, each owned by a work package.

/**
 * baseline: [{ rule, subject, wp, note }]; done: work package ids marked done.
 * Returns { errors, accepted, stale }: errors are failures with no baseline
 * entry, entries whose work package is done, and entries that no longer fail.
 */
export function applyBaseline(failures, baseline = [], done = []) {
  const doneSet = new Set(done);
  const key = (f) => `${f.rule}::${f.subject}`;
  const byKey = new Map(baseline.map((b) => [key(b), b]));
  const errors = [];
  const accepted = [];
  const seen = new Set();
  for (const f of failures) {
    const b = byKey.get(key(f));
    if (!b) {
      errors.push(f);
      continue;
    }
    seen.add(key(f));
    if (doneSet.has(b.wp)) {
      errors.push({ ...f, message: `${f.message} [baseline entry belongs to ${b.wp}, which is marked done]` });
    } else {
      accepted.push({ ...f, wp: b.wp });
    }
  }
  const stale = baseline
    .filter((b) => !seen.has(key(b)))
    .map((b) => ({ rule: 'baseline', subject: key(b), refs: [],
      message: `baseline_violations lists ${b.rule} ${b.subject} (${b.wp}), which no longer fails; remove the entry` }));
  return { errors: [...errors, ...stale], accepted, stale };
}
