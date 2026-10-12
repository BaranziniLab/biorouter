/**
 * @vitest-environment jsdom
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModelAndProviderProvider, useModelAndProvider } from './ModelAndProviderContext';
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
    expect(dialog).toHaveAccessibleDescription(
      'Qwen3.6 35B runs on this computer. First use can take a while because the model may need to load, download, and produce a test response.'
    );
  });

  it('draws the warnings as a note and the detail as a filled panel, with no outlined box', async () => {
    const dialog = await openTheSheet();
    // The 8 GiB machine against a 64 GiB recommendation, and Windows: two warnings, in one note.
    const note = within(dialog).getByText(/recommends 64 GiB GPU-addressable memory/);
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
