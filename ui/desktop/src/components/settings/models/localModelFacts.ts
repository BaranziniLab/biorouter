import type { LlamaCppModel } from '../../../api';
import { localModelsCopy } from './copy';

/**
 * What a local model's row says about it, in one place: Settings' Local models list and first
 * run's Llama Server panel read the same words (R-34, vision FR-R1-3 and FR-S-2: the first-run
 * panel printed seven lines of facts where the Settings row prints one).
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
 * - Installed: either copy is whole. The model works, so that is what a person is told; which copy
 *   serves it (and the other's half-finished download) is the daemon's business, and the dialog's
 *   Ollama model, Fallback GGUF and Model store rows say where things are.
 * - Partial download: nothing is whole yet and something has started. The Install button is the
 *   action.
 * - Needs download: nothing has started. The row says nothing about it; the button is the cue.
 */
export const stateWord = (model: LlamaCppModel) =>
  isInstalled(model)
    ? localModelsCopy.installed
    : hasPartialDownload(model)
      ? localModelsCopy.status.partial
      : localModelsCopy.status.needsDownload;

/** "Fast", "Fastest" or "Moderate": the head of the catalog's speed hint, before its detail. */
export const speedWord = (model: LlamaCppModel) => model.speed_hint.split(/[,;—]/)[0].trim();

/**
 * The one facts line under a model's name (`Fast · 9.6 GB · 128k context`). The speed word stays
 * because the per-row speed hint was asked for (#35); the rest of the hint, the family, the
 * model id and the fit sentence are in the info dialog.
 */
export const factsLine = (model: LlamaCppModel) =>
  [
    speedWord(model),
    model.download_size,
    localModelsCopy.contextWindow(model.context_limit),
    // The state word, only when it is the one thing a not-yet-installed row has to say.
    !isInstalled(model) && hasPartialDownload(model) ? stateWord(model) : null,
  ]
    .filter(Boolean)
    .join(' · ');
