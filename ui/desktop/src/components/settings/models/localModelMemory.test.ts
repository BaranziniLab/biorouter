import { describe, it, expect } from 'vitest';
import type { LlamaCppModel, LlamaCppSuitability, LlamaCppSystemInfo } from '../../../api';
import {
  acceleratorMemoryExplanation,
  acceleratorMemoryLabel,
  fitLine,
  fitVerdict,
  fitWarning,
  llamaMemoryWarning,
  ollamaFit,
  pickerFitLabel,
  windowsVramNote,
} from './localModelMemory';
import { speedDetail } from './localModelFacts';
import { localModelsCopy } from './copy';

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
    expect(warning).toMatch(/without GPU acceleration on this machine \(64 GB system RAM\)/);
    expect(warning).not.toMatch(/recommends/);
  });

  it('names the minimum to a machine below it', () => {
    const warning = llamaMemoryWarning(
      model('below_minimum'),
      system({ accelerator_memory_gib: 16 })
    );
    expect(warning).toMatch(/needs at least 32 GB of GPU-addressable memory \(48 GB recommended\)/);
    expect(warning).toMatch(/16 GB unified memory/);
    expect(warning).toMatch(/may fail to load or run very slowly/);
    expect(warning).not.toMatch(/laptop|Gemma/);
  });

  it('keeps the recommendation wording above the minimum', () => {
    expect(
      llamaMemoryWarning(
        model('above_recommendation'),
        system({ os: 'windows', accelerator_memory_gib: 40, accelerator_memory_kind: 'vram' })
      )
    ).toMatch(/recommends 48 GB GPU-addressable memory\. This machine reports 40 GB VRAM\./);
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

  /**
   * FR-R4-5: every amount the renderer writes is GB, the unit of the download sizes. These
   * helpers wrote GiB while the rows beside them wrote GB.
   */
  it('writes every memory amount in GB, never GiB', () => {
    const statuses: LlamaCppSuitability[] = [
      'suitable',
      'above_recommendation',
      'below_minimum',
      'cpu_only',
      'unknown_resources',
    ];
    for (const status of statuses) {
      for (const machine of [system(), intelMac, system({ accelerator_memory_gib: null })]) {
        const words = [
          llamaMemoryWarning(model(status), machine),
          fitVerdict(model(status), machine),
          fitLine(model(status), machine),
          pickerFitLabel(model(status), machine),
        ].join(' ');
        expect(words, status).not.toMatch(/GiB/);
      }
    }
  });

  /**
   * The fit words cover all five of the daemon's verdicts. `below_minimum` and `cpu_only` fell
   * through to "VRAM not detected", which told a 16 GB laptop and an Intel Mac that nothing was
   * measured.
   */
  it('says the fit in words for every suitability status', () => {
    const mac = system();
    const pc = system({ os: 'linux', accelerator_memory_kind: 'vram' });
    expect(fitVerdict(model('suitable'), mac)).toBe('Fits this Mac');
    expect(fitVerdict(model('suitable'), pc)).toBe('Fits this computer');
    expect(fitVerdict(model('above_recommendation'), mac)).toBe('Large for this Mac');
    expect(fitVerdict(model('above_recommendation'), pc)).toBe('Large for this computer');
    expect(fitVerdict(model('below_minimum'), mac)).toBe('Below the 32 GB minimum');
    expect(fitVerdict(model('cpu_only'), intelMac)).toBe('CPU only, slow');
    expect(fitVerdict(model('unknown_resources'), pc)).toBe('VRAM not detected');
    expect(fitVerdict(model('unknown_resources'), mac)).toBe('Unified memory not detected');

    expect(fitLine(model('suitable'), mac)).toBe(
      'Fits this Mac (64 GB unified memory, 48 GB recommended)'
    );
    expect(fitLine(model('below_minimum'), system({ accelerator_memory_gib: 16 }))).toBe(
      'Below the 32 GB minimum (16 GB unified memory, 48 GB recommended)'
    );
    expect(fitLine(model('cpu_only'), intelMac)).toBe(
      'CPU only, slow (64 GB system RAM, 48 GB recommended)'
    );
    expect(fitLine(model('unknown_resources'), system({ accelerator_memory_gib: null }))).toBe(
      'Unified memory not detected (48 GB recommended)'
    );
  });

  it('warns on a row only for a verdict that qualifies its Install', () => {
    expect(fitWarning(model('suitable'), system())).toBeNull();
    expect(fitWarning(model('unknown_resources'), system())).toBeNull();
    expect(fitWarning(model('above_recommendation'), system())).toBe('Large for this Mac');
    expect(fitWarning(model('below_minimum'), system())).toBe('Below the 32 GB minimum');
    expect(fitWarning(model('cpu_only'), intelMac)).toBe('CPU only, slow');
  });

  /**
   * One vocabulary: the picker said "Recommended here", "Needs 48 GB GPU memory" and "VRAM unknown"
   * for the verdicts Settings and first run call "Fits this Mac", "Large for this Mac" and "VRAM not
   * detected".
   */
  it('labels every suitability status in a picker row with the Settings row words', () => {
    const mac = system();
    const pc = system({
      os: 'linux',
      accelerator_memory_gib: null,
      accelerator_memory_kind: 'vram',
    });
    const statuses: LlamaCppSuitability[] = [
      'suitable',
      'above_recommendation',
      'below_minimum',
      'cpu_only',
      'unknown_resources',
    ];
    for (const status of statuses) {
      for (const machine of [mac, pc, intelMac]) {
        expect(pickerFitLabel(model(status), machine), status).toBe(
          fitVerdict(model(status), machine)
        );
      }
    }
    expect(pickerFitLabel(model('suitable'), mac)).toBe('Fits this Mac');
    expect(pickerFitLabel(model('above_recommendation'), mac)).toBe('Large for this Mac');
    expect(pickerFitLabel(model('unknown_resources'), pc)).toBe('VRAM not detected');
    expect(pickerFitLabel(undefined, mac)).toBeNull();
    for (const retired of ['Recommended here', 'GPU memory', 'VRAM unknown']) {
      for (const status of statuses) {
        expect(pickerFitLabel(model(status), mac) ?? '').not.toContain(retired);
      }
    }
  });

  /**
   * Ollama's verdict is the machine's, one for both recommended models. It was the daemon's
   * paragraph, in GiB, in a warning box; now it is the Llama Server rows' word and an InfoTip's
   * reason, in GB (FR-R4-5).
   */
  it('words Ollama’s verdict as the rows do, with its reason in GB', () => {
    const recs = (
      suitability: 'suitable' | 'below_minimum' | 'cpu_only' | 'unknown_resources',
      over: Partial<{ memory_gib: number | null; memory_kind: string }> = {}
    ) => ({
      suitability,
      memory_gib: 16,
      memory_kind: 'apple_unified',
      min_memory_gib: 32,
      ...over,
    });
    expect(ollamaFit(recs('suitable'))).toBeNull();

    const below = ollamaFit(recs('below_minimum'));
    expect(below).toEqual({
      word: localModelsCopy.belowMinimum(32),
      warning: true,
      help: localModelsCopy.ollamaFitHelp.belowMinimum('16 GB unified memory'),
    });

    const cpu = ollamaFit(recs('cpu_only', { memory_gib: 64, memory_kind: 'cpu_only' }));
    expect(cpu?.word).toBe(localModelsCopy.cpuOnly);
    expect(cpu?.warning).toBe(true);

    // A Linux CPU build reports `cpu_only` with an unknown verdict: what was not found is a GPU.
    const linux = ollamaFit(
      recs('unknown_resources', { memory_gib: null, memory_kind: 'cpu_only' })
    );
    expect(linux?.word).toBe('VRAM not detected');
    expect(linux?.warning).toBe(false);
    expect(ollamaFit(recs('unknown_resources', { memory_gib: null }))?.word).toBe(
      'Unified memory not detected'
    );

    for (const verdict of [below, cpu, linux]) {
      expect(verdict?.help).not.toMatch(/GiB/);
      // InfoTip text: at most two sentences, about 160 characters.
      expect(verdict?.help.length).toBeLessThanOrEqual(170);
      expect(verdict?.help.split(/\.\s/).length).toBeLessThanOrEqual(2);
    }
  });

  it('keeps the speed and drops the memory clause from an Expected speed row', () => {
    expect(speedDetail(model('suitable'))).toBe('Moderate');
    expect(
      speedDetail({
        ...model('suitable'),
        speed_hint: 'Moderate, dense 27B; needs 32 GiB of GPU memory, 48 GiB recommended',
      })
    ).toBe('Moderate, dense 27B');
    expect(speedDetail({ ...model('suitable'), speed_hint: 'Fast, ~4B active parameters' })).toBe(
      'Fast, ~4B active parameters'
    );
  });
});
