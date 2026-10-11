/**
 * How Biorouter describes a machine's memory budget for a Llama Server model.
 *
 * One copy. These helpers used to be pasted into `LocalModelInventory`,
 * `onboarding/LlamaServerInlineCard` and `ModelAndProviderContext`, and all
 * three went stale together when the daemon learned that the bundled
 * llama-server has no GPU backend on an Intel Mac or the Linux CPU build
 * (`accelerator_memory_kind: 'cpu_only'`, budgeted by system RAM): every one of
 * them still told an Intel Mac that its figure meant VRAM.
 *
 * The verdict itself is the daemon's (`suitability_status`, from
 * `routes/llamacpp.rs`). These functions only put it into words; they never
 * re-derive it from the raw numbers, which is how a 64 GiB Intel Mac used to
 * read as "meets the recommendation" here while the daemon called it CPU-only.
 */
import type { LlamaCppModel, LlamaCppSystemInfo } from '../../../api';

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

/** "64 GiB unified memory", or "unknown VRAM" when nothing was measured. */
export const detectedMemoryText = (system: LlamaCppSystemInfo) =>
  typeof system.accelerator_memory_gib === 'number'
    ? `${system.accelerator_memory_gib} GiB ${acceleratorMemoryLabel(system.accelerator_memory_kind)}`
    : `unknown ${acceleratorMemoryLabel(system.accelerator_memory_kind)}`;

/**
 * The warning to show before starting, installing or switching to `model`, or
 * `null` when the daemon rated it suitable for this machine.
 */
export function llamaMemoryWarning(
  model: LlamaCppModel,
  system: LlamaCppSystemInfo
): string | null {
  const kind = system.accelerator_memory_kind;
  const detected = detectedMemoryText(system);
  const explanation = acceleratorMemoryExplanation(kind);
  switch (model.suitability_status) {
    case 'suitable':
      return null;
    case 'cpu_only':
      // Memory is not the problem here, so no "recommends N GiB": a 64 GiB
      // Intel Mac clears every figure and is still a CPU.
      return `${model.display_name} would run without GPU acceleration on this machine (${detected}). ${explanation} Expect several minutes before the first reply of each chat.`;
    case 'below_minimum':
      return `${model.display_name} needs at least ${model.min_gpu_memory_gib} GiB of GPU-addressable memory (${model.recommended_gpu_memory_gib} GiB recommended). This machine reports ${detected}, so it may fail to load or run very slowly. ${explanation}`;
    case 'above_recommendation':
      return `${model.display_name} recommends ${model.recommended_gpu_memory_gib} GiB GPU-addressable memory. This machine reports ${detected}. ${explanation}`;
    case 'unknown_resources':
    default:
      return `Biorouter could not detect this machine's GPU memory. ${model.display_name} needs at least ${model.min_gpu_memory_gib} GiB of GPU-addressable memory (${model.recommended_gpu_memory_gib} GiB recommended). ${explanation}`;
  }
}

/**
 * The Windows reminder that system RAM does not count. Not shown on a CPU-only
 * machine, where system RAM is exactly what counts.
 */
export const windowsVramNote = (system: LlamaCppSystemInfo) =>
  system.os.toLowerCase().includes('windows') && system.accelerator_memory_kind !== 'cpu_only'
    ? 'On Windows, make sure free VRAM is high enough for the model and context window; regular system RAM does not satisfy the GPU memory recommendation.'
    : null;

/**
 * The fit chip beside a model in Settings → Local Model Inventory.
 *
 * `below_minimum` and `cpu_only` used to fall through to "Unknown" — on exactly
 * the machines that most need telling: a 16 GiB laptop, which is still offered
 * the default because there is no smaller model, and an Intel Mac whose RAM
 * clears every figure while the model runs on its processor.
 */
export const inventoryFitLabel = (model: LlamaCppModel) => {
  switch (model.suitability_status) {
    case 'suitable':
      return 'Recommended';
    case 'above_recommendation':
      return `${model.recommended_gpu_memory_gib} GiB GPU memory`;
    case 'below_minimum':
      return `Below ${model.min_gpu_memory_gib} GiB minimum`;
    case 'cpu_only':
      return 'CPU only (slow)';
    case 'unknown_resources':
      return 'VRAM unknown';
    default:
      return 'Unknown';
  }
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
      return `Needs ${model.recommended_gpu_memory_gib} GiB GPU memory`;
    case 'below_minimum':
      return `Below ${model.min_gpu_memory_gib} GiB minimum`;
    case 'cpu_only':
      return 'CPU only (slow)';
    case 'unknown_resources':
      return 'VRAM unknown';
    default:
      return null;
  }
};
