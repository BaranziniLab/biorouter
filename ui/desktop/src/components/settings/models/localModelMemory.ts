/**
 * How Biorouter describes a machine's memory budget for a Llama Server model: the one home for
 * every memory word the renderer writes (the Local models rows and info dialog, first run's Llama
 * Server panel, the model picker and the warm-up dialog).
 *
 * One copy. These helpers used to be pasted into `LocalModelInventory`,
 * `onboarding/LlamaServerInlineCard` and `ModelAndProviderContext`, and all
 * three went stale together when the daemon learned that the bundled
 * llama-server has no GPU backend on an Intel Mac or the Linux CPU build
 * (`accelerator_memory_kind: 'cpu_only'`, budgeted by system RAM): every one of
 * them still told an Intel Mac that its figure meant VRAM. The fit words
 * (`fitVerdict`, `fitLine`) lived in a second module that knew only three of the
 * daemon's five verdicts, so they are here too.
 *
 * The verdict itself is the daemon's (`suitability_status`, from
 * `routes/llamacpp.rs`). These functions only put it into words; they never
 * re-derive it from the raw numbers, which is how a 64 GiB Intel Mac used to
 * read as "meets the recommendation" here while the daemon called it CPU-only.
 *
 * Every amount is written in GB through `localModelsCopy.gigabytes`, the unit the download sizes
 * use (vision FR-R4-5). The daemon counts memory in GiB; a Mac's own About window calls the same
 * figure GB.
 */
import type { LlamaCppModel, LlamaCppSystemInfo } from '../../../api';
import { localModelsCopy } from './copy';

type MachineMemory = Pick<
  LlamaCppSystemInfo,
  'os' | 'accelerator_memory_gib' | 'accelerator_memory_kind'
>;

const { gigabytes, memoryAmount } = localModelsCopy;

/** What `accelerator_memory_gib` measures on this machine, as a noun. */
export const acceleratorMemoryLabel = (kind: string | undefined) => {
  switch (kind) {
    case 'apple_unified':
      return 'unified memory';
    case 'cpu_only':
      return 'system RAM';
    default:
      return 'VRAM';
  }
};

export const acceleratorMemoryExplanation = (kind: string | undefined) => {
  switch (kind) {
    case 'apple_unified':
      return 'On Apple Silicon, unified memory is the relevant GPU memory budget.';
    case 'cpu_only':
      // An Intel Mac lands here: upstream builds the macOS x64 llama-server
      // without Metal, so its discrete GPU is never used.
      return 'Llama Server has no GPU acceleration on this machine (an Intel Mac, or the Linux CPU build), so models run on the processor out of system RAM, at a few tokens per second.';
    default:
      return 'On Windows, Linux and other discrete-GPU systems, this means VRAM, not regular system RAM.';
  }
};

/** "64 GB unified memory", or "unknown VRAM" when nothing was measured. */
export const detectedMemoryText = (system: MachineMemory) =>
  typeof system.accelerator_memory_gib === 'number'
    ? memoryAmount(
        system.accelerator_memory_gib,
        acceleratorMemoryLabel(system.accelerator_memory_kind)
      )
    : `unknown ${acceleratorMemoryLabel(system.accelerator_memory_kind)}`;

/**
 * The warning to show before starting, installing or switching to `model`, or
 * `null` when the daemon rated it suitable for this machine.
 */
export function llamaMemoryWarning(model: LlamaCppModel, system: MachineMemory): string | null {
  const kind = system.accelerator_memory_kind;
  const detected = detectedMemoryText(system);
  const explanation = acceleratorMemoryExplanation(kind);
  const minimum = gigabytes(model.min_gpu_memory_gib);
  const recommended = gigabytes(model.recommended_gpu_memory_gib);
  switch (model.suitability_status) {
    case 'suitable':
      return null;
    case 'cpu_only':
      // Memory is not the problem here, so no "recommends N GB": a 64 GB
      // Intel Mac clears every figure and is still a CPU.
      return `${model.display_name} would run without GPU acceleration on this machine (${detected}). ${explanation} Expect several minutes before the first reply of each chat.`;
    case 'below_minimum':
      return `${model.display_name} needs at least ${minimum} of GPU-addressable memory (${recommended} recommended). This machine reports ${detected}, so it may fail to load or run very slowly. ${explanation}`;
    case 'above_recommendation':
      return `${model.display_name} recommends ${localModelsCopy.gpuMemory(model.recommended_gpu_memory_gib)}. This machine reports ${detected}. ${explanation}`;
    case 'unknown_resources':
    default:
      return `Biorouter could not detect this machine's GPU memory. ${model.display_name} needs at least ${minimum} of GPU-addressable memory (${recommended} recommended). ${explanation}`;
  }
}

/**
 * The Windows reminder that system RAM does not count. Not shown on a CPU-only
 * machine, where system RAM is exactly what counts.
 */
export const windowsVramNote = (system: MachineMemory) =>
  system.os.toLowerCase().includes('windows') && system.accelerator_memory_kind !== 'cpu_only'
    ? 'On Windows, make sure free VRAM is high enough for the model and context window; regular system RAM does not satisfy the GPU memory recommendation.'
    : null;

const isMac = (system: MachineMemory | null | undefined) => system?.os === 'macos';

/**
 * Whether the model fits this machine, as a verdict: "Fits this Mac", "Large for this Mac",
 * "Below the 32 GB minimum", "CPU only, slow", or "VRAM not detected" when the daemon could not
 * read the memory. From the daemon's own `suitability_status`, so the renderer never re-decides
 * the fit; only the words are its own.
 *
 * `below_minimum` and `cpu_only` used to fall through to "VRAM not detected" here (and to
 * "Unknown" in the older inventory chip), on exactly the machines that most need telling: a 16 GB
 * laptop, which is still offered the default because there is no smaller model, and an Intel Mac
 * whose RAM clears every figure while the model runs on its processor.
 */
export const fitVerdict = (model: LlamaCppModel, system: MachineMemory | null | undefined) => {
  switch (model.suitability_status) {
    case 'suitable':
      return isMac(system) ? localModelsCopy.fitsThisMac : localModelsCopy.fitsThisComputer;
    case 'above_recommendation':
      return isMac(system) ? localModelsCopy.largeForThisMac : localModelsCopy.largeForThisComputer;
    case 'below_minimum':
      return localModelsCopy.belowMinimum(model.min_gpu_memory_gib);
    case 'cpu_only':
      return localModelsCopy.cpuOnly;
    default:
      return localModelsCopy.memoryNotDetected(
        acceleratorMemoryLabel(system?.accelerator_memory_kind)
      );
  }
};

/**
 * The verdict a Local models row adds to its facts line, in warning ink, or `null` when the row
 * has nothing to warn about. Only the three verdicts that qualify the Install beside them: a model
 * that fits says nothing, and a machine whose memory was not read is not told it is too small.
 */
export const fitWarning = (model: LlamaCppModel, system: MachineMemory | null | undefined) => {
  switch (model.suitability_status) {
    case 'above_recommendation':
    case 'below_minimum':
    case 'cpu_only':
      return fitVerdict(model, system);
    default:
      return null;
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
      ? memoryAmount(gib, acceleratorMemoryLabel(system?.accelerator_memory_kind))
      : null;
  const facts = [reported, localModelsCopy.recommendedMemory(model.recommended_gpu_memory_gib)]
    .filter(Boolean)
    .join(', ');
  return `${fitVerdict(model, system)} (${facts})`;
};

/**
 * The same verdict in a model picker's row detail, or `null` to say nothing
 * (a model the catalog does not know).
 */
export const pickerFitLabel = (model: LlamaCppModel | undefined) => {
  switch (model?.suitability_status) {
    case 'suitable':
      return 'Recommended here';
    case 'above_recommendation':
      return `Needs ${gigabytes(model.recommended_gpu_memory_gib)} GPU memory`;
    case 'below_minimum':
      return localModelsCopy.belowMinimum(model.min_gpu_memory_gib);
    case 'cpu_only':
      return localModelsCopy.cpuOnly;
    case 'unknown_resources':
      return 'VRAM unknown';
    default:
      return null;
  }
};
