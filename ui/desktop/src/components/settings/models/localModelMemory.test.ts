import { describe, it, expect } from 'vitest';
import type { LlamaCppModel, LlamaCppSuitability, LlamaCppSystemInfo } from '../../../api';
import {
  acceleratorMemoryExplanation,
  acceleratorMemoryLabel,
  inventoryFitLabel,
  llamaMemoryWarning,
  pickerFitLabel,
  windowsVramNote,
} from './localModelMemory';

const model = (suitability_status: LlamaCppSuitability): LlamaCppModel => ({
  name: 'qwen3.8-27b',
  display_name: 'Qwen3.8 27B',
  family: 'Qwen3.8',
  ollama_name: 'qwen3.8:27b',
  official_url: 'https://ollama.com/library/qwen3.8:27b',
  hf_spec: 'ggml-org/Qwen3.8-27B-GGUF:Q4_K_M',
  download_size: '19 GB',
  description: 'Default local model',
  min_gpu_memory_gib: 32,
  recommended_gpu_memory_gib: 48,
  context_limit: 262144,
  active_params_b: 27,
  speed_hint: 'Moderate',
  is_default: true,
  downloaded: false,
  download_status: 'not_downloaded',
  download_source: 'none',
  fallback_downloaded: false,
  fallback_download_status: 'not_downloaded',
  model_path: null,
  suitable: suitability_status === 'suitable',
  suitability_status,
  suitability_message: '',
});

const system = (overrides: Partial<LlamaCppSystemInfo> = {}): LlamaCppSystemInfo => ({
  os: 'macos',
  total_memory_gib: 64,
  accelerator_memory_gib: 64,
  accelerator_memory_kind: 'apple_unified',
  default_context_size: 131072,
  model_cache_dir: '/tmp/models',
  model_cache_layout: 'test',
  ...overrides,
});

const intelMac = system({ accelerator_memory_gib: 64, accelerator_memory_kind: 'cpu_only' });

describe('localModelMemory', () => {
  /**
   * An Intel Mac reports `cpu_only` and its system RAM. Every copy of these
   * helpers called that figure VRAM, and the explanation named Intel Macs as
   * a discrete-GPU machine.
   */
  it('calls a CPU-only machine’s memory system RAM, and says why', () => {
    expect(acceleratorMemoryLabel('cpu_only')).toBe('system RAM');
    expect(acceleratorMemoryLabel('apple_unified')).toBe('unified memory');
    expect(acceleratorMemoryLabel('vram')).toBe('VRAM');
    expect(acceleratorMemoryLabel(undefined)).toBe('VRAM');

    expect(acceleratorMemoryExplanation('cpu_only')).toMatch(/no GPU acceleration/);
    expect(acceleratorMemoryExplanation('vram')).not.toMatch(/Intel Mac/);
  });

  it('says nothing for a model the daemon rated suitable', () => {
    expect(llamaMemoryWarning(model('suitable'), system())).toBeNull();
  });

  it('warns a CPU-only machine whatever its memory, without a memory figure to beat', () => {
    const warning = llamaMemoryWarning(model('cpu_only'), intelMac);
    expect(warning).toMatch(/without GPU acceleration on this machine \(64 GiB system RAM\)/);
    expect(warning).not.toMatch(/recommends/);
  });

  it('names the minimum to a machine below it', () => {
    const warning = llamaMemoryWarning(
      model('below_minimum'),
      system({ accelerator_memory_gib: 16 })
    );
    expect(warning).toMatch(/needs at least 32 GiB/);
    expect(warning).toMatch(/16 GiB unified memory/);
    expect(warning).toMatch(/may fail to load or run very slowly/);
    expect(warning).not.toMatch(/laptop|Gemma/);
  });

  it('keeps the recommendation wording above the minimum', () => {
    expect(
      llamaMemoryWarning(
        model('above_recommendation'),
        system({ os: 'windows', accelerator_memory_gib: 40, accelerator_memory_kind: 'vram' })
      )
    ).toMatch(/recommends 48 GiB GPU-addressable memory\. This machine reports 40 GiB VRAM\./);
  });

  it('says when nothing was measured', () => {
    expect(
      llamaMemoryWarning(
        model('unknown_resources'),
        system({
          os: 'linux',
          accelerator_memory_gib: null,
          accelerator_memory_kind: 'unknown_vram',
        })
      )
    ).toMatch(/could not detect/);
  });

  it('reminds Windows that system RAM is not VRAM, except where it is', () => {
    expect(windowsVramNote(system({ os: 'windows', accelerator_memory_kind: 'vram' }))).toMatch(
      /system RAM does not satisfy/
    );
    expect(windowsVramNote(system({ os: 'windows', accelerator_memory_kind: 'cpu_only' }))).toBe(
      null
    );
    expect(windowsVramNote(system())).toBeNull();
  });

  it('labels every suitability status, in both places', () => {
    expect(inventoryFitLabel(model('below_minimum'))).toBe('Below 32 GiB minimum');
    expect(inventoryFitLabel(model('cpu_only'))).toBe('CPU only (slow)');
    expect(inventoryFitLabel(model('suitable'))).toBe('Recommended');
    expect(inventoryFitLabel(model('above_recommendation'))).toBe('48 GiB GPU memory');
    expect(inventoryFitLabel(model('unknown_resources'))).toBe('VRAM unknown');

    expect(pickerFitLabel(model('below_minimum'))).toBe('Below 32 GiB minimum');
    expect(pickerFitLabel(model('cpu_only'))).toBe('CPU only (slow)');
    expect(pickerFitLabel(model('suitable'))).toBe('Recommended here');
    expect(pickerFitLabel(model('above_recommendation'))).toBe('Needs 48 GiB GPU memory');
    expect(pickerFitLabel(undefined)).toBeNull();
  });
});
