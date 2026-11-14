/**
 * What **Settings > Models > Speech models** asks of the daemon, and what it is waiting for
 * (DD-66, PR-18).
 *
 * The `llamaServerStore` pattern, split in two. The status itself, an install's progress
 * included, is the dictation status store's (`components/dictation/dictationStore.ts`): one
 * `GET /dictation/status` per window, re-read every 1.5 s only while `install_job` runs and never
 * otherwise. That store is a module singleton, so leaving Settings in the middle of a download
 * loses nothing: the poll is tied to the install, not to the page, and the page shows the
 * progress again when it comes back. This module holds the rest, also at module level for the
 * same reason: the request a row has in flight, the refusal it got, the language being saved,
 * the last Rescan result and Test speed run, and the times Test speed measured from this window,
 * per model (the Speed row shows the one for the model in use, and MSG-TURBO-CPU names one; the
 * status carries no bench record).
 *
 * Every request goes through `dictationModelsApi`, which sends `userActionHeaders()` on install,
 * cancel, delete, select and rescan. After a change it reads the status again, so the rows show
 * what the daemon holds, never what this module expected.
 *
 * The pure functions below decide what each row offers. They follow the daemon's own precedence
 * for an install refusal (`install_allowance` in `routes/dictation.rs`): MSG-NO-ENGINE, MSG-POLICY,
 * the processor's sentence, MSG-POLICY-DOWNLOADS, MSG-POLICY-MODELS, MSG-INSTALL-AGENT,
 * MSG-INSTALL-LOCKED. The status answers that question for the class default only; another model
 * differs from it by `allowed_models` alone, which is the one check repeated here.
 */
import { useSyncExternalStore } from 'react';
import { dictationMessage, type DictationMessageId } from '../../dictation/copy';
import {
  dictationModelsApi,
  type BenchResult,
  type DictationModelsApi,
  type DictationResult,
  type DictationStatus,
} from '../../dictation/dictationApi';
import { isDictationRunning } from '../../dictation/dictationSession';
import { getDictationStatusSnapshot, refreshDictationStatus } from '../../dictation/dictationStore';

export type SpeechModel = DictationStatus['models'][number];
export type SpeechModelActionKind = 'install' | 'cancel' | 'remove' | 'select';

/** A sentence the section shows, with the catalog id it came from when it has one. */
export interface SpeechNotice {
  text: string;
  id?: DictationMessageId;
  tone: 'neutral' | 'success' | 'warning';
}

export interface SpeechModelsSnapshot {
  /** The request each model has in flight. */
  pending: Readonly<Record<string, SpeechModelActionKind>>;
  /** The last refusal per model, as the daemon worded it. */
  errors: Readonly<Record<string, string>>;
  /** Which request each refusal in `errors` answered: a refused **Use** is moot once it is in use. */
  refused: Readonly<Record<string, SpeechModelActionKind>>;
  /**
   * Per model, the failed install the daemon still reports after that model was installed some
   * other way (Rescan, the CLI) and then removed from this window. The daemon keeps its last job
   * until the next install, so without this the row said "The install was cancelled." again after
   * Remove, about a job that ended before the model was installed [M]. A key from {@link failureKey}.
   */
  retiredFailures: Readonly<Record<string, string>>;
  language: { pending: string | null; error: string | null };
  rescan: { running: boolean; notice: SpeechNotice | null };
  /**
   * The last Test speed run: the model that was in use when it started, and its refusal. What it
   * measured is in `measured`, so the Speed row shows the time of the model in use, never of the
   * model that was in use when the last run ended (see {@link speedLine}).
   */
  bench: { running: boolean; model: string | null; error: string | null };
  /** What Test speed measured from this window, per model (MSG-BENCH, MSG-TURBO-CPU). */
  measured: Readonly<Record<string, BenchResult>>;
}

const INITIAL: SpeechModelsSnapshot = {
  pending: {},
  errors: {},
  refused: {},
  retiredFailures: {},
  language: { pending: null, error: null },
  rescan: { running: false, notice: null },
  bench: { running: false, model: null, error: null },
  measured: {},
};

let snapshot: SpeechModelsSnapshot = INITIAL;
const listeners = new Set<() => void>();
let api: DictationModelsApi = dictationModelsApi;

function publish(next: Partial<SpeechModelsSnapshot>): void {
  snapshot = { ...snapshot, ...next };
  for (const listener of [...listeners]) listener();
}

export function getSpeechModelsSnapshot(): SpeechModelsSnapshot {
  return snapshot;
}

export function subscribeSpeechModels(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useSpeechModels(): SpeechModelsSnapshot {
  return useSyncExternalStore(subscribeSpeechModels, getSpeechModelsSnapshot);
}

const without = <T>(record: Readonly<Record<string, T>>, key: string): Record<string, T> => {
  const next = { ...record };
  delete next[key];
  return next;
};

/**
 * A refusal as a person reads it: the daemon's sentence, or MSG-SETTINGS-REFUSED.
 *
 * One exception: the daemon's MSG-LANGUAGE-EN (`409 languageUnsupported`) names the model by its
 * catalog id (`DictationError::EnglishOnly` in `crates/biorouter/src/dictation/service.rs`, the
 * terminal's form), and this page names every model by its display name. `model` is the display
 * name of the English only model the refusal is about, and the catalog sentence is filled with it.
 */
export function refusalText(
  result: Extract<DictationResult<unknown>, { ok: false }>,
  model?: string
): string {
  if (model && result.status === 409 && result.error.code === 'languageUnsupported') {
    return dictationMessage('MSG-LANGUAGE-EN', { model });
  }
  if (result.error.message) return result.error.message;
  return dictationMessage('MSG-SETTINGS-REFUSED', {
    code: result.error.code,
    status: String(result.status),
  });
}

const UNREACHABLE = (): string => dictationMessage('MSG-SETTINGS-UNREACHABLE');

/**
 * One model's request: at most one at a time per model, and its refusal kept on the row. After
 * it, the status is read again so the rows show what the daemon now holds; `settled` says whether
 * that read already reflects the change, and when it does not (a read that was in flight before
 * the request answered, which the status store shares rather than repeats) one more is made.
 */
async function forModel<T>(
  model: string,
  kind: SpeechModelActionKind,
  call: () => Promise<DictationResult<T>>,
  settled: (status: DictationStatus | null, result: DictationResult<T>) => boolean = () => true,
  name?: string
): Promise<DictationResult<T> | null> {
  if (snapshot.pending[model]) return null;
  publish({
    pending: { ...snapshot.pending, [model]: kind },
    errors: without(snapshot.errors, model),
    refused: without(snapshot.refused, model),
  });
  const refuse = (text: string) =>
    publish({
      errors: { ...snapshot.errors, [model]: text },
      refused: { ...snapshot.refused, [model]: kind },
    });
  let result: DictationResult<T> | null = null;
  try {
    result = await call();
    if (!result.ok) refuse(refusalText(result, name));
  } catch {
    refuse(UNREACHABLE());
  }
  const status = await refreshDictationStatus();
  if (result?.ok && !settled(status, result)) await refreshDictationStatus();
  publish({ pending: without(snapshot.pending, model) });
  return result;
}

/**
 * Start (or resume) an install; the daemon answers 202 and reports progress in the status, which
 * the status store then re-reads every 1.5 s until the job ends. A read that shows no running job
 * after a 202 started before the install did, and would leave the store polling nothing.
 */
export async function installSpeechModel(model: string): Promise<void> {
  await forModel(
    model,
    'install',
    () => api.install(model),
    (status, result) =>
      result.status !== 202 ||
      jobRunning(status?.install_job ?? null) ||
      !!status?.models.find((m) => m.id === model)?.installed
  );
}

/** Stop a running install; its partial file stays, so the next install resumes. */
export async function cancelSpeechModelInstall(model: string): Promise<void> {
  await forModel(model, 'cancel', () => api.cancel(model));
}

/** What tells one install job's ending from another's: the status gives a job no id. */
export const failureKey = (job: InstallJob): string =>
  [job.model, job.bytes_done, job.bytes_total, job.error ?? ''].join('\n');

export async function removeSpeechModel(model: string): Promise<void> {
  // A failed install the status reports for a model that is installed now ended before that
  // install, so it is history, and stays history once the model is removed.
  const before = getDictationStatusSnapshot().status;
  const job = before?.install_job;
  if (
    job?.phase === 'failed' &&
    job.model === model &&
    before?.models.some((m) => m.id === model && m.installed)
  ) {
    publish({ retiredFailures: { ...snapshot.retiredFailures, [model]: failureKey(job) } });
  }
  await forModel(model, 'remove', () => api.remove(model));
}

/**
 * Use `model`. `name` is its display name: a `.en` model refuses the language in use with
 * MSG-LANGUAGE-EN, and the row says that sentence with the name the row shows.
 */
export async function selectSpeechModel(model: string, name?: string): Promise<void> {
  const result = await forModel(model, 'select', () => api.select({ model }), undefined, name);
  // A language refusal was about the model that was in use; it no longer describes this one.
  if (result?.ok && snapshot.language.error) {
    publish({ language: { ...snapshot.language, error: null } });
  }
}

/**
 * Save the language (`BIOROUTER_DICTATION_LANGUAGE`); a `.en` model refuses others with
 * MSG-LANGUAGE-EN, worded with the display name of the model the daemon has in use, which the
 * status read after the refusal names (the page's own copy of the status may be older).
 */
export async function setDictationLanguage(language: string): Promise<void> {
  if (snapshot.language.pending) return;
  publish({ language: { pending: language, error: null } });
  let error: string | null = null;
  let refusal: Extract<DictationResult<unknown>, { ok: false }> | null = null;
  try {
    const result = await api.select({ language });
    if (!result.ok) refusal = result;
  } catch {
    error = UNREACHABLE();
  }
  const status = await refreshDictationStatus();
  if (refusal) {
    const active = status ? activeSpeechModel(status) : null;
    error = refusalText(
      refusal,
      active && englishOnly(active.id) ? active.display_name : undefined
    );
  }
  if (refusal || error) {
    publish({ language: { pending: null, error } });
    return;
  }
  // A refused Use (MSG-LANGUAGE-EN on a `.en` row: "or set the language to English") was about
  // the language before this one, so it no longer says what Use would answer.
  const errors = { ...snapshot.errors };
  const refused = { ...snapshot.refused };
  for (const [id, kind] of Object.entries(refused)) {
    if (kind !== 'select') continue;
    delete errors[id];
    delete refused[id];
  }
  publish({ language: { pending: null, error: null }, errors, refused });
}

/**
 * Rescan: the air gapped path (DD-24). Files a person put in the model folder are identified by
 * their SHA-256 and placed; the route takes no argument, so no path crosses HTTP.
 */
export async function rescanSpeechModels(
  folder: string,
  displayName: (id: string) => string = (id) => id
): Promise<void> {
  if (snapshot.rescan.running) return;
  publish({ rescan: { running: true, notice: null } });
  let notice: SpeechNotice;
  try {
    const result = await api.rescan();
    notice = result.ok
      ? rescanNotice(result.data.added.map(displayName), result.data.unrecognized, folder)
      : { text: refusalText(result), tone: 'warning' };
  } catch {
    notice = { text: UNREACHABLE(), tone: 'warning' };
  }
  await refreshDictationStatus();
  publish({ rescan: { running: false, notice } });
}

/** What a Rescan found, in the catalog's words. */
export function rescanNotice(
  added: readonly string[],
  unrecognized: readonly string[],
  folder: string
): SpeechNotice {
  const parts: string[] = [];
  if (added.length > 0)
    parts.push(dictationMessage('MSG-RESCAN-ADDED', { models: added.join(', ') }));
  if (unrecognized.length > 0) {
    parts.push(dictationMessage('MSG-RESCAN-UNKNOWN', { names: unrecognized.join(', ') }));
  }
  if (parts.length === 0) {
    return {
      id: 'MSG-RESCAN-NONE',
      text: dictationMessage('MSG-RESCAN-NONE', { folder }),
      tone: 'neutral',
    };
  }
  return {
    id: added.length > 0 ? 'MSG-RESCAN-ADDED' : 'MSG-RESCAN-UNKNOWN',
    text: parts.join(' '),
    tone: unrecognized.length > 0 ? 'warning' : 'success',
  };
}

/**
 * Test speed (`POST /dictation/bench`): the active model's real time factor, recorded by the
 * daemon. `model` is the model in use as the page shows it, which a refusal is kept against.
 */
export async function testDictationSpeed(model: string | null = null): Promise<void> {
  // A bench decodes twice on the worker a dictation uses, and the supervisor serves those decodes
  // round robin beside a dictation's segments, one at a time, so a segment could wait behind one.
  // It never starts while this window dictates.
  if (snapshot.bench.running || isDictationRunning()) return;
  publish({ bench: { running: true, model, error: null } });
  try {
    const result = await api.bench();
    if (result.ok) {
      publish({
        bench: { running: false, model: result.data.model, error: null },
        measured: { ...snapshot.measured, [result.data.model]: result.data },
      });
    } else {
      publish({ bench: { running: false, model, error: refusalText(result) } });
    }
  } catch {
    publish({ bench: { running: false, model, error: UNREACHABLE() } });
  }
  // The record changes this model's suitability (a factor above 0.5 is `too_slow`).
  await refreshDictationStatus();
}

/** The model in use, when it is installed: what Test speed and the Language row are about. */
export function activeSpeechModel(status: DictationStatus): SpeechModel | null {
  return status.models.find((m) => m.id === status.active_model && m.installed) ?? null;
}

/**
 * What the Speed row says: nothing while a run is in progress, then the refusal or the time Test
 * speed measured for the model in use. A time measured for another model, one no longer in use or
 * no longer installed, is not shown; it comes back when that model is in use again.
 */
export function speedLine(
  status: DictationStatus,
  models: SpeechModelsSnapshot = snapshot
): { error: string } | { result: BenchResult; model: SpeechModel } | null {
  const active = activeSpeechModel(status);
  if (models.bench.running || !active) return null;
  if (models.bench.error && models.bench.model === active.id) {
    return { error: models.bench.error };
  }
  const result = models.measured[active.id];
  return result ? { result, model: active } : null;
}

// ── what each row offers ──────────────────────────────────────────────────────────────────

export interface Allowance {
  allowed: boolean;
  reason: string | null;
}

/** MSG-POLICY-MODELS as the daemon words it: the allowed ids, or `none`. */
export function policyModelsSentence(allowed: readonly string[]): string {
  return dictationMessage('MSG-POLICY-MODELS', {
    models: allowed.length > 0 ? allowed.join(', ') : 'none',
  });
}

const excludedByPolicy = (status: DictationStatus, id: string): boolean =>
  !!status.policy.allowed_models && !status.policy.allowed_models.includes(id);

/**
 * Whether this caller may install `model`. `status.install` is the daemon's answer for the class
 * default; when the only thing that refused the default is `allowed_models`, the checks after it
 * (the proof, an operator's lock) were never asked, and a model the list admits is offered: the
 * daemon asks them itself on the request, and a refusal shows on the row.
 */
export function installAllowance(status: DictationStatus, model: SpeechModel): Allowance {
  const models = status.policy.allowed_models;
  const modelsRefusal = models ? policyModelsSentence(models) : null;
  const onlyThePolicyList =
    !status.install.allowed && modelsRefusal !== null && status.install.reason === modelsRefusal;
  if (!status.install.allowed && !onlyThePolicyList) {
    return { allowed: false, reason: status.install.reason };
  }
  if (excludedByPolicy(status, model.id)) return { allowed: false, reason: modelsRefusal };
  return { allowed: true, reason: null };
}

/**
 * An install refusal that holds for every model and that the policy note does not already say:
 * MSG-NO-ENGINE, the processor's sentence, MSG-INSTALL-AGENT or MSG-INSTALL-LOCKED. Shown once
 * above the rows, in place of every Install button.
 */
export function sectionInstallRefusal(status: DictationStatus): string | null {
  if (status.install.allowed || !status.install.reason) return null;
  const reason = status.install.reason;
  const policy = status.policy;
  const policySentences = [
    policy.enabled.reason,
    policy.install.reason,
    policy.allowed_models ? policyModelsSentence(policy.allowed_models) : null,
  ];
  return policySentences.includes(reason) ? null : reason;
}

export type InstallJob = NonNullable<DictationStatus['install_job']>;

export const jobRunning = (job: DictationStatus['install_job']): job is InstallJob =>
  !!job && (job.phase === 'downloading' || job.phase === 'verifying');

export interface SpeechModelRow {
  model: SpeechModel;
  active: boolean;
  /** This model's install is running. */
  installing: InstallJob | null;
  /** Its last install failed and it is still not installed: the daemon's sentence. */
  failure: string | null;
  /** The last refusal of a request from this window. */
  error: string | null;
  allowance: Allowance;
  /** Install, Update and Use are offered; another install, or dictation being off, holds them. */
  canInstall: boolean;
  canUpdate: boolean;
  canUse: boolean;
  /** Installed in this account's own folder, so Remove reaches it. */
  canRemove: boolean;
  /** Installed in a folder Remove does not reach (DD-19). */
  elsewhere: 'system' | 'managed' | 'env' | null;
  /** Managed policy's `allowed_models` leaves it out: it may not be installed, chosen or used. */
  notAllowed: boolean;
  pending: SpeechModelActionKind | null;
}

export function speechModelRows(
  status: DictationStatus,
  models: SpeechModelsSnapshot = snapshot
): SpeechModelRow[] {
  const job = status.install_job;
  const running = jobRunning(job) ? job : null;
  const enabled = status.policy.enabled.allowed;
  return status.models.map((model) => {
    const allowance = installAllowance(status, model);
    const pending = models.pending[model.id] ?? null;
    const free = !running && !pending;
    const usable = model.installed && !excludedByPolicy(status, model.id);
    const elsewhere =
      model.installed && model.location && model.location !== 'user' ? model.location : null;
    const active = status.active_model === model.id && model.installed;
    return {
      model,
      active,
      installing: running && running.model === model.id ? running : null,
      failure:
        job &&
        job.phase === 'failed' &&
        job.model === model.id &&
        !model.installed &&
        models.retiredFailures[model.id] !== failureKey(job)
          ? job.error
          : null,
      // A refused Use says nothing once the model is in use, however it got there.
      error:
        active && models.refused[model.id] === 'select' ? null : (models.errors[model.id] ?? null),
      allowance,
      canInstall: enabled && !model.installed && allowance.allowed && free,
      canUpdate: enabled && model.installed && model.superseded && allowance.allowed && free,
      canUse: enabled && usable && status.active_model !== model.id && !pending,
      // Removing is never refused by policy (the delete route says so); only a copy this account
      // did not install, and one being installed, are out of its reach.
      canRemove:
        model.installed &&
        model.location === 'user' &&
        !pending &&
        !(running && running.model === model.id),
      elsewhere,
      notAllowed: excludedByPolicy(status, model.id),
      pending,
    };
  });
}

/** Whole percent of an install, 0 to 100, as the composer's "Downloading speech model… <n>%". */
export function installPercent(job: InstallJob): number {
  if (job.bytes_total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.floor((job.bytes_done / job.bytes_total) * 100)));
}

/**
 * An install's bytes as its row counts them. The job also fetches what the model needs and this
 * folder lacks, first (`with_requirements` in `catalog.rs`: the 885,098 B voice detector), so its
 * total is larger than the size the row showed before the click: turbo's 574,041,195 B row read
 * "574 MB" and its download "of 575 MB" [M]. The megabytes count the model's own bytes instead,
 * so the total is the row's size; the percent stays the whole job's ({@link installPercent}).
 */
export function installBytes(job: InstallJob, modelBytes: number): { done: number; total: number } {
  const before = job.bytes_total - modelBytes;
  if (modelBytes <= 0 || before <= 0) return { done: job.bytes_done, total: job.bytes_total };
  return { done: Math.max(0, Math.min(modelBytes, job.bytes_done - before)), total: modelBytes };
}

/**
 * The model the daemon uses once `removed` is gone, by its own rule (`active_model` in
 * `crates/biorouter/src/dictation/service.rs`): the configured model when it is usable, which the
 * removed one no longer is, then this computer's default (the row marked `recommended`), then the
 * first usable model in catalog order. Usable here is installed and admitted by managed policy.
 */
export function modelAfterRemoving(status: DictationStatus, removed: string): SpeechModel | null {
  const usable = (m: SpeechModel) =>
    m.id !== removed && m.installed && !excludedByPolicy(status, m.id);
  return (
    status.models.find((m) => m.recommended && usable(m)) ?? status.models.find(usable) ?? null
  );
}

/**
 * The question asked before Remove. Removing the model in use makes the daemon use another one at
 * once, so that question names it (MSG-REMOVE-CONFIRM-ACTIVE), or says no installed model can take
 * its place (MSG-REMOVE-CONFIRM-LAST); any other model gets MSG-REMOVE-CONFIRM.
 */
export function removeQuestion(status: DictationStatus, model: SpeechModel): string {
  const values = { model: model.display_name, folder: status.models_dir };
  if (status.active_model !== model.id) return dictationMessage('MSG-REMOVE-CONFIRM', values);
  const next = modelAfterRemoving(status, model.id);
  return next
    ? dictationMessage('MSG-REMOVE-CONFIRM-ACTIVE', { ...values, next: next.display_name })
    : dictationMessage('MSG-REMOVE-CONFIRM-LAST', values);
}

/**
 * The question asked before an install the daemon rated slow or short of memory (DD-22, P-8: the
 * person is warned, then free). MSG-TURBO-CPU names the time Test speed measured for that model
 * from this window; without one, MSG-TOO-SLOW-SETTINGS names the rule instead, as the terminal's
 * MSG-TOO-SLOW does, because the time does not exist before the model is installed.
 */
export function installQuestion(
  model: SpeechModel,
  measured: Readonly<Record<string, BenchResult>> = snapshot.measured
): SpeechNotice | null {
  if (model.suitability === 'too_slow') {
    const bench = measured[model.id];
    if (bench && bench.rtf > 0) {
      return {
        id: 'MSG-TURBO-CPU',
        text: dictationMessage('MSG-TURBO-CPU', {
          t: (bench.elapsed_ms / 1000).toFixed(1),
          a: (bench.elapsed_ms / bench.rtf / 1000).toFixed(1),
        }),
        tone: 'warning',
      };
    }
    return {
      id: 'MSG-TOO-SLOW-SETTINGS',
      text: dictationMessage('MSG-TOO-SLOW-SETTINGS', { model: model.display_name }),
      tone: 'warning',
    };
  }
  if (model.suitability === 'below_minimum') {
    return {
      id: 'MSG-BELOW-MINIMUM-SETTINGS',
      text: dictationMessage('MSG-BELOW-MINIMUM-SETTINGS', { model: model.display_name }),
      tone: 'warning',
    };
  }
  return null;
}

/** Whether a model understands only English (a Whisper `.en` id). */
export const englishOnly = (id: string): boolean => /\.en(-|$)/.test(id);

/**
 * Every code the engine takes besides `auto`: `WHISPER_LANGUAGES` in
 * `crates/biorouter-dictation/src/protocol.rs`, in its order (`speechModelStore.test.ts` reads that
 * file and holds the two equal). The daemon refuses any other code with a 422, so the list only
 * decides what the menu offers.
 */
// prettier-ignore
export const DICTATION_LANGUAGES = [
  'en', 'zh', 'de', 'es', 'ru', 'ko', 'fr', 'ja', 'pt', 'tr', 'pl', 'ca', 'nl', 'ar', 'sv', 'it',
  'id', 'hi', 'fi', 'vi', 'he', 'uk', 'el', 'ms', 'cs', 'ro', 'da', 'hu', 'ta', 'no', 'th', 'ur',
  'hr', 'bg', 'lt', 'la', 'mi', 'ml', 'cy', 'sk', 'te', 'fa', 'lv', 'bn', 'sr', 'az', 'sl', 'kn',
  'et', 'mk', 'br', 'eu', 'is', 'hy', 'ne', 'mn', 'bs', 'kk', 'sq', 'sw', 'gl', 'mr', 'pa', 'si',
  'km', 'sn', 'yo', 'so', 'af', 'oc', 'ka', 'be', 'tg', 'sd', 'gu', 'am', 'yi', 'lo', 'uz', 'fo',
  'ht', 'ps', 'tk', 'nn', 'mt', 'sa', 'lb', 'my', 'bo', 'tl', 'mg', 'as', 'tt', 'haw', 'ln',
  'ha', 'ba', 'jw', 'su', 'yue',
] as const;

export type DictationLanguage = (typeof DICTATION_LANGUAGES)[number];

/**
 * Each code's English name, as CLDR words it (Node 24's full ICU, 2026-11-07). The menu reads this
 * table rather than `Intl.DisplayNames`, because Electron's renderer carries a reduced ICU that
 * names 97 of these codes and answers `bo` and `ba` with the code itself, so the menu read "ba"
 * between Azerbaijani and Bangla [M]. The type makes a code without a name fail to compile.
 */
// prettier-ignore
const LANGUAGE_NAMES: Readonly<Record<DictationLanguage, string>> = {
  en: 'English', zh: 'Chinese', de: 'German', es: 'Spanish', ru: 'Russian', ko: 'Korean',
  fr: 'French', ja: 'Japanese', pt: 'Portuguese', tr: 'Turkish', pl: 'Polish', ca: 'Catalan',
  nl: 'Dutch', ar: 'Arabic', sv: 'Swedish', it: 'Italian', id: 'Indonesian', hi: 'Hindi',
  fi: 'Finnish', vi: 'Vietnamese', he: 'Hebrew', uk: 'Ukrainian', el: 'Greek', ms: 'Malay',
  cs: 'Czech', ro: 'Romanian', da: 'Danish', hu: 'Hungarian', ta: 'Tamil', no: 'Norwegian',
  th: 'Thai', ur: 'Urdu', hr: 'Croatian', bg: 'Bulgarian', lt: 'Lithuanian', la: 'Latin',
  mi: 'Māori', ml: 'Malayalam', cy: 'Welsh', sk: 'Slovak', te: 'Telugu', fa: 'Persian',
  lv: 'Latvian', bn: 'Bangla', sr: 'Serbian', az: 'Azerbaijani', sl: 'Slovenian', kn: 'Kannada',
  et: 'Estonian', mk: 'Macedonian', br: 'Breton', eu: 'Basque', is: 'Icelandic', hy: 'Armenian',
  ne: 'Nepali', mn: 'Mongolian', bs: 'Bosnian', kk: 'Kazakh', sq: 'Albanian', sw: 'Swahili',
  gl: 'Galician', mr: 'Marathi', pa: 'Punjabi', si: 'Sinhala', km: 'Khmer', sn: 'Shona',
  yo: 'Yoruba', so: 'Somali', af: 'Afrikaans', oc: 'Occitan', ka: 'Georgian', be: 'Belarusian',
  tg: 'Tajik', sd: 'Sindhi', gu: 'Gujarati', am: 'Amharic', yi: 'Yiddish', lo: 'Lao',
  uz: 'Uzbek', fo: 'Faroese', ht: 'Haitian Creole', ps: 'Pashto', tk: 'Turkmen',
  nn: 'Norwegian Nynorsk', mt: 'Maltese', sa: 'Sanskrit', lb: 'Luxembourgish', my: 'Burmese',
  bo: 'Tibetan', tl: 'Filipino', mg: 'Malagasy', as: 'Assamese', tt: 'Tatar', haw: 'Hawaiian',
  ln: 'Lingala', ha: 'Hausa', ba: 'Bashkir', jw: 'Javanese', su: 'Sundanese', yue: 'Cantonese',
};

const isDictationLanguage = (code: string): code is DictationLanguage =>
  Object.prototype.hasOwnProperty.call(LANGUAGE_NAMES, code);

/** A language's name in English; another code by the platform's name for it, or as written. */
export function languageName(code: string): string {
  if (isDictationLanguage(code)) return LANGUAGE_NAMES[code];
  try {
    const name = new Intl.DisplayNames(['en'], { type: 'language' }).of(code);
    return name && name.toLowerCase() !== code.toLowerCase() ? name : code;
  } catch {
    return code;
  }
}

/**
 * The Language menu: Detect automatically and English first, then every other language by name.
 * A code the status reports that the list does not hold (a hand edited `config.yaml`) is kept, so
 * the menu never states a value nobody chose.
 *
 * With an English only model in use (`englishOnlyModel`), the daemon refuses every value but `en`,
 * `auto` included (`check_language` in `service.rs`, MSG-LANGUAGE-EN), so the menu offers English
 * alone, beside the current value when that is something else.
 */
export function languageOptions(
  autoLabel: string,
  current?: string,
  englishOnlyModel = false
): Array<{ value: string; label: string }> {
  const rest = DICTATION_LANGUAGES.filter((code) => code !== 'en')
    .map((code) => ({ value: code as string, label: languageName(code) }))
    .sort((a, b) => a.label.localeCompare(b.label, 'en'));
  const options = [
    { value: 'auto', label: autoLabel },
    { value: 'en', label: languageName('en') },
    ...rest,
  ];
  if (current && !options.some((option) => option.value === current)) {
    options.push({ value: current, label: current });
  }
  if (englishOnlyModel) {
    return options.filter((option) => option.value === 'en' || option.value === current);
  }
  return options;
}

// ── tests ───────────────────────────────────────────────────────────────────────────────────

/** For tests: start over, optionally with a fake API. */
export function resetSpeechModelsForTests(fake?: DictationModelsApi): void {
  snapshot = INITIAL;
  listeners.clear();
  api = fake ?? dictationModelsApi;
}
