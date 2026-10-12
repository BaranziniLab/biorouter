import type { LlamaCppModel } from '../../../api';
import { localModelsCopy } from './copy';

/**
 * What a local model's row says about it, in one place: Settings' Local models list and first
 * run's Llama Server panel read the same words (R-34, vision FR-R1-3 and FR-S-2: the first-run
 * panel printed seven lines of facts where the Settings row prints one). Whether the model fits
 * this machine's memory is `localModelMemory`'s, the one home for memory words.
 */

export const isInstalled = (model: LlamaCppModel) =>
  model.download_status === 'downloaded' || model.fallback_download_status === 'downloaded';

export const hasPartialDownload = (model: LlamaCppModel) =>
  model.download_status === 'partial' || model.fallback_download_status === 'partial';

/**
 * A model's ONE state word, read by the row's trailing slot, its facts line and the info dialog's
 * Status row alike (vision round 2, V4-R2-5: principle 4, one status per row). They used to ask
 * three different questions of the same two fields, so the one model whose Ollama copy was partial
 * and whose fallback GGUF was whole said "Installed" at rest, "Partial download" under its name
 * and "Fallback ready" in the dialog.
 *
 * - Downloaded in Ollama: the model's own file is whole and Ollama holds it. Said in place of
 *   Installed because Delete removes it from Ollama, under every tag that uses that file (#393),
 *   which can take a model the Ollama provider chats with; a person should know where the model
 *   lives before choosing Delete.
 * - Installed: either copy is whole otherwise. The model works, so that is what a person is told;
 *   which copy serves it (and the other's half-finished download) is the daemon's business, and the
 *   dialog's Ollama model, Fallback download and Model store rows say where things are.
 * - Partial download: nothing is whole yet and something has started. The Install button is the
 *   action.
 * - Needs download: nothing has started. The row says nothing about it; the button is the cue.
 */
export const stateWord = (model: LlamaCppModel) =>
  model.download_status === 'downloaded' && model.download_source === 'ollama'
    ? localModelsCopy.status.inOllama
    : isInstalled(model)
      ? localModelsCopy.installed
      : hasPartialDownload(model)
        ? localModelsCopy.status.partial
        : localModelsCopy.status.needsDownload;

/** "Fast", "Fastest" or "Moderate": the head of the catalog's speed hint, before its detail. */
export const speedWord = (model: LlamaCppModel) => model.speed_hint.split(/[,;—]/)[0].trim();

/**
 * The catalog's speed hint without its memory clause, for an Expected speed row: "Moderate, dense
 * 27B" from "Moderate, dense 27B; needs 32 GiB of GPU memory, 48 GiB recommended" (#393). The
 * memory figures are the Memory rows' to say, in GB (vision FR-R4-5), and saying them twice, in two
 * units, a line apart, was the defect.
 */
export const speedDetail = (model: LlamaCppModel) =>
  model.speed_hint
    .split(';')
    .map((clause) => clause.trim())
    .filter((clause, index) => index === 0 || !/\bGi?B\b/.test(clause))
    .join('; ');

/**
 * The one facts line under a model's name (`Fast · 9.6 GB · 128k context`). The speed word stays
 * because the per-row speed hint was asked for (#35); the rest of the hint, the family, the
 * model id and the fit sentence are in the info dialog.
 *
 * `size: false` leaves the download size out, for a surface that already says it: first run's
 * Llama Server panel names it in the model's option and in its button ("Download and start (7.6
 * GB)"), so a third copy in the Details summary was one too many (vision FR-R2-6, principle 2).
 */
export const factsLine = (model: LlamaCppModel, { size = true }: { size?: boolean } = {}) =>
  [
    speedWord(model),
    size ? model.download_size : null,
    localModelsCopy.contextWindow(model.context_limit),
    // The state word, only when it is the one thing a not-yet-installed row has to say.
    !isInstalled(model) && hasPartialDownload(model) ? stateWord(model) : null,
  ]
    .filter(Boolean)
    .join(' · ');
