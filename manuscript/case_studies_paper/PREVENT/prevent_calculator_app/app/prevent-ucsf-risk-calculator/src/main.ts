import { createApp, mountTimeline } from "./sdk";
import { computeRisks, formatPct, projectAges, referenceCohort, riskPercentile, ucsfCalibrated, ucsfRecalibration, type PreventInputs, type RiskKey, type Sex } from "./prevent";
import { OMOP_RULES } from "./preventData";
import { RULES, breakdown, buildCohort, cohortPercentile, cohortScenario, demoPayload, exclusions, histogram, normalize, quantile, summarize, values, type Cohort, type Member } from "./cohort";

type SourceMap = Record<string, { date?: string; source?: string; note?: string }>;
type Risks = ReturnType<typeof computeRisks>;
type Scenario = { name: string; patient: PreventInputs; risks: Risks };
type Field = { key: keyof PreventInputs; label: string; short: string; kind: "number" | "select" | "toggle"; unit: string; min: number; max: number; step: number; modifiable: boolean };
type Group = Sex | "overall" | "cohort";
type Bin = { lo: number; hi: number; n: number };

const br = createApp({ autoChat: false });
mountTimeline(br, "[data-br-progress]", { maxItems: 12 });

const demo: PreventInputs = { age: 58, sex: "female", sbp: 132, total_cholesterol_mg_dl: 198, hdl_mg_dl: 54, bmi: 28.4, egfr: 82, diabetes: false, current_smoker: false, bp_treatment: true, statin: false };
let patient: PreventInputs = { ...demo };
let scenario: PreventInputs = { ...demo };
let sources: SourceMap = labelAll("Synthetic demo value");
let missing: Record<string, boolean> = {};
let scenarios: Scenario[] = [];
let cohort: Cohort | null = null;
let origin: { name: string; key: string } | null = null;
let shownRows = 50;

const RISKS: RiskKey[] = ["total_cvd", "ascvd", "hf"];
const LABEL: Record<RiskKey, string> = { total_cvd: "Total CVD", ascvd: "ASCVD", hf: "Heart failure" };
const PLAIN: Record<RiskKey, string> = { total_cvd: "heart attack, stroke or heart failure", ascvd: "heart attack or stroke", hf: "heart failure" };
// Input ranges are those of PREVENT and of the reference cohort (adults 30-79).
const FIELDS: Field[] = [
  { key: "age", label: "Age", short: "Age", kind: "number", unit: "years", min: 30, max: 79, step: 1, modifiable: false },
  { key: "sex", label: "Sex", short: "Sex", kind: "select", unit: "", min: 0, max: 0, step: 1, modifiable: false },
  { key: "sbp", label: "Systolic blood pressure", short: "SBP", kind: "number", unit: "mmHg", min: 90, max: 180, step: 1, modifiable: true },
  { key: "total_cholesterol_mg_dl", label: "Total cholesterol", short: "Total chol.", kind: "number", unit: "mg/dL", min: 130, max: 320, step: 1, modifiable: true },
  { key: "hdl_mg_dl", label: "HDL cholesterol", short: "HDL", kind: "number", unit: "mg/dL", min: 20, max: 100, step: 1, modifiable: true },
  { key: "bmi", label: "Body-mass index", short: "BMI", kind: "number", unit: "kg/m²", min: 18.5, max: 39.9, step: 0.1, modifiable: true },
  { key: "egfr", label: "eGFR", short: "eGFR", kind: "number", unit: "mL/min/1.73 m²", min: 15, max: 140, step: 1, modifiable: true },
  { key: "diabetes", label: "Diabetes", short: "Diabetes", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
  { key: "current_smoker", label: "Current smoking", short: "Smoking", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
  { key: "bp_treatment", label: "Blood-pressure treatment", short: "BP treatment", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
  { key: "statin", label: "Statin", short: "Statin", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
];
// The examples of the brief; each changes inputs literally and PREVENT is recomputed.
const PRESETS: Array<{ label: string; apply: (s: any) => void }> = [
  { label: "SBP −10 mmHg", apply: (s) => { s.sbp = Math.max(90, Number(s.sbp) - 10); } },
  { label: "Stop smoking", apply: (s) => { s.current_smoker = false; } },
  { label: "Statin use: yes", apply: (s) => { s.statin = true; } },
];
const SVG_NS = "http://www.w3.org/2000/svg";

// ── helpers ─────────────────────────────────────────────────────────────────
function el<T extends HTMLElement>(id: string): T { return document.getElementById(id) as T; }
function make<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag); if (cls) e.className = cls; if (text) e.textContent = text; return e;
}
function svg(tag: string, attrs: Record<string, string | number>, text = ""): SVGElement {
  const e = document.createElementNS(SVG_NS, tag); for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v)); if (text) e.textContent = text; return e;
}
function flag(v: unknown): boolean { return v === true || v === 1 || v === "1" || v === "true"; }
function labelAll(source: string): SourceMap { const m: SourceMap = {}; for (const k of ["age","sex","sbp","total_cholesterol_mg_dl","hdl_mg_dl","bmi","egfr","diabetes","current_smoker","bp_treatment","statin"]) m[k] = { source }; return m; }
function group(): Group { return (el<HTMLSelectElement>("sexFilter").value || "overall") as Group; }
function groupName(g: Group): string { return g === "female" ? "women (MOCK reference)" : g === "male" ? "men (MOCK reference)" : g === "cohort" ? "members of this cohort" : "MOCK reference adults"; }
function pct(x: number | null | undefined, d = 1): string { return formatPct(x === undefined ? null : x, d); }
function signed(x: number, digits: number, unit: string): string { return (x > 0 ? "+" : x < 0 ? "−" : "±") + Math.abs(x).toFixed(digits) + unit; }
function showPatients(): boolean { return el<HTMLInputElement>("showPatients").checked; }
/** Counts from 1 to 10 are shown as "<11" unless individual patients are shown. */
function cnt(n: number): string { return !showPatients() && n >= 1 && n <= 10 ? "<11" : n.toLocaleString("en-US"); }
function fmtValue(f: Field, v: unknown): string {
  if (f.kind === "toggle") return flag(v) ? "Yes" : "No";
  if (f.kind === "select") return v === "male" ? "Male" : "Female";
  return v === "" || v === undefined || v === null ? "–" : String(v) + (f.unit ? " " + f.unit : "");
}
function fieldLabel(key: string): string { const f = FIELDS.find((x) => x.key === key); return f ? f.label : key; }
function fieldProblem(f: Field, v: unknown): string {
  if (f.kind !== "number") return "";
  const n = Number(v);
  if (v === "" || v === null || v === undefined || !Number.isFinite(n)) return "Enter a value";
  if (n < f.min || n > f.max) return "Outside the PREVENT range (" + f.min + "–" + f.max + "): risk not computed";
  return "";
}
function problems(p: PreventInputs): Field[] { return FIELDS.filter((f) => fieldProblem(f, (p as any)[f.key])); }
let toastTimer = 0;
function toast(text: string) { const t = el<HTMLDivElement>("toast"); t.textContent = text; t.hidden = false; window.clearTimeout(toastTimer); toastTimer = window.setTimeout(() => { t.hidden = true; }, 6000); }

// percentile and distribution for the chosen comparison group
function percentileIn(k: RiskKey, v: number | null, g: Group): number {
  if (v === null) return NaN;
  if (g === "cohort") return cohort ? cohortPercentile(values(cohort.ok, k, "prevent"), v) : NaN;
  return riskPercentile(k, v, g);
}
function binsFor(k: RiskKey, g: Group): { bins: Bin[]; n: number } {
  if (g === "cohort") {
    if (!cohort) return { bins: [], n: 0 };
    const v = values(cohort.ok, k, "prevent").sort((a, b) => a - b);
    const xmax = Math.min(0.6, Math.max(0.1, Math.ceil(quantile(v, 0.99) * 1.1 * 20) / 20));
    return { bins: histogram(v, 0.005, xmax), n: v.length };
  }
  return { bins: ((referenceCohort as any).distribution[k] || {})[g] || [], n: Number((referenceCohort as any).n[g]) };
}

// ── patient form (built once per patient, so typing never loses focus) ────
function renderForm() {
  const host = el<HTMLDivElement>("fields"); host.textContent = "";
  for (const f of FIELDS) {
    const wrap = make("label", "br-field field" + (f.kind === "toggle" ? " field-toggle" : ""));
    wrap.appendChild(make("span", "br-label", f.unit ? f.label + " (" + f.unit + ")" : f.label));
    let control: HTMLElement;
    if (f.kind === "select") {
      const s = make("select", "br-select");
      for (const o of ["female", "male"]) { const opt = make("option", "", o === "female" ? "Female" : "Male"); opt.value = o; s.appendChild(opt); }
      s.value = String(patient.sex); s.onchange = () => setInput(f, s.value); control = s;
    } else if (f.kind === "toggle") {
      const row = make("span", "toggle"); const c = make("input"); c.type = "checkbox"; c.checked = flag((patient as any)[f.key]);
      const t = make("span", "toggle-text", c.checked ? "Yes" : "No");
      c.onchange = () => { t.textContent = c.checked ? "Yes" : "No"; setInput(f, c.checked); };
      c.id = "input-" + f.key; row.append(c, t); control = row;
    } else {
      const i = make("input", "br-input"); i.type = "number"; i.inputMode = "decimal";
      i.min = String(f.min); i.max = String(f.max); i.step = String(f.step); i.value = String((patient as any)[f.key] ?? "");
      i.oninput = () => setInput(f, i.value === "" ? "" : Number(i.value)); control = i;
    }
    if (f.kind !== "toggle") control.id = "input-" + f.key;
    wrap.appendChild(control);
    const note = make("span", "field-note"); note.id = "note-" + f.key; wrap.appendChild(note);
    host.appendChild(wrap);
  }
  updateFieldNotes();
}

function updateFieldNotes() {
  for (const f of FIELDS) {
    const note = document.getElementById("note-" + f.key); if (!note) continue;
    const prob = fieldProblem(f, (patient as any)[f.key]); const s = sources[f.key];
    const where = missing[f.key] ? "Not available in the source: enter by hand" : s && s.source ? s.source + (s.date ? " · " + s.date : "") : "";
    note.textContent = prob || where || (f.kind === "number" ? "Range " + f.min + "–" + f.max : "");
    note.classList.toggle("warn", Boolean(prob) || Boolean(missing[f.key]));
    const ctl = document.getElementById("input-" + f.key); if (ctl) ctl.setAttribute("aria-invalid", prob ? "true" : "false");
  }
}

function setInput(f: Field, value: unknown) {
  (patient as any)[f.key] = value;
  missing[f.key] = false;
  sources[f.key] = { source: "Entered by hand" };
  scenario = { ...patient };
  br.state.set("/patient", patient);
  br.signals.emit("patient_changed", { changed: f.key, patient });
  updateFieldNotes();
  renderOutputs();
}

// ── risk cards ──────────────────────────────────────────────────────────────
// The twin-scale strip: bar = PREVENT, coral mark = calibrated (MOCK), on a 0-30% scale.
const SCALE_MAX = 0.3;
function at(v: number): number { return (Math.min(Math.max(v, 0), SCALE_MAX) / SCALE_MAX) * 100; }
function strip(k: RiskKey, prevent: number | null, ucsf: number | null): HTMLElement {
  const s = make("div", "strip"); s.style.setProperty("--series", "var(--c-" + k + ")");
  if (prevent === null) return s;
  const bar = make("span", "strip-bar"); bar.style.width = at(prevent) + "%"; s.appendChild(bar);
  if (ucsf !== null) { const t = make("span", "strip-tick"); t.style.left = at(ucsf) + "%"; s.appendChild(t); }
  s.title = "PREVENT " + pct(prevent) + (ucsf !== null ? ", calibrated (MOCK) " + pct(ucsf) : "") + " (scale 0–30%)";
  return s;
}
function bandStrip(k: RiskKey, q1: number, q3: number, median: number, medianUcsf: number): HTMLElement {
  const s = make("div", "strip"); s.style.setProperty("--series", "var(--c-" + k + ")");
  if (!Number.isFinite(median)) return s;
  const band = make("span", "strip-band"); band.style.left = at(q1) + "%"; band.style.width = Math.max(0.5, at(q3) - at(q1)) + "%"; s.appendChild(band);
  const med = make("span", "strip-median"); med.style.left = at(median) + "%"; s.appendChild(med);
  const t = make("span", "strip-tick"); t.style.left = at(medianUcsf) + "%"; s.appendChild(t);
  s.title = "PREVENT median " + pct(median) + ", interquartile range " + pct(q1) + "–" + pct(q3) + "; calibrated (MOCK) median " + pct(medianUcsf) + " (scale 0–30%)";
  return s;
}
function valueRow(label: string, base: number | null, next?: number | null): HTMLElement {
  const row = make("div", "value-row");
  row.appendChild(make("span", "value-label", label));
  const right = make("span", "value-right");
  if (next === undefined) right.appendChild(make("span", "value", pct(base)));
  else {
    right.appendChild(make("span", "value", pct(base) + " → " + pct(next)));
    if (base !== null && next !== null) {
      const d = next - base; const rel = base > 0 ? (d / base) * 100 : 0;
      right.appendChild(make("span", "delta " + (d < -0.00005 ? "down" : d > 0.00005 ? "up" : ""), signed(d * 100, 1, " pp") + " (" + signed(rel, 0, "%") + ")"));
    }
  }
  row.appendChild(right);
  return row;
}

function riskCard(k: RiskKey, r: Risks, c?: Risks): HTMLElement {
  const card = make("div", "risk-card"); card.style.setProperty("--series", "var(--c-" + k + ")");
  card.appendChild(make("div", "risk-name", LABEL[k]));
  card.appendChild(make("div", "risk-plain", PLAIN[k]));
  const p = r[k];
  card.appendChild(valueRow("PREVENT", p, c ? c[k] : undefined));
  card.appendChild(valueRow("calibrated (MOCK)", ucsfCalibrated(k, p), c ? ucsfCalibrated(k, c[k]) : undefined));
  if (!c) {
    card.appendChild(strip(k, p, ucsfCalibrated(k, p)));
    const pc = percentileIn(k, p, group());
    card.appendChild(make("div", "risk-pct", Number.isFinite(pc) ? "Higher than " + Math.round(pc) + "% of " + groupName(group()) : "Not computed"));
  }
  return card;
}

function renderRiskCards(target: HTMLElement, base: PreventInputs, compare?: PreventInputs) {
  target.textContent = "";
  const r = computeRisks(base); const c = compare ? computeRisks(compare) : undefined;
  for (const k of RISKS) target.appendChild(riskCard(k, r, c));
}

function renderStatus(id: string, p: PreventInputs) {
  const bad = problems(p); const box = el<HTMLDivElement>(id);
  box.textContent = bad.length ? "Risk not computed. Check: " + bad.map((f) => f.label).join(", ") + "." : "";
  box.hidden = !bad.length;
}

function renderSourceRows() {
  const body = el<HTMLTableSectionElement>("sourceRows"); body.textContent = "";
  for (const f of FIELDS) {
    const tr = make("tr"); const s = sources[f.key] || {};
    for (const txt of [f.label, fmtValue(f, (patient as any)[f.key]), missing[f.key] ? "Not available; enter by hand" : s.source || "–", s.date || "–"]) tr.appendChild(make("td", "", txt));
    body.appendChild(tr);
  }
}

function renderOrigin() {
  const box = el<HTMLDivElement>("patientOrigin");
  box.hidden = !origin;
  if (origin) el<HTMLSpanElement>("originText").textContent = "Member " + origin.key + " of the cohort \"" + origin.name + "\". Values can be edited; the cohort is not changed.";
}

// ── histogram of a distribution (reference cohort or this cohort) with the patient marked ──
function renderHistogram(host: SVGSVGElement, k: RiskKey, value: number | null, g: Group, height: number) {
  host.textContent = "";
  const { bins } = binsFor(k, g);
  const w = Math.max(320, host.clientWidth || (host.parentElement ? host.parentElement.clientWidth : 0) || 640), h = height;
  host.setAttribute("viewBox", "0 0 " + w + " " + h); host.setAttribute("height", String(h));
  if (!bins.length) return;
  const last = bins[bins.length - 1];
  const xmax = Math.min(0.6, Math.max(0.1, Math.ceil((Number(last.lo) + 0.005) * 20) / 20));
  const m = { l: 12, r: 14, t: 18, b: 26 }, iw = w - m.l - m.r, ih = h - m.t - m.b;
  const X = (v: number) => m.l + (Math.min(Math.max(v, 0), xmax) / xmax) * iw;
  const dens = bins.map((b) => { const hi = Number(b.hi) >= 1 ? xmax : Math.min(Number(b.hi), xmax); return Number(b.n) / Math.max((hi - Number(b.lo)) / 0.005, 1); });
  const dmax = Math.max(...dens, 1e-9);
  const step = xmax <= 0.2 ? 0.05 : 0.1;
  for (let t = 0; t <= xmax + 1e-9; t += step) {
    host.appendChild(svg("line", { x1: X(t), x2: X(t), y1: m.t, y2: m.t + ih, class: "grid" }));
    host.appendChild(svg("text", { x: X(t), y: h - 8, class: "tick", "text-anchor": "middle" }, Math.round(t * 100) + "%"));
  }
  bins.forEach((b, i) => {
    const lo = Number(b.lo), hi = Number(b.hi) >= 1 ? xmax : Math.min(Number(b.hi), xmax);
    if (lo >= xmax || b.n === 0) return;
    const bh = (dens[i] / dmax) * ih; const inBin = value !== null && value >= lo && (value < Number(b.hi) || Number(b.hi) >= 1);
    const rect = svg("rect", { x: X(lo) + 0.5, y: m.t + ih - bh, width: Math.max(1, X(hi) - X(lo) - 1), height: bh, rx: 1.5, class: "bar" + (inBin ? " bar-patient" : "") });
    rect.style.setProperty("--series", "var(--c-" + k + ")");
    const count = g === "cohort" ? cnt(Number(b.n)) : Number(b.n).toLocaleString("en-US");
    rect.appendChild(svg("title", {}, (lo * 100).toFixed(1) + "–" + (Number(b.hi) >= 1 ? "100" : (Number(b.hi) * 100).toFixed(1)) + "%: " + count));
    host.appendChild(rect);
  });
  if (value !== null && Number.isFinite(value)) {
    const x = X(value);
    host.appendChild(svg("line", { x1: x, x2: x, y1: m.t - 4, y2: m.t + ih, class: "marker" }));
    const right = x > w - 120;
    host.appendChild(svg("text", { x: right ? x - 6 : x + 6, y: m.t + 6, class: "marker-label", "text-anchor": right ? "end" : "start" }, "Patient " + pct(value) + (value > xmax ? " (off scale)" : "")));
  }
}

function legend(k: RiskKey, g: Group): HTMLElement {
  const lg = make("div", "legend"); const { n } = binsFor(k, g);
  const name = g === "cohort" && cohort ? "Cohort \"" + cohort.meta.name + "\"" : groupName(g);
  const a = make("span", "lg-bar", name + ", PREVENT risk (n = " + cnt(n) + ")"); a.style.setProperty("--series", "var(--c-" + k + ")");
  lg.append(a, make("span", "lg-marker", "This patient"));
  return lg;
}

function renderMini() {
  const r = computeRisks(patient);
  renderHistogram(el<SVGSVGElement>("miniPosition") as any, "total_cvd", r.total_cvd, group(), 120);
  const host = el<HTMLDivElement>("miniLegend"); host.textContent = ""; host.appendChild(legend("total_cvd", group()));
  el<HTMLHeadingElement>("miniTitle").textContent = "Total CVD: position among " + groupName(group());
}

function renderPosition() {
  const r = computeRisks(patient); const g = group();
  const host = el<HTMLDivElement>("histograms"); host.textContent = "";
  for (const k of RISKS) {
    const frame = make("div", "chart-frame");
    const head = make("div", "chart-head");
    head.appendChild(make("h3", "", LABEL[k] + ": " + pct(r[k]) + " PREVENT · " + pct(ucsfCalibrated(k, r[k])) + " calibrated (MOCK)"));
    const pc = percentileIn(k, r[k], g);
    head.appendChild(make("p", "subtle", Number.isFinite(pc) ? "Higher than " + Math.round(pc) + "% of " + groupName(g) + "." : "Not computed: complete the inputs within range."));
    frame.appendChild(head);
    const s = document.createElementNS(SVG_NS, "svg") as SVGSVGElement; s.setAttribute("class", "chart"); s.setAttribute("role", "img");
    s.setAttribute("aria-label", LABEL[k] + " risk distribution of " + groupName(g) + " with the patient marked");
    frame.appendChild(s); frame.appendChild(legend(k, g)); host.appendChild(frame);
    renderHistogram(s, k, r[k], g, 150);
  }
}

// ── what if (one patient) ───────────────────────────────────────────────────
function describe(s: PreventInputs, base: PreventInputs): string {
  const out: string[] = [];
  for (const f of FIELDS) {
    if (!f.modifiable) continue;
    const a = (base as any)[f.key], b = (s as any)[f.key];
    if (f.kind === "toggle") { if (flag(a) !== flag(b)) out.push(f.short + ": " + (flag(b) ? "yes" : "no")); }
    else if (Number(a) !== Number(b)) out.push(f.short + " " + a + "→" + b);
  }
  return out.length ? out.join(", ") : "No change";
}

function renderScenarioControls() {
  const host = el<HTMLDivElement>("scenarioControls"); host.textContent = "";
  for (const f of FIELDS) {
    if (!f.modifiable) continue;
    const wrap = make("label", "br-field field" + (f.kind === "toggle" ? " field-toggle" : ""));
    wrap.appendChild(make("span", "br-label", f.unit ? f.label + " (" + f.unit + ")" : f.label));
    const now = (patient as any)[f.key];
    if (f.kind === "toggle") {
      const row = make("span", "toggle"); const c = make("input"); c.type = "checkbox"; c.checked = flag((scenario as any)[f.key]);
      const t = make("span", "toggle-text", (c.checked ? "Yes" : "No") + " (now " + (flag(now) ? "yes" : "no") + ")");
      c.onchange = () => { (scenario as any)[f.key] = c.checked; t.textContent = (c.checked ? "Yes" : "No") + " (now " + (flag(now) ? "yes" : "no") + ")"; renderScenario(); };
      row.append(c, t); wrap.appendChild(row);
    } else {
      const row = make("span", "slider-row");
      const input = make("input", "br-slider"); input.type = "range"; input.min = String(f.min); input.max = String(f.max); input.step = String(f.step);
      input.value = String((scenario as any)[f.key]);
      const val = make("span", "slider-val", String((scenario as any)[f.key]) + " (now " + now + ")");
      input.oninput = () => { (scenario as any)[f.key] = Number(input.value); val.textContent = input.value + " (now " + now + ")"; renderScenario(); };
      row.append(input, val); wrap.appendChild(row);
    }
    host.appendChild(wrap);
  }
  const presets = el<HTMLDivElement>("presets"); presets.textContent = "";
  for (const p of PRESETS) {
    const b = make("button", "br-btn br-btn--secondary chip-btn", p.label); b.type = "button";
    b.onclick = () => { p.apply(scenario as any); renderScenarioControls(); renderScenario(); };
    presets.appendChild(b);
  }
}

function renderScenario() {
  renderRiskCards(el<HTMLDivElement>("scenarioRisks"), patient, scenario);
  el<HTMLParagraphElement>("scenarioLabel").textContent = "Scenario: " + describe(scenario, patient);
  renderStatus("whatifStatus", scenario);
  renderSaved();
}

function renderSaved() {
  const table = el<HTMLTableElement>("savedTable"); table.textContent = "";
  const cols: Array<{ name: string; risks: Risks; changes: string }> = [{ name: "Current", risks: computeRisks(patient), changes: "–" }]
    .concat(scenarios.map((s, i) => ({ name: "Scenario " + (i + 1), risks: s.risks, changes: s.name })));
  const head = make("tr"); head.appendChild(make("th", "", "10-year risk: PREVENT · calibrated (MOCK)")); for (const c of cols) head.appendChild(make("th", "", c.name));
  const thead = make("thead"); thead.appendChild(head); table.appendChild(thead);
  const body = make("tbody");
  for (const k of RISKS) {
    const tr = make("tr"); tr.appendChild(make("td", "", LABEL[k]));
    for (const c of cols) tr.appendChild(make("td", "num", pct(c.risks[k]) + " · " + pct(ucsfCalibrated(k, c.risks[k]))));
    body.appendChild(tr);
  }
  const ch = make("tr", "changes"); ch.appendChild(make("td", "", "Changes")); for (const c of cols) ch.appendChild(make("td", "", c.changes)); body.appendChild(ch);
  table.appendChild(body);
  el<HTMLParagraphElement>("savedHint").textContent = scenarios.length ? "" : "Save up to 3 scenarios to compare them side by side.";
}

// ── projection ──────────────────────────────────────────────────────────────
function renderProjection() {
  const host = el<SVGSVGElement>("projectionChart") as any as SVGSVGElement; host.textContent = "";
  const rows = el<HTMLTableSectionElement>("projectionRows"); rows.textContent = "";
  const note = el<HTMLParagraphElement>("projectionNote");
  const age = Number(patient.age);
  const pts = Number.isFinite(age) && computeRisks(patient).valid ? projectAges(patient, 79 - Math.ceil(age)) : [];
  if (pts.length < 2) { note.textContent = pts.length === 1 ? "The patient is 79, the upper age limit of PREVENT: no projection." : "Complete the inputs within range to see the projection."; note.hidden = false; return; }
  note.textContent = ""; note.hidden = true;
  const w = Math.max(360, host.clientWidth || (host.parentElement ? host.parentElement.clientWidth : 0) || 860), h = 300;
  host.setAttribute("viewBox", "0 0 " + w + " " + h);
  const m = { l: 44, r: 16, t: 14, b: 34 }, iw = w - m.l - m.r, ih = h - m.t - m.b;
  const top = Math.max(0.05, Math.ceil(Math.max(...pts.flatMap((p) => [p.total_cvd, p.ascvd, p.hf])) * 1.1 * 20) / 20);
  const a0 = pts[0].age, a1 = pts[pts.length - 1].age;
  const X = (a: number) => m.l + ((a - a0) / Math.max(1, a1 - a0)) * iw;
  const Y = (v: number) => m.t + ih - (v / top) * ih;
  const ystep = top <= 0.2 ? 0.05 : 0.1;
  for (let t = 0; t <= top + 1e-9; t += ystep) {
    host.appendChild(svg("line", { x1: m.l, x2: m.l + iw, y1: Y(t), y2: Y(t), class: "grid" }));
    host.appendChild(svg("text", { x: m.l - 6, y: Y(t) + 4, class: "tick", "text-anchor": "end" }, Math.round(t * 100) + "%"));
  }
  const astep = a1 - a0 > 20 ? 10 : 5;
  for (let a = Math.ceil(a0 / astep) * astep; a <= a1; a += astep) host.appendChild(svg("text", { x: X(a), y: h - 12, class: "tick", "text-anchor": "middle" }, String(a)));
  host.appendChild(svg("text", { x: m.l + iw / 2, y: h - 1, class: "tick", "text-anchor": "middle" }, "Age (years)"));
  for (const k of RISKS) {
    const pre = pts.map((p) => X(p.age) + "," + Y((p as any)[k])).join(" ");
    const cal = pts.map((p) => X(p.age) + "," + Y(ucsfCalibrated(k, (p as any)[k]) ?? 0)).join(" ");
    for (const [pointsAttr, dashed] of [[pre, false], [cal, true]] as Array<[string, boolean]>) {
      const line = svg("polyline", { points: pointsAttr, class: "series" + (dashed ? " dashed" : "") }); line.style.setProperty("--series", "var(--c-" + k + ")"); host.appendChild(line);
    }
    for (const p of pts) {
      const v = (p as any)[k] as number; const dot = svg("circle", { cx: X(p.age), cy: Y(v), r: 6, class: "hit" });
      dot.appendChild(svg("title", {}, "Age " + p.age + ", " + LABEL[k] + ": " + pct(v) + " PREVENT, " + pct(ucsfCalibrated(k, v)) + " calibrated (MOCK)"));
      host.appendChild(dot);
    }
  }
  for (const p of pts) {
    if (p.age !== a0 && p.age !== a1 && (p.age - a0) % 5 !== 0) continue;
    const tr = make("tr"); tr.appendChild(make("td", "", String(p.age) + (p.age === a0 ? " (now)" : "")));
    for (const k of RISKS) tr.appendChild(make("td", "num", pct((p as any)[k]) + " · " + pct(ucsfCalibrated(k, (p as any)[k]))));
    rows.appendChild(tr);
  }
}

// ── cohort ──────────────────────────────────────────────────────────────────
function loadCohort(c: Cohort) {
  cohort = c; shownRows = 50;
  const opt = el<HTMLOptionElement>("optCohort"); opt.hidden = false; opt.disabled = false; opt.textContent = "This cohort";
  renderCohort();
}

function tableRows(table: HTMLTableElement, head: string[], rows: string[][], numericFrom = 1) {
  table.textContent = "";
  if (head.some((h) => h)) { const thead = make("thead"); const tr = make("tr"); for (const h of head) tr.appendChild(make("th", "", h)); thead.appendChild(tr); table.appendChild(thead); }
  const body = make("tbody");
  for (const r of rows) { const row = make("tr"); r.forEach((v, i) => row.appendChild(make("td", i >= numericFrom ? "num" : "", v))); body.appendChild(row); }
  table.appendChild(body);
}

function renderCohort() {
  el<HTMLDivElement>("cohortEmpty").hidden = Boolean(cohort);
  el<HTMLDivElement>("cohortLoaded").hidden = !cohort;
  if (!cohort) return;
  const c = cohort; const ex = exclusions(c);
  el<HTMLHeadingElement>("cohortName").textContent = c.meta.name;
  el<HTMLParagraphElement>("cohortPurpose").textContent = c.meta.purpose;
  tableRows(el<HTMLTableElement>("cohortMeta"), ["", ""], [["Definition", c.meta.definition || "–"], ["As-of date", c.meta.as_of || "–"], ["Source", c.meta.source], ["Received", c.meta.received]], 99);
  const countRows: string[][] = [];
  for (const [k, v] of Object.entries(c.meta.upstream)) countRows.push([k.replace(/_/g, " "), typeof v === "number" ? cnt(v) : String(v)]);
  countRows.push(["Members received", cnt(c.members.length)], ["PREVENT computable", cnt(c.ok.length)], ["Excluded: missing input", cnt(ex.missing)], ["Excluded: input outside the PREVENT range", cnt(ex.range)]);
  for (const [k, v] of Object.entries(ex.byInput).sort((a, b) => b[1] - a[1])) { const parts = k.split(" "); const key = parts.pop() || ""; countRows.push(["· " + parts.join(" ") + ": " + fieldLabel(key), cnt(v)]); }
  tableRows(el<HTMLTableElement>("cohortCounts"), ["", "n"], countRows);

  // risk summary per outcome
  const grid = el<HTMLDivElement>("cohortRisk"); grid.textContent = "";
  for (const k of RISKS) {
    const sp = summarize(values(c.ok, k, "prevent")), su = summarize(values(c.ok, k, "ucsf"));
    const card = make("div", "risk-card"); card.style.setProperty("--series", "var(--c-" + k + ")");
    card.appendChild(make("div", "risk-name", LABEL[k])); card.appendChild(make("div", "risk-plain", PLAIN[k]));
    for (const [label, s] of [["PREVENT", sp], ["calibrated (MOCK)", su]] as Array<[string, any]>) {
      const row = make("div", "value-row"); row.appendChild(make("span", "value-label", label + ": median (IQR)"));
      const right = make("span", "value-right"); right.appendChild(make("span", "value", pct(s.median)));
      right.appendChild(make("span", "delta", pct(s.q1) + "–" + pct(s.q3))); row.appendChild(right); card.appendChild(row);
    }
    card.appendChild(bandStrip(k, sp.q1, sp.q3, sp.median, su.median));
    card.appendChild(make("div", "risk-pct", "Predicted events in 10 years: " + sp.sum.toFixed(1) + " (PREVENT) · " + su.sum.toFixed(1) + " (calibrated (MOCK)), among " + cnt(sp.n) + " members"));
    grid.appendChild(card);
  }
  renderCohortChart();
  const rows = breakdown(c.ok).map((g) => [g.label, cnt(g.n)].concat(RISKS.map((k) => g.n === 0 || (!showPatients() && g.n <= 10) ? "–" : pct(g.medians[k][0]) + " · " + pct(g.medians[k][1]))));
  tableRows(el<HTMLTableElement>("cohortBreakdown"), ["Group", "n", "Total CVD", "ASCVD", "Heart failure"], rows);
  renderCohortRules(); renderCohortScenario(); renderPatientTable();
}

function renderCohortChart() {
  if (!cohort) return;
  const k = el<HTMLSelectElement>("cohortOutcome").value as RiskKey;
  const host = el<SVGSVGElement>("cohortChart") as any as SVGSVGElement; host.textContent = "";
  const vals = values(cohort.ok, k, "prevent").sort((a, b) => a - b);
  const w = Math.max(360, host.clientWidth || (host.parentElement ? host.parentElement.clientWidth : 0) || 860), h = 220;
  host.setAttribute("viewBox", "0 0 " + w + " " + h); host.setAttribute("height", String(h));
  const lg = el<HTMLDivElement>("cohortLegend"); lg.textContent = "";
  if (!vals.length) return;
  const xmax = Math.min(0.6, Math.max(0.1, Math.ceil(quantile(vals, 0.99) * 1.1 * 20) / 20)); const width = 0.01;
  const bins = histogram(vals, width, xmax); const share = bins.map((b) => (b.n / vals.length) * 100);
  // reference: share of reference adults per bin, from the aggregated reference bins (overlap-weighted)
  const ref = ((referenceCohort as any).distribution[k] || {}).overall || []; const nRef = Number((referenceCohort as any).n.overall);
  const refShare = bins.map((b, i) => {
    const lo = b.lo, hi = i === bins.length - 1 ? 1 : b.hi; let s = 0;
    for (const r of ref) { const a = Math.max(lo, Number(r.lo)), z = Math.min(hi, Number(r.hi)); if (z > a) s += Number(r.n) * (z - a) / Math.max(Number(r.hi) - Number(r.lo), 1e-9); }
    return (s / nRef) * 100;
  });
  const peak = Math.max(...share, ...refShare); const ystep = peak > 20 ? 10 : peak > 8 ? 4 : 2; const ymax = Math.ceil((peak * 1.05) / ystep) * ystep;
  const m = { l: 40, r: 14, t: 10, b: 30 }, iw = w - m.l - m.r, ih = h - m.t - m.b;
  const X = (v: number) => m.l + (Math.min(v, xmax) / xmax) * iw; const Y = (v: number) => m.t + ih - (v / ymax) * ih;
  for (let t = 0; t <= ymax; t += ystep) { host.appendChild(svg("line", { x1: m.l, x2: m.l + iw, y1: Y(t), y2: Y(t), class: "grid" })); host.appendChild(svg("text", { x: m.l - 6, y: Y(t) + 4, class: "tick", "text-anchor": "end" }, t + "%")); }
  const xstep = xmax <= 0.2 ? 0.05 : 0.1;
  for (let t = 0; t <= xmax + 1e-9; t += xstep) host.appendChild(svg("text", { x: X(t), y: h - 10, class: "tick", "text-anchor": "middle" }, Math.round(t * 100) + "%"));
  bins.forEach((b, i) => {
    if (!b.n) return;
    const rect = svg("rect", { x: X(b.lo) + 0.5, y: Y(share[i]), width: Math.max(1, X(b.lo + width) - X(b.lo) - 1), height: m.t + ih - Y(share[i]), rx: 1.5, class: "bar" });
    rect.style.setProperty("--series", "var(--c-" + k + ")");
    rect.appendChild(svg("title", {}, (b.lo * 100).toFixed(0) + "–" + (i === bins.length - 1 ? "100" : ((b.lo + width) * 100).toFixed(0)) + "%: " + cnt(b.n) + " members (" + share[i].toFixed(1) + "%); MOCK reference " + refShare[i].toFixed(1) + "%"));
    host.appendChild(rect);
  });
  const step = bins.map((b, i) => X(b.lo) + "," + Y(refShare[i]) + " " + X(b.lo + width) + "," + Y(refShare[i])).join(" ");
  host.appendChild(svg("polyline", { points: step, class: "ref-line" }));
  const a = make("span", "lg-bar", "This cohort, PREVENT risk (n = " + cnt(vals.length) + ")"); a.style.setProperty("--series", "var(--c-" + k + ")");
  lg.append(a, make("span", "lg-ref", "MOCK reference adults, reference (n = " + nRef.toLocaleString("en-US") + ")"));
  const note = make("span", "", "Share of each group per 1-point risk band; last band includes higher risks"); lg.appendChild(note);
}

function chosenRules(): string[] { return RULES.filter((r) => (document.getElementById("rule-" + r.id) as HTMLInputElement | null)?.checked).map((r) => r.id); }

function renderCohortRules() {
  if (!cohort) return;
  const host = el<HTMLDivElement>("cohortRules"); const keep = chosenRules(); host.textContent = "";
  const counts = cohortScenario(cohort.ok, RULES.map((r) => r.id)).perRule;
  for (const r of RULES) {
    const lab = make("label", "toggle rule"); const c = make("input"); c.type = "checkbox"; c.id = "rule-" + r.id; c.checked = keep.includes(r.id);
    c.onchange = renderCohortScenario;
    lab.append(c, make("span", "", r.label + ": applies to " + cnt(counts[r.id] || 0) + " members (" + r.eligibility + ")"));
    host.appendChild(lab);
  }
}

function renderCohortScenario() {
  if (!cohort) return;
  const ids = chosenRules(); const table = el<HTMLTableElement>("cohortScenario"); const note = el<HTMLParagraphElement>("cohortScenarioNote");
  if (!ids.length) { table.textContent = ""; note.textContent = "Choose one or more changes."; return; }
  const s = cohortScenario(cohort.ok, ids);
  const rows = RISKS.map((k) => {
    const o = s.outcomes[k];
    return [LABEL[k], o.sumBefore[0].toFixed(1) + " · " + o.sumBefore[1].toFixed(1), o.sumAfter[0].toFixed(1) + " · " + o.sumAfter[1].toFixed(1),
      pct(o.medianBefore[0]) + " → " + pct(o.medianAfter[0]), pct(o.medianBefore[1]) + " → " + pct(o.medianAfter[1])];
  });
  tableRows(table, ["", "Predicted events now (PREVENT · calibrated)", "With the changes", "Median, PREVENT", "Median, calibrated (MOCK)"], rows);
  const parts = ["Members whose inputs change: " + cnt(s.changed) + " of " + cnt(cohort.ok.length) + "."];
  if (s.higher > 0) parts.push("With these changes PREVENT gives a higher total CVD risk for " + cnt(s.higher) + " of them.");
  if (ids.includes("statin")) parts.push("In PREVENT, statin use describes current treatment: setting it to yes recomputes the equation for a treated person with the same cholesterol; it is not an estimate of the effect of starting a statin.");
  parts.push("Predicted events are the sum of the members' 10-year risks; PREVENT is a prognostic equation, so differences are not treatment effects.");
  note.textContent = parts.join(" ");
}

function memberStatus(m: Member): string {
  if (m.status === "ok") return "";
  return (m.status === "missing" ? "Missing: " : "Out of range: ") + m.problems.map(fieldLabel).join(", ");
}

function renderPatientTable() {
  const card = el<HTMLDivElement>("patientListCard"); card.hidden = !cohort || !showPatients();
  if (!cohort || card.hidden) return;
  const sorted = cohort.members.slice().sort((a, b) => ((b.prevent.total_cvd ?? -1) - (a.prevent.total_cvd ?? -1)));
  const table = el<HTMLTableElement>("patientTable"); table.textContent = "";
  const thead = make("thead"); const hr = make("tr");
  for (const h of ["Member", "Age", "Sex", "Total CVD (PREVENT · calibrated)", "ASCVD", "Heart failure", "Note"]) hr.appendChild(make("th", "", h));
  thead.appendChild(hr); table.appendChild(thead);
  const body = make("tbody");
  for (const m of sorted.slice(0, shownRows)) {
    const tr = make("tr", "click"); tr.tabIndex = 0;
    const cells = [m.key, String(m.inputs.age ?? "–"), m.inputs.sex === "male" ? "M" : "F"].concat(RISKS.map((k) => m.prevent[k] === null ? "–" : pct(m.prevent[k]) + " · " + pct(m.ucsf[k]))).concat([memberStatus(m)]);
    cells.forEach((v, i) => tr.appendChild(make("td", i >= 3 && i <= 5 ? "num" : "", v)));
    const open = () => openMember(m); tr.onclick = open; tr.onkeydown = (e) => { if (e.key === "Enter") open(); };
    body.appendChild(tr);
  }
  table.appendChild(body);
  const more = el<HTMLButtonElement>("morePatients"); more.hidden = sorted.length <= shownRows;
  more.textContent = "Show more (" + (sorted.length - shownRows) + " not shown)";
}

function openMember(m: Member) {
  if (!cohort) return;
  const p: any = { ...m.inputs }; const miss: Record<string, boolean> = {};
  for (const k of m.problems) if (m.status === "missing") { miss[k] = true; if (k !== "sex") p[k] = typeof p[k] === "boolean" ? false : ""; }
  origin = { name: cohort.meta.name, key: m.key };
  el<HTMLSelectElement>("sexFilter").value = "cohort";
  newPatient(p, labelAll("Cohort \"" + cohort.meta.name + "\" · " + m.key));
  missing = miss; updateFieldNotes(); renderSourceRows();
}

// ── receiving data from a Biorouter chat (handoff.mjs) ──────────────────────
// The address carries only a pointer to a one-time local WebSocket (allowed by the app's
// content security policy); the data are never in the address or the browser history.
function receive(payload: any) {
  if (!payload || payload.v !== 1) throw new Error("unknown format");
  if (payload.kind === "cohort") {
    const c = buildCohort(payload); loadCohort(c); showTab("cohortview");
    toast("Cohort received: " + c.members.length.toLocaleString("en-US") + " members, PREVENT computed for " + c.ok.length.toLocaleString("en-US") + ".");
  } else if (payload.kind === "patient") {
    const n = normalize(payload.inputs || {}); const src: SourceMap = labelAll(String(payload.source || "Biorouter chat"));
    if (payload.sources && typeof payload.sources === "object") for (const [k, v] of Object.entries(payload.sources)) src[k] = v as any;
    origin = null; newPatient(n.inputs, src);
    missing = {}; for (const k of n.missing) missing[k] = true; updateFieldNotes(); renderSourceRows();
    toast("Patient received" + (payload.key ? " (" + payload.key + ")" : "") + (n.missing.length ? "; missing: " + n.missing.map(fieldLabel).join(", ") : "") + ".");
  } else throw new Error("unknown kind " + String(payload.kind));
}
function readHandoff() {
  const m = /^#handoff=(ws:\/\/127\.0\.0\.1:\d{2,5}\/[0-9a-f]{32,64})$/.exec(location.hash);
  if (!m) return;
  history.replaceState(null, "", location.pathname + location.search);
  toast("Receiving data from the Biorouter chat…");
  let got = false; const ws = new WebSocket(m[1]);
  ws.onmessage = (ev) => { got = true; try { receive(JSON.parse(String(ev.data))); } catch (e) { toast("Could not read the data: " + String((e as any)?.message || e)); } ws.close(); };
  ws.onerror = () => { if (!got) toast("The sender is no longer available: ask the chat to send the data again."); };
}

// ── page ────────────────────────────────────────────────────────────────────
function renderOutputs() {
  renderRiskCards(el<HTMLDivElement>("riskGrid"), patient);
  renderStatus("inputStatus", patient);
  renderSourceRows();
  renderOrigin();
  renderMini();
  renderPosition();
  renderScenarioControls();
  renderScenario();
  renderProjection();
}
function renderAll() { renderForm(); renderOutputs(); }

const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>("[data-tab]"));
function showTab(id: string) {
  for (const t of tabs) t.setAttribute("aria-selected", String(t.dataset.tab === id));
  for (const s of Array.from(document.querySelectorAll<HTMLElement>(".section"))) s.classList.toggle("active", s.id === id);
  renderOutputs(); if (id === "cohortview") renderCohort();
}
for (const t of tabs) t.onclick = () => showTab(t.dataset.tab || "patient");

function newPatient(p: PreventInputs, src: SourceMap) {
  patient = { ...p }; scenario = { ...p }; sources = src; missing = {}; scenarios = [];
  br.state.set("/patient", patient); br.state.set("/scenarios", scenarios);
  renderAll(); showTab("patient");
}

el<HTMLSelectElement>("sexFilter").onchange = renderOutputs;
el<HTMLButtonElement>("demoPatient").onclick = () => { origin = null; newPatient(demo, labelAll("Synthetic demo value")); };
el<HTMLButtonElement>("manualEntry").onclick = () => {
  const blank: any = { ...patient };
  for (const f of FIELDS) { if (f.kind === "number") blank[f.key] = ""; if (f.kind === "toggle") blank[f.key] = false; }
  origin = null; newPatient(blank, labelAll("Entered by hand"));
  el<HTMLInputElement>("input-age").focus();
};
el<HTMLButtonElement>("backToCohort").onclick = () => showTab("cohortview");
for (const b of Array.from(document.querySelectorAll<HTMLButtonElement>("[data-goto]"))) b.onclick = () => { showTab(b.dataset.goto || "about"); window.scrollTo({ top: 0 }); };
el<HTMLButtonElement>("demoCohort").onclick = () => { loadCohort(buildCohort(demoPayload())); renderCohort(); };
el<HTMLButtonElement>("demoCohortTop").onclick = () => { loadCohort(buildCohort(demoPayload())); showTab("cohortview"); };
el<HTMLInputElement>("showPatients").onchange = () => { renderCohort(); renderOutputs(); };
el<HTMLSelectElement>("cohortOutcome").onchange = renderCohortChart;
el<HTMLButtonElement>("morePatients").onclick = () => { shownRows += 100; renderPatientTable(); };
el<HTMLButtonElement>("resetScenario").onclick = () => { scenario = { ...patient }; renderScenarioControls(); renderScenario(); };
el<HTMLButtonElement>("saveScenario").onclick = () => {
  if (scenarios.length >= 3) scenarios.shift();
  const saved: Scenario = { name: describe(scenario, patient), patient: { ...scenario }, risks: computeRisks(scenario) };
  scenarios.push(saved); br.state.set("/scenarios", scenarios); br.signals.emit("scenario_saved", saved); renderSaved();
};
el<HTMLButtonElement>("clearScenarios").onclick = () => { scenarios = []; br.state.set("/scenarios", scenarios); renderSaved(); };
el<HTMLButtonElement>("loadOmop").onclick = async () => {
  const personId = el<HTMLInputElement>("personId").value.trim(); const asOf = el<HTMLInputElement>("asOfDate").value;
  const msg = el<HTMLParagraphElement>("omopPrivacy");
  if (!personId || !asOf) { msg.textContent = "Enter a person_id and an as-of date. They are sent only to your local app agent."; return; }
  const rules = OMOP_RULES; const outputs = Object.keys(patient);
  const request = { person_id: personId, as_of_date: asOf, omop_rules: rules, required_outputs: outputs };
  msg.textContent = "Loading from OMOP…";
  try { await br.call("load_patient_from_omop", request, { timeoutMs: 180000 }); msg.textContent = "Loaded. Each value shows its source and date."; }
  catch (e) { msg.textContent = "Could not load from OMOP: " + String((e as any)?.message || e); }
};
// Cohort selection chat: each message goes to the app's agent; its answer streams into a new bubble.
const COHORT_NOTE = "\n\n(Cohort selection tab. Help define the cohort: purpose, criteria, codes, as-of date. Show the definition and its size from aggregate SQL and wait for confirmation. Aggregates only; counts from 1 to 10 as <11. Do not compute PREVENT: the app computes it. If the OMOP tools are not available in this app session, say so plainly and suggest running the selection in a Biorouter chat with the skill prevent-calculator-handoff.)";
async function sendCohortMessage() {
  const box = el<HTMLTextAreaElement>("cohortQuestion"); const q = box.value.trim(); if (!q) return;
  box.value = "";
  const log = el<HTMLDivElement>("chatLog"); log.hidden = false;
  log.appendChild(make("div", "msg user", q));
  const reply = make("div", "msg agent", "…"); log.appendChild(reply); log.scrollTop = log.scrollHeight;
  try { await br.run(q + COHORT_NOTE, reply, { timeoutMs: 1200000 }); }
  catch (e) { reply.textContent = "The agent could not answer: " + String((e as any)?.message || e); }
  log.scrollTop = log.scrollHeight;
}
el<HTMLButtonElement>("askCohort").onclick = sendCohortMessage;
el<HTMLTextAreaElement>("cohortQuestion").onkeydown = (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); sendCohortMessage(); } };
let resizeTimer = 0;
window.addEventListener("resize", () => { window.clearTimeout(resizeTimer); resizeTimer = window.setTimeout(() => { renderOutputs(); renderCohortChart(); }, 150); });
window.addEventListener("hashchange", readHandoff);

br.actions.register("display_loaded_patient", async (args) => {
  const a = (args || {}) as any;
  const loaded = a.patient ? { ...patient, ...a.patient } : patient;
  origin = null; newPatient(loaded, a.sources || {});
  missing = a.missing || {}; updateFieldNotes(); renderSourceRows();
  const missingKeys = Object.keys(missing).filter((k) => missing[k]);
  return { ok: true, missing: missingKeys };
});
br.actions.register("display_cohort_result", async (args) => { br.state.set("/cohort", args || null); return { ok: true }; });

el<HTMLElement>("calibrationFacts").textContent = (ucsfRecalibration as any).mock ? "MOCK parameters: illustrative only, not fitted to patient data." : RISKS.map((k) => {
  const cv = (ucsfRecalibration as any).outcomes[k].cross_validated;
  return LABEL[k] + ": observed/expected " + cv.oe_ratio.toFixed(2) + ", calibration slope " + cv.calibration_slope.toFixed(2) + ", C " + cv.c_statistic.toFixed(3);
}).join("; ") + ".";
renderAll();
renderCohort();
readHandoff();
