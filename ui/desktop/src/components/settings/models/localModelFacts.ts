import type { LlamaCppModel, LlamaCppSystemInfo } from '../../../api';
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
 *   Ollama model, Fallback download and Model store rows say where things are.
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

type MachineMemory = Pick<
  LlamaCppSystemInfo,
  'os' | 'accelerator_memory_gib' | 'accelerator_memory_kind'
>;

/** What the machine's GPU-addressable memory is called: unified memory on Apple Silicon, else VRAM. */
export const memoryKindWord = (kind: string | undefined) =>
  kind === 'apple_unified' ? 'unified memory' : 'VRAM';

const isMac = (system: MachineMemory | null | undefined) => system?.os === 'macos';

/**
 * Whether the model fits this machine, as a verdict: "Fits this Mac", "Large for this Mac", or
 * "VRAM not detected" when the daemon could not read the memory. From the daemon's own
 * `suitability_status`, so the renderer never re-decides the fit; only the words are its own.
 */
export const fitVerdict = (model: LlamaCppModel, system: MachineMemory | null | undefined) => {
  switch (model.suitability_status) {
    case 'suitable':
      return isMac(system) ? localModelsCopy.fitsThisMac : localModelsCopy.fitsThisComputer;
    case 'above_recommendation':
      return isMac(system) ? localModelsCopy.largeForThisMac : localModelsCopy.largeForThisComputer;
    default:
      return localModelsCopy.memoryNotDetected(memoryKindWord(system?.accelerator_memory_kind));
  }
};

/**
 * The verdict with the two numbers it rests on, in plain words: "Fits this Mac (128 GB unified
 * memory, 32 GB recommended)" (vision FR-R2-6; in GB, the download sizes' unit, since FR-R4-5).
 * It replaces the daemon's sentence on a reading surface, which said "Recommended for this
 * machine: 128 GiB apple_unified meets the 32 GiB recommendation." under the label
 * "Recommendation": the label twice and a machine id once.
 */
export const fitLine = (model: LlamaCppModel, system: MachineMemory | null | undefined) => {
  const gib = system?.accelerator_memory_gib;
  const reported =
    typeof gib === 'number'
      ? localModelsCopy.memoryAmount(gib, memoryKindWord(system?.accelerator_memory_kind))
      : null;
  const facts = [reported, localModelsCopy.recommendedMemory(model.recommended_gpu_memory_gib)]
    .filter(Boolean)
    .join(', ');
  return `${fitVerdict(model, system)} (${facts})`;
};
