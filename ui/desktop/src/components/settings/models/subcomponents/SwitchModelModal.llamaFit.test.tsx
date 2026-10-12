import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type * as Api from '../../../../api';
import type { LlamaCppModel, LlamaCppSuitability, ProviderDetails } from '../../../../api';
import { localModelsCopy } from '../copy';
import { SwitchModelModal } from './SwitchModelModal';

/**
 * The Llama Server rows of the Switch models picker say a model's fit in the words Settings' Local
 * models rows and first run say it (#393 merged into the redesign): "Fits this Mac", "Large for
 * this Mac", "Below the 32 GB minimum", "CPU only, slow". The picker had its own three words for
 * the same verdicts ("Recommended here", "Needs 48 GB GPU memory", "VRAM unknown").
 *
 * The real react-select runs here, as in `SwitchModelModal.privacy.test.tsx`: the option rows are
 * what is asserted.
 */

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getProviderModels: vi.fn(),
  read: vi.fn(),
  llamacppStatus: vi.fn(),
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    getProviderModels: mocks.getProviderModels,
    read: mocks.read,
  }),
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    changeModel: vi.fn(),
    currentModel: null,
    currentProvider: 'llamacpp',
  }),
}));

vi.mock('../predefinedModelsUtils', () => ({
  getPredefinedModelsFromEnv: () => [],
  shouldShowPredefinedModels: () => false,
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof Api>()),
  llamacppStatus: mocks.llamacppStatus,
}));

const llamaProvider = {
  name: 'llamacpp',
  is_configured: true,
  provider_type: 'Builtin',
  affiliation: null,
  metadata: {
    config_keys: [],
    default_model: 'qwen3.8-27b',
    description: '',
    display_name: 'Llama Server',
    known_models: [{ name: 'qwen3.8-27b' }, { name: 'gemma4-31b' }],
    model_doc_link: '',
    name: 'llamacpp',
    tier: 'private',
    runs_locally: true,
  },
} as unknown as ProviderDetails;

const entry = (
  name: string,
  display: string,
  suitability_status: LlamaCppSuitability
): LlamaCppModel =>
  ({
    name,
    display_name: display,
    family: display,
    ollama_name: null,
    official_url: '',
    hf_spec: 'x',
    download_size: '19 GB',
    description: '',
    min_gpu_memory_gib: 32,
    recommended_gpu_memory_gib: 48,
    context_limit: 262144,
    active_params_b: 27,
    speed_hint: 'Moderate',
    is_default: name === 'qwen3.8-27b',
    downloaded: false,
    download_status: 'not_downloaded',
    download_source: 'none',
    fallback_downloaded: false,
    fallback_download_status: 'not_downloaded',
    model_path: null,
    suitable: suitability_status === 'suitable',
    suitability_status,
    suitability_message: 'the daemon sentence, never shown',
  }) as LlamaCppModel;

const status = (
  statuses: [LlamaCppSuitability, LlamaCppSuitability],
  system: Partial<Api.LlamaCppSystemInfo> = {}
) => ({
  data: {
    sidecar: { state: 'stopped', warmed: false, build: 'test', detail: null, model: null },
    catalog: [
      entry('qwen3.8-27b', 'Qwen3.8 27B', statuses[0]),
      entry('gemma4-31b', 'Gemma 4 31B', statuses[1]),
    ],
    system: {
      os: 'macos',
      total_memory_gib: 64,
      accelerator_memory_gib: 64,
      accelerator_memory_kind: 'apple_unified',
      default_context_size: 65536,
      model_cache_dir: '/tmp',
      model_cache_layout: 'ollama',
      ...system,
    },
  },
});

async function optionTexts() {
  await screen.findByText('Qwen3.8 27B');
  const combos = screen.getAllByRole('combobox');
  fireEvent.keyDown(combos[combos.length - 1], { key: 'ArrowDown', code: 'ArrowDown' });
  const options = await screen.findAllByRole('option');
  return options.map((option) => option.textContent ?? '');
}

describe('SwitchModelModal — Llama Server fit words', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([llamaProvider]);
    mocks.getProviderModels.mockResolvedValue([]);
    mocks.read.mockResolvedValue('');
  });

  it('says the Settings row words on a Mac', async () => {
    mocks.llamacppStatus.mockResolvedValue(status(['suitable', 'above_recommendation']));
    render(
      <SwitchModelModal
        sessionId={null}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="llamacpp"
      />
    );

    const [qwen, gemma] = await optionTexts();
    expect(qwen).toContain(localModelsCopy.fitsThisMac);
    expect(gemma).toContain(localModelsCopy.largeForThisMac);
    for (const retired of ['Recommended here', 'GPU memory', 'VRAM unknown', 'GiB']) {
      expect(`${qwen} ${gemma}`).not.toContain(retired);
    }
  });

  it('names the minimum and a CPU-only machine as the rows do', async () => {
    mocks.llamacppStatus.mockResolvedValue(
      status(['below_minimum', 'cpu_only'], { accelerator_memory_gib: 16 })
    );
    render(
      <SwitchModelModal
        sessionId={null}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="llamacpp"
      />
    );

    const [qwen, gemma] = await optionTexts();
    expect(qwen).toContain(localModelsCopy.belowMinimum(32));
    expect(gemma).toContain(localModelsCopy.cpuOnly);
  });

  it('says the Settings row state word, not a second one', async () => {
    const response = status(['suitable', 'suitable']);
    response.data.catalog[0] = {
      ...response.data.catalog[0],
      download_status: 'downloaded',
      download_source: 'huggingface',
    } as LlamaCppModel;
    mocks.llamacppStatus.mockResolvedValue(response);
    render(
      <SwitchModelModal
        sessionId={null}
        onClose={vi.fn()}
        setView={vi.fn()}
        initialProvider="llamacpp"
      />
    );

    const [qwen, gemma] = await optionTexts();
    expect(qwen).toBe(
      `Qwen3.8 27B19 GB · ${localModelsCopy.installed} · ${localModelsCopy.fitsThisMac}`
    );
    expect(gemma).toContain(localModelsCopy.status.needsDownload);
  });
});
