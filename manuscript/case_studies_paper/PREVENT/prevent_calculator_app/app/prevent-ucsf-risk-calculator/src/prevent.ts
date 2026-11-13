import { PREVENT_COEFFICIENTS, REFERENCE_COHORT, UCSF_RECALIBRATION } from "./preventData";

export type Sex = "female" | "male";
export type RiskKey = "total_cvd" | "ascvd" | "hf";
export type InputKey = "age" | "sex" | "sbp" | "total_cholesterol_mg_dl" | "hdl_mg_dl" | "bmi" | "egfr" | "diabetes" | "current_smoker" | "bp_treatment" | "statin";

export interface PreventInputs {
  age: number; sex: Sex; sbp: number; total_cholesterol_mg_dl: number; hdl_mg_dl: number; bmi: number; egfr: number;
  diabetes: boolean | number; current_smoker: boolean | number; bp_treatment: boolean | number; statin: boolean | number;
}

export interface RiskResult { total_cvd: number | null; ascvd: number | null; hf: number | null; valid: boolean; problems: string[]; }

export const RANGE_LABELS: Record<InputKey, string> = {
  age: "30-79 years", sex: "female or male", sbp: "90-180 mmHg", total_cholesterol_mg_dl: "130-320 mg/dL",
  hdl_mg_dl: "20-100 mg/dL", bmi: "18.5-39.9 kg/m2", egfr: "15-140 mL/min/1.73 m2",
  diabetes: "yes/no", current_smoker: "yes/no", bp_treatment: "yes/no", statin: "yes/no"
};

export function toFlag(v: boolean | number | string | undefined | null): number {
  return v === true || v === 1 || v === "1" || v === "true" || v === "yes" ? 1 : 0;
}

function checkRange(name: InputKey, value: unknown, lo: number, hi: number, problems: string[]) {
  const n = Number(value);
  if (!Number.isFinite(n)) { problems.push(`${name} must be numeric`); return; }
  if (n < lo || n > hi) problems.push(`${name} ${n} is outside PREVENT range ${RANGE_LABELS[name]}`);
}

export function validateInputs(p: Partial<PreventInputs>): string[] {
  const problems: string[] = [];
  const req: InputKey[] = ["age","sex","sbp","total_cholesterol_mg_dl","hdl_mg_dl","bmi","egfr","diabetes","current_smoker","bp_treatment","statin"];
  for (const k of req) if ((p as any)[k] === undefined || (p as any)[k] === null || (p as any)[k] === "") problems.push(`${k} is missing`);
  if (p.sex !== "female" && p.sex !== "male") problems.push("sex must be female or male");
  checkRange("age", p.age, 30, 79, problems); checkRange("sbp", p.sbp, 90, 180, problems);
  checkRange("total_cholesterol_mg_dl", p.total_cholesterol_mg_dl, 130, 320, problems); checkRange("hdl_mg_dl", p.hdl_mg_dl, 20, 100, problems);
  checkRange("bmi", p.bmi, 18.5, 39.9, problems); checkRange("egfr", p.egfr, 15, 140, problems);
  return problems;
}

export function terms(p: PreventInputs): Record<string, number> {
  const age = (Number(p.age) - 55) / 10;
  const nonHdl = (Number(p.total_cholesterol_mg_dl) - Number(p.hdl_mg_dl)) * 0.02586 - 3.5;
  const hdl = (Number(p.hdl_mg_dl) * 0.02586 - 1.3) / 0.3;
  const sbpLow = (Math.min(Number(p.sbp), 110) - 110) / 20;
  const sbpHigh = (Math.max(Number(p.sbp), 110) - 130) / 20;
  const bmiLow = (Math.min(Number(p.bmi), 30) - 25) / 5;
  const bmiHigh = (Math.max(Number(p.bmi), 30) - 30) / 5;
  const egfrLow = (Math.min(Number(p.egfr), 60) - 60) / -15;
  const egfrHigh = (Math.max(Number(p.egfr), 60) - 90) / -15;
  const diabetes = toFlag(p.diabetes), smoker = toFlag(p.current_smoker), bpTx = toFlag(p.bp_treatment), statin = toFlag(p.statin);
  return {
    "Age, per 10 years": age, "non-HDL-C per 1 mmol/L": nonHdl, "HDL-C per 0.3 mmol/L": hdl,
    "SBP <110 per 20 mmHg": sbpLow, "SBP ≥110 per 20 mmHg": sbpHigh, "Diabetes": diabetes, "Current smoking": smoker,
    "BMI <30, per 5 kg/m2": bmiLow, "BMI 30+, per 5 kg/m2": bmiHigh, "eGFR <60, per -15 ml": egfrLow, "eGFR 60+, per -15 ml": egfrHigh,
    "Anti-hypertensive use": bpTx, "Statin use": statin, "Treated SBP ≥110 mm Hg per 20 mm Hg": bpTx * sbpHigh, "Treated non-HDL-C": statin * nonHdl,
    "Age per 10yr * non-HDL-C per 1 mmol/L": age * nonHdl, "Age per 10yr * HDL-C per 1 mml/L": age * hdl,
    "Age per 10yr * SBP ≥110 mm Hg per 20 mmHg": age * sbpHigh, "Age per 10yr * diabetes": age * diabetes, "Age per 10yr * current smoking": age * smoker,
    "Age per 10yr * BMI 30+ per 5 kg/m2": age * bmiHigh, "Age per 10yr * eGFR <60, per -15 ml": age * egfrLow, "Constant": 1
  };
}

export function preventRisk(p: PreventInputs, risk: RiskKey): number {
  const t = terms(p); const coeffs = (PREVENT_COEFFICIENTS as any).coefficients[risk][p.sex]; let lp = 0;
  for (const [name, beta] of Object.entries(coeffs)) lp += Number(beta) * (t[name] ?? 0);
  return 1 / (1 + Math.exp(-lp));
}

export function computeRisks(p: PreventInputs): RiskResult {
  const problems = validateInputs(p);
  if (problems.length) return { total_cvd: null, ascvd: null, hf: null, valid: false, problems };
  return { total_cvd: preventRisk(p, "total_cvd"), ascvd: preventRisk(p, "ascvd"), hf: preventRisk(p, "hf"), valid: true, problems: [] };
}

/** UCSF-calibrated 10-year risk from the published PREVENT risk: 1 - s0 ^ exp(beta * (logit(p) - x_mean)).
 *  The transformation is monotone, so the percentile in the reference cohort is the same on both scales. */
export function ucsfCalibrated(risk: RiskKey, p: number | null | undefined): number | null {
  if (p === null || p === undefined || !Number.isFinite(p) || p <= 0 || p >= 1) return null;
  const c = (UCSF_RECALIBRATION as any).outcomes[risk];
  const x = Math.log(p / (1 - p));
  return 1 - Math.pow(c.s0_10y, Math.exp(c.beta * (x - c.x_mean)));
}

export function riskPercentile(risk: RiskKey, value: number, sex: Sex | "overall" = "overall"): number {
  const bins = ((REFERENCE_COHORT as any).distribution[risk] || {})[sex] || [];
  const n = Number((REFERENCE_COHORT as any).n[sex]) || bins.reduce((s: number, b: any) => s + Number(b.n || 0), 0);
  if (!bins.length || !Number.isFinite(value) || n <= 0) return NaN;
  let below = 0;
  for (const b of bins) {
    const lo = Number(b.lo), hi = Number(b.hi), count = Number(b.n);
    if (value >= hi) { below += count; continue; }
    if (value < lo) break;
    below += count * Math.min(1, Math.max(0, (value - lo) / Math.max(hi - lo, 1e-9))); break;
  }
  return Math.max(0, Math.min(100, (below / n) * 100));
}

export function formatPct(x: number | null | undefined, digits = 1): string {
  return x === null || x === undefined || !Number.isFinite(x) ? "-" : `${(x * 100).toFixed(digits)}%`;
}

export function projectAges(p: PreventInputs, years = 10): Array<{age:number,total_cvd:number,ascvd:number,hf:number}> {
  const out: Array<{age:number,total_cvd:number,ascvd:number,hf:number}> = [];
  const start = Math.ceil(Number(p.age)); const last = Math.min(79, start + years);
  for (let age = start; age <= last; age++) { const q = {...p, age}; const r = computeRisks(q); if (r.valid && r.total_cvd !== null && r.ascvd !== null && r.hf !== null) out.push({age,total_cvd:r.total_cvd,ascvd:r.ascvd,hf:r.hf}); }
  return out;
}

export function ckdepi2021(scrMgDl: number, age: number, sex: Sex): number {
  const female = sex === "female"; const kappa = female ? 0.7 : 0.9; const alpha = female ? -0.241 : -0.302; const ratio = scrMgDl / kappa;
  return 142 * Math.pow(Math.min(ratio, 1), alpha) * Math.pow(Math.max(ratio, 1), -1.2) * Math.pow(0.9938, age) * (female ? 1.012 : 1);
}

export const referenceCohort = REFERENCE_COHORT;
export const ucsfRecalibration = UCSF_RECALIBRATION;
export const coefficients = PREVENT_COEFFICIENTS;
