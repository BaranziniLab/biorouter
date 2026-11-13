// Cohort analysis for the PREVENT calculator: no DOM here, so it can be tested with node.
// A cohort arrives from a Biorouter chat (handoff.mjs) or from the synthetic demo generator.
import { computeRisks, ucsfCalibrated, type PreventInputs, type RiskKey } from "./prevent";

export type Scale = { total_cvd: number | null; ascvd: number | null; hf: number | null };
export type Member = { key: string; inputs: PreventInputs; status: "ok" | "missing" | "range"; problems: string[]; prevent: Scale; ucsf: Scale };
export type CohortMeta = { name: string; purpose: string; definition: string; as_of: string; source: string; received: string; upstream: Record<string, unknown> };
export type Cohort = { meta: CohortMeta; members: Member[]; ok: Member[] };
export type Summary = { n: number; mean: number; median: number; q1: number; q3: number; sum: number };
export type Rule = { id: string; label: string; eligibility: string; eligible: (p: PreventInputs) => boolean; apply: (p: PreventInputs) => PreventInputs };

export const OUTCOMES: RiskKey[] = ["total_cvd", "ascvd", "hf"];
export const INPUT_KEYS = ["age", "sex", "sbp", "total_cholesterol_mg_dl", "hdl_mg_dl", "bmi", "egfr", "diabetes", "current_smoker", "bp_treatment", "statin"];
const NUMERIC = ["age", "sbp", "total_cholesterol_mg_dl", "hdl_mg_dl", "bmi", "egfr"];
const FLAGS = ["diabetes", "current_smoker", "bp_treatment", "statin"];
export const AGE_BANDS: Array<[number, number]> = [[30, 39], [40, 49], [50, 59], [60, 69], [70, 79]];

export function parseSex(v: unknown): "female" | "male" | null {
  const s = String(v ?? "").trim().toLowerCase();
  if (["female", "f", "woman", "8532"].includes(s)) return "female";
  if (["male", "m", "man", "8507"].includes(s)) return "male";
  return null;
}
export function parseFlag(v: unknown): boolean | null {
  const s = String(v ?? "").trim().toLowerCase();
  if (["1", "true", "yes", "y", "t"].includes(s)) return true;
  if (["0", "false", "no", "n", "f"].includes(s)) return false;
  return null;
}
function flag(v: unknown): boolean { return v === true || v === 1 || v === "1" || v === "true"; }

/** Normalise one record to PREVENT inputs; returns the inputs and the names of missing or unreadable inputs. */
export function normalize(rec: Record<string, unknown>): { inputs: PreventInputs; missing: string[] } {
  const out: any = {}; const missing: string[] = [];
  for (const k of NUMERIC) {
    const raw = rec[k]; const n = raw === null || raw === undefined || raw === "" ? NaN : Number(raw);
    if (Number.isFinite(n)) out[k] = n; else { out[k] = ""; missing.push(k); }
  }
  const sex = parseSex(rec.sex); if (sex) out.sex = sex; else { out.sex = "female"; missing.push("sex"); }
  for (const k of FLAGS) { const b = parseFlag(rec[k]); if (b === null) { out[k] = false; missing.push(k); } else out[k] = b; }
  return { inputs: out as PreventInputs, missing };
}

function scaleOf(r: { total_cvd: number | null; ascvd: number | null; hf: number | null }): Scale { return { total_cvd: r.total_cvd, ascvd: r.ascvd, hf: r.hf }; }
function ucsfOf(s: Scale): Scale { return { total_cvd: ucsfCalibrated("total_cvd", s.total_cvd), ascvd: ucsfCalibrated("ascvd", s.ascvd), hf: ucsfCalibrated("hf", s.hf) }; }

export function evaluate(key: string, inputs: PreventInputs, missing: string[]): Member {
  const none: Scale = { total_cvd: null, ascvd: null, hf: null };
  if (missing.length) return { key, inputs, status: "missing", problems: missing.slice(), prevent: none, ucsf: none };
  const r = computeRisks(inputs);
  if (!r.valid) return { key, inputs, status: "range", problems: r.problems.map((p) => p.split(" ")[0]), prevent: none, ucsf: none };
  const prevent = scaleOf(r);
  return { key, inputs, status: "ok", problems: [], prevent, ucsf: ucsfOf(prevent) };
}

/** Build a cohort from a handoff payload: {kind:"cohort", columns:[...], rows:[[...]], name, purpose, definition, as_of, source, counts}. */
export function buildCohort(payload: any): Cohort {
  const cols: string[] = Array.isArray(payload.columns) ? payload.columns.map((c: unknown) => String(c)) : [];
  const idx: Record<string, number> = {}; cols.forEach((c, i) => { idx[c] = i; });
  for (const k of INPUT_KEYS) if (!(k in idx)) throw new Error("the cohort has no column " + k);
  const rows: unknown[][] = Array.isArray(payload.rows) ? payload.rows : [];
  const members = rows.map((row, i) => {
    const rec: Record<string, unknown> = {}; for (const k of INPUT_KEYS) rec[k] = row[idx[k]];
    const key = "key" in idx && row[idx.key] !== undefined && row[idx.key] !== "" ? String(row[idx.key]) : "C" + String(i + 1).padStart(5, "0");
    const n = normalize(rec); return evaluate(key, n.inputs, n.missing);
  });
  const meta: CohortMeta = {
    name: String(payload.name || "Unnamed cohort"), purpose: String(payload.purpose || ""), definition: String(payload.definition || ""),
    as_of: String(payload.as_of || ""), source: String(payload.source || "Biorouter chat"), received: new Date().toISOString().slice(0, 16).replace("T", " "),
    upstream: payload.counts && typeof payload.counts === "object" ? payload.counts : {},
  };
  return { meta, members, ok: members.filter((m) => m.status === "ok") };
}

export function quantile(sorted: number[], q: number): number {
  if (!sorted.length) return NaN; const pos = (sorted.length - 1) * q; const lo = Math.floor(pos), hi = Math.ceil(pos);
  return lo === hi ? sorted[lo] : sorted[lo] * (hi - pos) + sorted[hi] * (pos - lo);
}
export function summarize(values: number[]): Summary {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b); const sum = v.reduce((s, x) => s + x, 0);
  return { n: v.length, mean: v.length ? sum / v.length : NaN, median: quantile(v, 0.5), q1: quantile(v, 0.25), q3: quantile(v, 0.75), sum };
}
export function values(members: Member[], k: RiskKey, scale: "prevent" | "ucsf"): number[] { return members.map((m) => m[scale][k]).filter((x): x is number => x !== null); }

/** Counts of excluded members by reason and input. */
export function exclusions(c: Cohort): { missing: number; range: number; byInput: Record<string, number> } {
  const byInput: Record<string, number> = {}; let missing = 0, range = 0;
  for (const m of c.members) {
    if (m.status === "missing") missing++; else if (m.status === "range") range++; else continue;
    for (const p of m.problems) byInput[(m.status === "missing" ? "missing " : "out of range ") + p] = (byInput[(m.status === "missing" ? "missing " : "out of range ") + p] || 0) + 1;
  }
  return { missing, range, byInput };
}

export type GroupRow = { label: string; n: number; medians: Record<RiskKey, [number, number]> };
export function breakdown(ok: Member[]): GroupRow[] {
  const groups: Array<[string, (m: Member) => boolean]> = [
    ["Women", (m) => m.inputs.sex === "female"], ["Men", (m) => m.inputs.sex === "male"],
    ...AGE_BANDS.map(([a, b]) => ["Age " + a + "–" + b, (m: Member) => Number(m.inputs.age) >= a && Number(m.inputs.age) <= b] as [string, (m: Member) => boolean]),
  ];
  return groups.map(([label, pick]) => {
    const g = ok.filter(pick); const medians: any = {};
    for (const k of OUTCOMES) medians[k] = [summarize(values(g, k, "prevent")).median, summarize(values(g, k, "ucsf")).median];
    return { label, n: g.length, medians };
  });
}

/** Share of members per bin of width w (risk scale), up to xmax; the last bin collects the tail. */
export function histogram(vals: number[], w: number, xmax: number): Array<{ lo: number; hi: number; n: number }> {
  const nb = Math.round(xmax / w); const bins = Array.from({ length: nb }, (_, i) => ({ lo: i * w, hi: i === nb - 1 ? 1 : (i + 1) * w, n: 0 }));
  for (const v of vals) bins[Math.min(nb - 1, Math.max(0, Math.floor(v / w)))].n++;
  return bins;
}

/** Empirical percentile of a value among the cohort's members (mid-rank for ties). */
export function cohortPercentile(vals: number[], value: number): number {
  if (!vals.length || !Number.isFinite(value)) return NaN;
  let below = 0, equal = 0; for (const v of vals) { if (v < value) below++; else if (v === value) equal++; }
  return ((below + equal / 2) / vals.length) * 100;
}

// ── what if: PREVENT recomputed with literally changed inputs, only for the members a change applies to ──
// No treatment effects are assumed: PREVENT is a prognostic equation, and statin use is an input describing
// current treatment, so setting it to "yes" recomputes the equation for a treated person with the same values.
export const RULES: Rule[] = [
  { id: "sbp", label: "Systolic blood pressure −10 mmHg", eligibility: "all members (not below 90 mmHg, the PREVENT lower limit)", eligible: (p) => Number(p.sbp) > 90, apply: (p) => ({ ...p, sbp: Math.max(90, Number(p.sbp) - 10) }) },
  { id: "smoking", label: "Stop smoking", eligibility: "current smokers", eligible: (p) => flag(p.current_smoker), apply: (p) => ({ ...p, current_smoker: false }) },
  { id: "statin", label: "Statin use: yes", eligibility: "members not on a statin", eligible: (p) => !flag(p.statin), apply: (p) => ({ ...p, statin: true }) },
];

export type ScenarioResult = {
  perRule: Record<string, number>; changed: number; higher: number;
  outcomes: Record<RiskKey, { sumBefore: [number, number]; sumAfter: [number, number]; medianBefore: [number, number]; medianAfter: [number, number] }>;
};

/** Recompute PREVENT for every computable member with the chosen input changes applied where they fit.
 *  Reports the sum of predicted 10-year risks (expected events under the model) and the median, before and after. */
export function cohortScenario(ok: Member[], ruleIds: string[]): ScenarioResult {
  const rules = RULES.filter((r) => ruleIds.includes(r.id)); const perRule: Record<string, number> = {};
  for (const r of rules) perRule[r.id] = ok.filter((m) => r.eligible(m.inputs)).length;
  const before: Record<RiskKey, [number[], number[]]> = { total_cvd: [[], []], ascvd: [[], []], hf: [[], []] };
  const after: Record<RiskKey, [number[], number[]]> = { total_cvd: [[], []], ascvd: [[], []], hf: [[], []] };
  let changed = 0, higher = 0;
  for (const m of ok) {
    let p = m.inputs; let touched = false;
    for (const r of rules) if (r.eligible(m.inputs)) { p = r.apply(p); touched = true; }
    const res = touched ? computeRisks(p) : null; const use = touched && res && res.valid;
    if (use) { changed++; if ((res!.total_cvd as number) > (m.prevent.total_cvd as number)) higher++; }
    for (const k of OUTCOMES) {
      const b = m.prevent[k] as number; const a = use ? (res![k] as number) : b;
      before[k][0].push(b); before[k][1].push(m.ucsf[k] as number);
      after[k][0].push(a); after[k][1].push(use ? (ucsfCalibrated(k, a) as number) : (m.ucsf[k] as number));
    }
  }
  const out: any = {};
  for (const k of OUTCOMES) {
    const sb = before[k].map((v) => summarize(v)), sa = after[k].map((v) => summarize(v));
    out[k] = { sumBefore: [sb[0].sum, sb[1].sum], sumAfter: [sa[0].sum, sa[1].sum], medianBefore: [sb[0].median, sb[1].median], medianAfter: [sa[0].median, sa[1].median] };
  }
  return { perRule, changed, higher, outcomes: out };
}

// ── synthetic demo cohort (not patient data), deterministic ──
function rng(seed: number): () => number { let s = seed >>> 0; return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296; }; }
function normal(r: () => number, mean: number, sd: number): number { const u = Math.max(r(), 1e-12), v = r(); return mean + sd * Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v); }
export function demoPayload(n = 600, seed = 2026): any {
  const r = rng(seed); const rows: unknown[][] = [];
  for (let i = 0; i < n; i++) {
    const age = Math.round(Math.min(79, Math.max(30, normal(r, 58, 11)))); const female = r() < 0.55;
    const diabetes = r() < 0.08 + 0.004 * (age - 30); const bpTx = r() < 0.1 + 0.009 * (age - 30); const statin = r() < 0.05 + 0.006 * (age - 30) + (diabetes ? 0.25 : 0);
    let sbp: number | string = Math.round(normal(r, 118 + 0.35 * (age - 30) + (bpTx ? 6 : 0), 15));
    let tc: number | string = Math.round(normal(r, statin ? 172 : 200, 36)); const hdl = Math.round(normal(r, female ? 60 : 48, 13));
    let bmi: number | string = Math.round(normal(r, 28.5, 5.5) * 10) / 10; const egfr = Math.round(Math.min(130, normal(r, 108 - 0.75 * (age - 30), 15)));
    if (r() < 0.06) tc = ""; if (r() < 0.03) bmi = ""; if (r() < 0.02) sbp = "";
    rows.push(["D" + String(i + 1).padStart(4, "0"), age, female ? "female" : "male", sbp, tc, hdl, bmi, egfr, diabetes ? 1 : 0, r() < 0.11 ? 1 : 0, bpTx ? 1 : 0, statin ? 1 : 0]);
  }
  return { v: 1, kind: "cohort", name: "Synthetic demo cohort", purpose: "Demonstration only: simulated adults, not patient data.", definition: "600 simulated adults aged 30–79 without cardiovascular disease, with some missing and out-of-range values.", as_of: "2016-01-01", source: "Synthetic (generated in the app)", columns: ["key"].concat(INPUT_KEYS), rows };
}
