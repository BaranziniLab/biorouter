/**
 * @vitest-environment jsdom
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ModelAndProviderProvider,
  useModelAndProvider,
  warmupFirstUseHelp,
  WARMUP_FIRST_USE_LABEL,
} from './ModelAndProviderContext';
import { MODAL_ANCHOR_TOP_STYLE } from './ModalShell';
import type Model from './settings/models/modelInterface';

/**
 * The local-model warm-up sheet must fit the window, and the part that overflows
 * must be the DETAIL, never the buttons.
 *
 * The bug: its `DialogContent` had no height ceiling and no scrolling body. The
 * detail block grows with the model (blob path, suitability message, fallback
 * state, context and GPU-memory rows) and a dialog is centred with
 * `top-1/2 -translate-y-1/2`, so once the content exceeded the viewport the
 * sheet grew symmetrically past BOTH edges. Its own footer went off the bottom
 * of the screen, taking "Warm up" and "Keep previous" with it. That is not an
 * ugly dialog, it is an unreachable one: the sheet is modal, so with neither
 * button clickable the only way out is Escape.
 *
 * It is a `ModalShell` now (vision round 7, V3-R7-1: the last raw dialog in the
 * models flow), with `scrollBody`: the shell pins the dialog's top edge, caps its
 * height against the window, scrolls ONE body between the header and footer
 * rules and keeps the footer in view.
 *
 * ⚠ **The ceiling is asserted on the real DOM and on the source, because jsdom
 * has no layout engine.** Every box reports zero size, `max-height` clamps
 * nothing and an overflowing child never scrolls, so a test that rendered this
 * dialog at a short viewport and asked whether the footer was on screen would
 * answer the same way with the bug present and with it fixed. What can be pinned
 * is the construction: the shell's viewport-relative ceiling, the one scroller
 * with `min-h-0 flex-1` (without `min-h-0` a flex item refuses to shrink below
 * its content and the scrollbar never appears), and the header and footer being
 * OUTSIDE it.
 */
const SOURCE = readFileSync(join(__dirname, 'ModelAndProviderContext.tsx'), 'utf8');

const mocks = vi.hoisted(() => ({
  llamacppStatus: vi.fn(),
  llamacppWarmup: vi.fn(),
  setConfigProvider: vi.fn(),
  updateAgentProvider: vi.fn(),
  read: vi.fn(),
  getProviders: vi.fn(),
  refreshConfig: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock('../api', () => ({
  llamacppStatus: mocks.llamacppStatus,
  llamacppWarmup: mocks.llamacppWarmup,
  setConfigProvider: mocks.setConfigProvider,
  updateAgentProvider: mocks.updateAgentProvider,
}));

vi.mock('../toasts', () => ({
  toastError: mocks.toastError,
  toastSuccess: mocks.toastSuccess,
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    getProviders: mocks.getProviders,
    refreshConfig: mocks.refreshConfig,
  }),
}));

// The model picker is the person's act, and the daemon tells it from a model by this header.
Object.defineProperty(window, 'electron', {
  writable: true,
  value: { getUserActionKey: async () => 'user-action-key-under-test' },
});

const llamaModel: Model = {
  name: 'qwen3.6',
  provider: 'llamacpp',
  alias: 'Qwen3.6',
  subtext: 'Llama Server',
  context_limit: 131072,
};

const sidecar = {
  state: 'stopped',
  model: null,
  hf_spec: null,
  ollama_name: null,
  model_path: null,
  model_source: null,
  port: 11543,
  binary_path: '/opt/homebrew/bin/llama-server',
  build: 'test',
  detail: null,
  context_size: null,
  warmed: false,
};

const statusResponse = {
  data: {
    sidecar,
    catalog: [
      {
        name: 'qwen3.6',
        display_name: 'Qwen3.6 35B',
        ollama_name: 'qwen3.6:latest',
        hf_spec: 'unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q4_K_M',
        download_size: '23 GB',
        description: 'Large Qwen3.6 MoE model from Ollama library.',
        min_gpu_memory_gib: 48,
        recommended_gpu_memory_gib: 64,
        context_limit: 262144,
        downloaded: false,
        download_status: 'not_downloaded',
        download_source: 'none',
        fallback_downloaded: false,
        fallback_download_status: 'not_downloaded',
        model_path: '/Users/test/.ollama/models/blobs/sha256-0123456789abcdef',
        suitable: false,
        suitability_status: 'above_recommendation',
        suitability_message:
          'This model recommends 64 GiB of GPU-addressable memory; this machine reports 8 GiB vram.',
        is_default: true,
      },
    ],
    system: {
      os: 'windows',
      total_memory_gib: 32,
      accelerator_memory_gib: 8,
      accelerator_memory_kind: 'vram',
      default_context_size: 32768,
      model_cache_dir: '/Users/test/.ollama/models',
      model_cache_layout: 'Ollama manifests/blobs; Hugging Face fallback cache under the same root',
    },
  },
};

function SwitchHarness() {
  const { changeModel } = useModelAndProvider();
  return (
    <button type="button" onClick={() => void changeModel(null, llamaModel)}>
      Switch to local
    </button>
  );
}

/** The same model, already on this computer: its Hugging Face file is in the cache. */
const downloadedStatus = {
  data: {
    ...statusResponse.data,
    catalog: [
      {
        ...statusResponse.data.catalog[0],
        downloaded: true,
        download_status: 'downloaded',
        download_source: 'huggingface_cache',
        fallback_downloaded: true,
        fallback_download_status: 'downloaded',
      },
    ],
  },
};

async function openTheSheet() {
  render(
    <ModelAndProviderProvider>
      <SwitchHarness />
    </ModelAndProviderProvider>
  );
  fireEvent.click(screen.getByRole('button', { name: 'Switch to local' }));
  await screen.findByText('Warm up local model');
  return screen.getByRole('dialog');
}

/** Open the folded detail list (owner, 2026-11-13: it starts closed, under its summary). */
function openTheDetail() {
  fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_MODEL') return 'gpt-4o';
    if (key === 'BIOROUTER_PROVIDER') return 'openai';
    return null;
  });
  mocks.getProviders.mockResolvedValue([]);
  mocks.llamacppStatus.mockResolvedValue(statusResponse);
  mocks.llamacppWarmup.mockResolvedValue({
    data: {
      output: 'pong',
      sidecar: { ...sidecar, state: 'ready', model: llamaModel.name, warmed: true },
    },
  });
  mocks.setConfigProvider.mockResolvedValue(undefined);
  mocks.updateAgentProvider.mockResolvedValue(undefined);
  mocks.refreshConfig.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('the local-model warm-up dialog fits the window', () => {
  it('is a ModalShell with a scrolling body, and no raw DialogContent is left in the file', () => {
    expect(SOURCE).toMatch(/<ModalShell\b/);
    // The bare attribute: `scrollBody={false}` would read as present to a looser match.
    expect(SOURCE).toMatch(/^\s+scrollBody$/m);
    expect(SOURCE).not.toMatch(/<DialogContent/);
  });

  it('pins its top edge and caps its height against the viewport', async () => {
    const dialog = await openTheSheet();
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    // The cap is applied inline by the shell (the constant is what it spreads), and it is
    // relative to the window: a ceiling in rem would still exceed a short one.
    expect(MODAL_ANCHOR_TOP_STYLE.maxHeight).toMatch(/vh/);
    const classes = dialog.className.split(/\s+/);
    expect(classes).toContain('flex');
    expect(classes).toContain('flex-col');
    expect(classes).toContain('overflow-hidden');
    expect(classes.find((token) => token.startsWith('max-h-'))).toMatch(/vh/);
  });

  it('scrolls the detail block between the header and the footer, not the sheet', async () => {
    const dialog = await openTheSheet();
    openTheDetail();
    const scrollers = [...dialog.querySelectorAll('.overflow-y-auto')];
    // Exactly one: a second scroller would mean the header or the footer had been folded into
    // the scrolled area, which is the failure this file is about.
    expect(scrollers).toHaveLength(1);
    const scroller = scrollers[0];
    const scrollerClasses = scroller.className.split(/\s+/);
    expect(scrollerClasses).toContain('min-h-0');
    expect(scrollerClasses).toContain('flex-1');

    expect(scroller).toContainElement(screen.getByText('Model store'));
    expect(scroller).toContainElement(screen.getByText('Recommended GPU memory'));

    // The title, the sentence and both buttons stay outside it, so they stay in view.
    expect(scroller).not.toContainElement(screen.getByText('Warm up local model'));
    expect(scroller).not.toContainElement(screen.getByRole('button', { name: 'Warm up model' }));
    expect(scroller).not.toContainElement(
      screen.getByRole('button', { name: 'Keep previous model' })
    );
    // A scrolling body gives the footer its own hairline, so the scroll edge is legible.
    const footer = screen.getByRole('button', { name: 'Warm up model' }).parentElement;
    expect(footer?.className.split(/\s+/)).toContain('border-t');
  });

  it('names the model in the sentence that describes the dialog', async () => {
    const dialog = await openTheSheet();
    expect(dialog).toHaveAccessibleName('Warm up local model');
    // One line under the title; why first use is slow is the title's InfoTip (owner, 2026-11-13).
    expect(dialog).toHaveAccessibleDescription('Qwen3.6 35B runs on this computer.');
    expect(
      within(dialog).getByRole('button', { name: `About ${WARMUP_FIRST_USE_LABEL}` })
    ).toHaveAccessibleDescription(warmupFirstUseHelp(false));
    expect(warmupFirstUseHelp(false)).toMatch(/^First use can take a while/);
  });

  it('opens on the warning and the buttons, with the detail folded under its summary', async () => {
    const dialog = await openTheSheet();
    const toggle = within(dialog).getByRole('button', { name: 'Advanced' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    // The summary says the size and the fit word; the rows are not mounted until it opens.
    expect(within(dialog).getByTestId('warmup-summary')).toHaveTextContent('23 GB · ');
    expect(within(dialog).queryByText('Model store')).toBeNull();
    // The warning leads with its consequence; what the memory figure means here is folded.
    expect(within(dialog).getByText(/This machine reports 8 GB VRAM\.$/)).toBeInTheDocument();
    expect(within(dialog).queryByText(/this means VRAM, not regular system RAM/)).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Details' }));
    expect(within(dialog).getByText(/this means VRAM, not regular system RAM/)).toBeInTheDocument();
    // The Windows reminder is the second warning, so it is folded with it.
    expect(within(dialog).getByText(/regular system RAM does not satisfy/)).toBeInTheDocument();
    openTheDetail();
    // Where the model lives is a place in sans (D-42); the Ollama tag is an id in mono.
    const store = within(dialog).getByText('/Users/test/.ollama/models');
    expect(store).toHaveAttribute('data-path', '/Users/test/.ollama/models');
    expect(store.closest('.font-mono')).toBeNull();
    expect(within(dialog).getByText('qwen3.6:latest')).toHaveClass('font-mono', 'text-code');
  });

  it('draws the warnings as a note and the detail as a filled panel, with no outlined box', async () => {
    const dialog = await openTheSheet();
    openTheDetail();
    // The 8 GB machine against a 64 GB recommendation, and Windows: two warnings, in one note.
    const note = within(dialog).getByText(/recommends 64 GB GPU-addressable memory/);
    expect(note.closest('.br-fill-surface')).not.toBeNull();
    expect(dialog.querySelector('.biorouter-modal-panel')).not.toBeNull();
    // Nothing in the body still names the sizes this redesign retired, or the outline it dropped.
    // (The header and footer rules are the shell's own, and structural.)
    const body = dialog.querySelector('.overflow-y-auto')?.innerHTML ?? '';
    expect(body).not.toBe('');
    expect(body).not.toMatch(
      /text-\[11px\]|text-xs|text-sm\b|border-border-subtle|bg-background-medium/
    );
  });
});

/**
 * W2-5. The sheet talked about a download for a model already on disk: a "Download: 23 GB" row
 * beside "Local copy: Downloaded", a sentence saying the model "may need to ... download", and the
 * fallback and Ollama store rows that only describe where a download would come from.
 */
describe('the warm-up dialog says only what is true of the local copy', () => {
  it('names the size as the model size, whichever way the copy stands', async () => {
    const dialog = await openTheSheet();
    openTheDetail();
    expect(within(dialog).getByText('Model size')).toBeInTheDocument();
    expect(within(dialog).getByText('23 GB')).toBeInTheDocument();
    expect(within(dialog).queryByText('Download')).toBeNull();
  });

  it('keeps the download wording and its details while the model still needs downloading', async () => {
    const dialog = await openTheSheet();
    // The one line under the title; why first use is slow, download and all, is its InfoTip.
    expect(dialog).toHaveAccessibleDescription('Qwen3.6 35B runs on this computer.');
    expect(
      within(dialog).getByRole('button', { name: `About ${WARMUP_FIRST_USE_LABEL}` })
    ).toHaveAccessibleDescription(/may need to load, download, and produce/);
    openTheDetail();
    expect(within(dialog).getByText('Needs download')).toBeInTheDocument();
    for (const label of ['Llama Server fallback', 'Ollama model', 'Model store', 'Model blob']) {
      expect(within(dialog).getByText(label)).toBeInTheDocument();
    }
  });

  it('drops the download wording and its details once the model is on this computer', async () => {
    mocks.llamacppStatus.mockResolvedValue(downloadedStatus);
    const dialog = await openTheSheet();
    expect(dialog).toHaveAccessibleDescription('Qwen3.6 35B runs on this computer.');
    expect(
      within(dialog).getByRole('button', { name: `About ${WARMUP_FIRST_USE_LABEL}` })
    ).toHaveAccessibleDescription(
      'First use can take a while because the model may need to load and produce a test response.'
    );
    openTheDetail();
    expect(within(dialog).getByText('Model size')).toBeInTheDocument();
    expect(within(dialog).getByText('Downloaded')).toBeInTheDocument();
    for (const label of ['Llama Server fallback', 'Ollama model', 'Model store', 'Model blob']) {
      expect(within(dialog).queryByText(label)).toBeNull();
    }
    expect(within(dialog).queryByText('/Users/test/.ollama/models')).toBeNull();
    // What still decides whether to go ahead stays: the fit, the window and the memory.
    expect(within(dialog).getByText('Default context')).toBeInTheDocument();
    expect(within(dialog).getByText('Recommended GPU memory')).toBeInTheDocument();
    expect(within(dialog).getByText('Large for this computer')).toBeInTheDocument();
  });

  it('says "Downloaded in Ollama" for a copy in the Ollama store, and still drops the details', async () => {
    mocks.llamacppStatus.mockResolvedValue({
      data: {
        ...downloadedStatus.data,
        catalog: [
          {
            ...downloadedStatus.data.catalog[0],
            download_source: 'ollama',
            fallback_downloaded: false,
            fallback_download_status: 'not_downloaded',
          },
        ],
      },
    });
    const dialog = await openTheSheet();
    openTheDetail();
    expect(within(dialog).getByText('Downloaded in Ollama')).toBeInTheDocument();
    expect(within(dialog).queryByText('Llama Server fallback')).toBeNull();
    expect(within(dialog).queryByText('Model blob')).toBeNull();
  });
});

describe('after the warm-up, the switch names the model as the dialog did', () => {
  // The picker's model object for a local row carries no alias, so the toast used to say `qwen3.6`.
  it('uses the catalog name when the picked model carries no alias', async () => {
    const user = userEvent.setup();
    function BareSwitch() {
      const { changeModel } = useModelAndProvider();
      return (
        <button
          type="button"
          onClick={() =>
            void changeModel(null, {
              name: 'qwen3.6',
              provider: 'llamacpp',
              subtext: 'Llama Server',
            } as Model)
          }
        >
          Switch bare
        </button>
      );
    }
    render(
      <ModelAndProviderProvider>
        <BareSwitch />
      </ModelAndProviderProvider>
    );
    fireEvent.click(screen.getByRole('button', { name: 'Switch bare' }));
    await screen.findByText('Warm up local model');
    await user.click(screen.getByRole('button', { name: 'Warm up model' }));
    await waitFor(() => expect(mocks.toastSuccess).toHaveBeenCalledTimes(1));
    expect(mocks.toastSuccess.mock.calls[0][0].msg).toBe(
      'New chats in every window now start on Qwen3.6 35B from Llama Server. Existing chats keep their own model.'
    );
  });
});

describe('the warm-up dialog keeps its decline and its work-in-flight rules', () => {
  it('Escape keeps the previous model', async () => {
    const user = userEvent.setup();
    await openTheSheet();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByText('Warm up local model')).not.toBeInTheDocument());
    expect(mocks.llamacppWarmup).not.toHaveBeenCalled();
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();
    expect(mocks.updateAgentProvider).not.toHaveBeenCalled();
  });

  it('the × keeps the previous model', async () => {
    const user = userEvent.setup();
    await openTheSheet();
    await user.click(screen.getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByText('Warm up local model')).not.toBeInTheDocument());
    expect(mocks.llamacppWarmup).not.toHaveBeenCalled();
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();
  });

  it('while the model warms up nothing closes it: no ×, Escape does nothing, Keep is off', async () => {
    const user = userEvent.setup();
    let finish: (value: unknown) => void = () => {};
    mocks.llamacppWarmup.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      })
    );
    const dialog = await openTheSheet();

    await user.click(screen.getByRole('button', { name: 'Warm up model' }));

    // The label says what is happening, and neither button can be pressed.
    const working = await screen.findByRole('button', { name: 'Warming up…' });
    expect(working).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Keep previous model' })).toBeDisabled();
    // The busy line names the wait, with the one spinner.
    expect(within(dialog).getByText('Waiting for the model to generate…')).toBeInTheDocument();
    expect(dialog.querySelector('.br-spinner')).not.toBeNull();
    // The work is in flight: there is no way out but the answer.
    expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();
    await user.keyboard('{Escape}');
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();

    // The answer arrives: the dialog closes and the switch is made.
    finish({
      data: {
        output: 'pong',
        sidecar: { ...sidecar, state: 'ready', model: llamaModel.name, warmed: true },
      },
    });
    await waitFor(() => expect(screen.queryByText('Warm up local model')).not.toBeInTheDocument());
    expect(mocks.setConfigProvider).toHaveBeenCalledTimes(1);
  });
});

/**
 * The fit, in plain words. The dialog printed the daemon's sentence as it came:
 * "Recommended for this machine: 128 GiB apple_unified meets the 48 GiB recommendation."
 * (live QA, 2026-11-12), the memory kind's id and a unit the rest of the app does not
 * use. It now says the verdict the Settings info dialog says, and the memory rows name
 * the kind in words.
 */
describe('the warm-up dialog says the fit in plain words', () => {
  const machine = (accelerator_memory_kind: string, os: string, gib: number | null = 128) => ({
    data: {
      ...statusResponse.data,
      catalog: [
        {
          ...statusResponse.data.catalog[0],
          min_gpu_memory_gib: 32,
          recommended_gpu_memory_gib: 48,
          suitable: true,
          suitability_status: 'suitable',
          suitability_message: `Recommended for this machine: ${gib} GiB ${accelerator_memory_kind} meets the 48 GiB recommendation.`,
        },
      ],
      system: {
        ...statusResponse.data.system,
        os,
        accelerator_memory_gib: gib,
        accelerator_memory_kind,
      },
    },
  });

  it.each([
    ['apple_unified', 'macos', '128 GB unified memory', 'Fits this Mac'],
    ['vram', 'linux', '128 GB VRAM', 'Fits this computer'],
    ['cpu_only', 'linux', '128 GB system RAM', 'Fits this computer'],
  ])('%s reads as words, never as the id', async (kind, os, detected, verdict) => {
    mocks.llamacppStatus.mockResolvedValue(machine(kind, os));
    const dialog = await openTheSheet();
    openTheDetail();
    const row = (label: string) => within(dialog).getByText(label).nextElementSibling;
    expect(row('Detected GPU memory')).toHaveTextContent(detected);
    expect(row('Recommended GPU memory')).toHaveTextContent('48 GB');
    expect(row('Recommendation')).toHaveTextContent(verdict);
    // No machine id and no daemon sentence anywhere in the sheet.
    expect(dialog).not.toHaveTextContent(kind);
    expect(dialog).not.toHaveTextContent('GiB');
    expect(dialog).not.toHaveTextContent('Recommended for this machine');
  });
});
