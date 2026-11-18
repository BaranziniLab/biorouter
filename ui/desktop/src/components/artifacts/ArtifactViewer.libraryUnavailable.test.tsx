import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import { inlineArtifactCdnAssets } from '../../utils/artifactCdnAssets';
import ArtifactViewer, { LIBRARY_UNAVAILABLE_REASON } from './ArtifactViewer';

/**
 * A CDN-mode figure whose library the desktop app could not download (rq2-138).
 *
 * The figure used to say "d3 is not defined" in a red card, and the panel
 * forwarded that to the agent as a render error to fix: one paid turn that no
 * edit to the figure could satisfy, because the fault was the network. Now the
 * rewriter records the library it could not fetch, the figure's shared runtime
 * says so in one plain line and tags the error, and the panel does not start a
 * repair for it.
 */

const here = dirname(fileURLToPath(import.meta.url));
const COMMON_JS = readFileSync(
  resolve(here, '../../../../../crates/biorouter-mcp/src/autovisualiser/templates/_common.js'),
  'utf-8'
);
const D3 = 'https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js';
const PLAIN_LINE =
  'This figure needs a chart library that could not be downloaded. Check the connection, then open the figure again.';

type RenderErrorMessage = {
  type: 'biorouter-viz-render-error';
  payload: { message: string; detail: string; reason?: string };
};

/**
 * Run a figure through the real rewriter and the real shared runtime: its
 * library tag goes through `inlineArtifactCdnAssets` with the network down,
 * then each script runs, in order, in a frame of its own (jsdom has no
 * `srcdoc`, so they are appended one by one). Returns the frame's document and
 * whatever the figure posted to the panel.
 */
async function runFigure(draw: string, fetchFails: boolean) {
  const figure =
    `<head><script src="${D3}"></script><script>${COMMON_JS}</script></head>` +
    `<body><script>${draw}</script></body>`;
  const html = await inlineArtifactCdnAssets(figure, async () => {
    if (fetchFails) throw new TypeError('fetch failed');
    return 'window.d3 = { select: function () { return null; } };';
  });

  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const doc = frame.contentDocument as Document;
  doc.body.innerHTML = '<div class="viz-root"></div>';

  // The window the figure posts to. Not the test's `window`: vitest copies the
  // jsdom window's properties onto Node's global, while the frame's parent is
  // the jsdom window itself.
  const panel = (frame.contentWindow as Window).parent;
  const posted: unknown[] = [];
  const postMessage = vi.spyOn(panel, 'postMessage').mockImplementation((data: unknown) => {
    posted.push(data);
  });
  try {
    for (const [, source] of html.matchAll(/<script>([\s\S]*?)<\/script>/g)) {
      const script = doc.createElement('script');
      script.textContent = source;
      doc.head.appendChild(script);
    }
  } finally {
    postMessage.mockRestore();
  }
  const renderErrors = posted.filter(
    (data): data is RenderErrorMessage =>
      (data as { type?: string } | null)?.type === 'biorouter-viz-render-error'
  );
  return { doc, renderErrors };
}

/**
 * One of the figure's own theme colours (`BioRouterViz.colors`), as jsdom
 * reports an inline colour, so a card painted in it compares equal.
 */
function themeInk(doc: Document, name: 'text' | 'border'): string {
  const figureWindow = doc.defaultView as unknown as {
    BioRouterViz: { colors: Record<string, string> };
  };
  const probe = doc.createElement('div');
  probe.style.color = figureWindow.BioRouterViz.colors[name];
  expect(probe.style.color).not.toBe('');
  return probe.style.color;
}

function installElectron() {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    writable: true,
    value: {
      prepareArtifactHtml: vi.fn(async ({ html }: { html: string }) => ({ html })),
      readArtifactFile: vi.fn(),
      openDirectoryInExplorer: vi.fn(),
      openExternal: vi.fn(),
      openArtifactInBrowser: vi.fn(),
      platform: 'darwin',
    },
  });
}

async function mountFigurePanel(onRenderError: () => void) {
  installElectron();
  const { container } = render(
    <ThemeProvider>
      <ArtifactViewer
        artifact={{
          kind: 'html',
          title: 'Sankey Diagram',
          html: '<!doctype html><html><body><div class="viz-root"></div></body></html>',
        }}
        onClose={vi.fn()}
        onOpenArtifact={vi.fn()}
        onRenderError={onRenderError}
      />
    </ThemeProvider>
  );
  let frame: HTMLIFrameElement | null = null;
  await waitFor(() => {
    frame = container.querySelector('iframe');
    expect(frame).not.toBeNull();
  });
  return frame as unknown as HTMLIFrameElement;
}

/** Post `data` as the panel's own figure frame, the only sender it trusts. */
const postFromFigure = (frame: HTMLIFrameElement, data: unknown) =>
  window.dispatchEvent(new MessageEvent('message', { source: frame.contentWindow, data }));

beforeEach(() => {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: vi.fn().mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  });
});

afterEach(() => {
  cleanup();
  document.querySelectorAll('body > iframe').forEach((frame) => frame.remove());
});

describe('a figure whose library could not be downloaded', () => {
  it('the_figure_names_a_missing_library_plainly', async () => {
    const { doc, renderErrors } = await runFigure(
      "BioRouterViz.guard(function () { d3.select('.viz-root'); });",
      true
    );

    const card = doc.querySelector('[role="alert"]') as HTMLElement;
    expect(card).not.toBeNull();
    // One plain line, and nothing of the ReferenceError behind it.
    expect(card.textContent).toBe(PLAIN_LINE);
    expect(card.textContent).not.toContain('not defined');
    // Neutral ink from the figure's own theme, not a red card.
    expect(card.style.color).toBe(themeInk(doc, 'text'));
    expect(card.style.borderColor).toBe(themeInk(doc, 'border'));

    expect(renderErrors).toHaveLength(1);
    expect(renderErrors[0].payload.message).toBe(PLAIN_LINE);
    expect(renderErrors[0].payload.reason).toBe(LIBRARY_UNAVAILABLE_REASON);
    expect(renderErrors[0].payload.detail).toContain('d3');
  });

  it('an ordinary failure still reports its own error, with no library reason', async () => {
    const { doc, renderErrors } = await runFigure(
      "BioRouterViz.guard(function () { throw new Error('bad data point'); });",
      false
    );

    const card = doc.querySelector('[role="alert"]') as HTMLElement;
    expect(card.textContent).toContain('This visualization could not be rendered.');
    expect(card.textContent).toContain('bad data point');
    expect(card.style.color).toBe(themeInk(doc, 'text'));
    expect(renderErrors).toHaveLength(1);
    expect(renderErrors[0].payload.reason).toBeUndefined();
    expect(renderErrors[0].payload.detail).toBe('bad data point');
  });

  it('a_library_unavailable_error_does_not_start_a_repair_turn', async () => {
    const { renderErrors } = await runFigure(
      "BioRouterViz.guard(function () { d3.select('.viz-root'); });",
      true
    );
    expect(renderErrors).toHaveLength(1);
    expect(renderErrors[0].payload.reason).toBe(LIBRARY_UNAVAILABLE_REASON);
    const onRenderError = vi.fn();
    const frame = await mountFigurePanel(onRenderError);

    // The message exactly as the figure's runtime posted it.
    postFromFigure(frame, renderErrors[0]);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(onRenderError).not.toHaveBeenCalled();

    // An ordinary render error from the same frame still asks for a repair.
    postFromFigure(frame, {
      type: 'biorouter-viz-render-error',
      payload: {
        message: 'This visualization could not be rendered.',
        detail: "TypeError: Cannot read properties of undefined (reading 'value')",
      },
    });
    expect(onRenderError).toHaveBeenCalledTimes(1);
    expect(onRenderError).toHaveBeenCalledWith(
      expect.objectContaining({
        artifactTitle: 'Sankey Diagram',
        detail: "TypeError: Cannot read properties of undefined (reading 'value')",
      })
    );
  });
});
