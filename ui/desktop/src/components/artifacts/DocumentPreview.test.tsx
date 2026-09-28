import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from '../../styles/themes.generated';
import DocumentPreview from './DocumentPreview';
import type { ArtifactDocumentFormat, ArtifactFilePreview } from './artifactTypes';

const renderDocx = vi.fn(async (_data: ArrayBuffer, container: HTMLElement) => {
  const wrapper = document.createElement('div');
  wrapper.className = 'docx-wrapper';
  const page = document.createElement('section');
  page.className = 'docx';
  page.textContent = 'Genome report';
  page.getBoundingClientRect = vi.fn(() => ({
    bottom: 800,
    height: 800,
    left: 0,
    right: 640,
    top: 0,
    width: 640,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  }));
  wrapper.append(page);
  container.append(wrapper);
});
const cancelPdfRender = vi.fn();
const renderPdfPage = vi.fn(() => ({ promise: Promise.resolve(), cancel: cancelPdfRender }));
const cleanupPdfPage = vi.fn();
const getPdfPage = vi.fn(async () => ({
  getViewport: ({ scale }: { scale: number }) => ({ width: 612 * scale, height: 792 * scale }),
  render: renderPdfPage,
  cleanup: cleanupPdfPage,
}));
const destroyPdf = vi.fn(async () => undefined);
// Typed parameter, not `()`, so the asset-configuration test can read
// `mock.calls[0][0]` — with no declared argument the tuple is empty and
// inspecting the options object is a type error.
const getPdfDocument = vi.fn((_options: Record<string, unknown>) => ({
  promise: Promise.resolve({ numPages: 2, getPage: getPdfPage, destroy: destroyPdf }),
  destroy: destroyPdf,
}));
const renderWorkbook = vi.fn(async () => [
  '<html><head></head><body><script>window.bad = true</script><table><tr><td style="color: rgba(255,255,255,0); background-color: rgba(23,59,87,0)">Gene</td></tr></table></body></html>',
]);
const destroyPresentation = vi.fn();
// The options parameter is declared so the centring test can read
// `mock.calls[0][2].scrollContainer`; without it the tuple has two entries.
const renderPresentation = vi.fn(
  async (
    _data: ArrayBuffer,
    container: HTMLElement,
    _options: { scrollContainer?: HTMLElement | null }
  ) => {
    container.textContent = 'PowerPoint slide';
    return { destroy: destroyPresentation };
  }
);

vi.mock('docx-preview', () => ({ renderAsync: renderDocx }));
// The non-legacy entry point, and `workerPort` rather than `workerSrc` — both
// deliberate. jsdom has no Worker, so the component's `new Worker(...)` is
// stubbed in the setup below.
vi.mock('pdfjs-dist/legacy/build/pdf.mjs', () => ({
  getDocument: getPdfDocument,
  GlobalWorkerOptions: { workerPort: null },
}));
vi.mock('xlsx-preview', () => ({ xlsx2Html: renderWorkbook }));
vi.mock('@aiden0z/pptx-renderer', () => ({
  PptxViewer: { open: renderPresentation },
  RECOMMENDED_ZIP_LIMITS: {},
}));

// jsdom implements no Worker. `PdfPreview` constructs one for pdf.js's
// `workerPort`, so without this every PDF test throws before it renders.
class StubWorker {
  terminate() {}
  postMessage() {}
  addEventListener() {}
  removeEventListener() {}
}
vi.stubGlobal('Worker', StubWorker);

function documentFile(format: ArtifactDocumentFormat) {
  return {
    kind: 'document',
    format,
    title: `preview.${format}`,
    path: `/work/preview.${format}`,
    mimeType:
      format === 'pdf' ? 'application/pdf' : 'application/vnd.openxmlformats-officedocument',
    data: new Uint8Array([1, 2, 3]).buffer,
    size: 3,
    found: true,
  } satisfies Extract<ArtifactFilePreview, { kind: 'document' }>;
}

describe('DocumentPreview', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal('IntersectionObserver', undefined);
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(
          private readonly callback: (
            entries: Array<{ contentRect: { width: number } }>,
            observer: unknown
          ) => void
        ) {}

        observe() {
          this.callback([{ contentRect: { width: 480 } }], this);
        }

        disconnect() {}
        unobserve() {}
      }
    );
  });

  it('renders PDF pages without the embedded Chromium controls', async () => {
    render(<DocumentPreview file={documentFile('pdf')} resolvedTheme="light" isResizing={false} />);

    const preview = await screen.findByLabelText('preview.pdf PDF preview');
    expect(preview.tagName).toBe('DIV');
    expect(screen.queryByRole('iframe')).toBeNull();
    expect(await screen.findByLabelText('Page 1 of 2')).toBeVisible();
    expect(screen.getByLabelText('Page 2 of 2')).toBeVisible();
    await waitFor(() => expect(getPdfPage).toHaveBeenCalledTimes(2));
    await waitFor(() => {
      expect(document.querySelectorAll('canvas[data-rendered="true"]')).toHaveLength(2);
    });
  });

  it('releases a PDF page canvas backing store after the page leaves the viewport margin', async () => {
    const intersectionCallbacks: Array<(entries: Array<{ isIntersecting: boolean }>) => void> = [];
    vi.stubGlobal(
      'IntersectionObserver',
      class {
        constructor(callback: (entries: Array<{ isIntersecting: boolean }>) => void) {
          intersectionCallbacks.push(callback);
        }

        observe() {}
        disconnect() {}
        unobserve() {}
      }
    );

    render(<DocumentPreview file={documentFile('pdf')} resolvedTheme="light" isResizing={false} />);

    await screen.findByLabelText('preview.pdf PDF preview');
    await waitFor(() => expect(intersectionCallbacks).toHaveLength(2));
    const firstCanvas = screen.getByLabelText('Page 1 of 2').querySelector('canvas');
    expect(firstCanvas).not.toBeNull();
    await waitFor(() => expect(firstCanvas).toHaveProperty('width', 0));

    act(() => intersectionCallbacks[0]([{ isIntersecting: true }]));
    await waitFor(() => expect(firstCanvas!.width).toBeGreaterThan(0));
    await waitFor(() => expect(firstCanvas).toHaveAttribute('data-rendered', 'true'));

    act(() => intersectionCallbacks[0]([{ isIntersecting: false }]));
    await waitFor(() => expect(firstCanvas).toHaveProperty('width', 0));
    expect(firstCanvas).toHaveProperty('height', 0);
    expect(cleanupPdfPage).toHaveBeenCalled();
  });

  // The regression this guards is not cosmetic and does not announce itself.
  // pdf.js 6 loads its JPEG 2000 and JBIG2 decoders from `wasmUrl`; without it
  // a scanned or medical PDF renders with its images silently missing — no
  // error, no placeholder. The panel shipped that way. `cMapUrl` is the same
  // shape of failure for CJK glyphs.
  it('gives pdf.js its offline decoders, cmaps, fonts and colour profiles', async () => {
    render(<DocumentPreview file={documentFile('pdf')} resolvedTheme="light" isResizing={false} />);
    await screen.findByLabelText('preview.pdf PDF preview');

    expect(getPdfDocument).toHaveBeenCalledTimes(1);
    const options = getPdfDocument.mock.calls[0][0];

    for (const key of ['wasmUrl', 'cMapUrl', 'standardFontDataUrl', 'iccUrl']) {
      const value = options[key];
      expect(value, `${key} must be configured`).toEqual(expect.any(String));
      // pdf.js throws "Invalid factory url … must include trailing slash"
      // rather than degrading, so this is a hard failure at load, not a nicety.
      expect(String(value).endsWith('/'), `${key} must end in a trailing slash`).toBe(true);
    }
    expect(options.cMapPacked).toBe(true);
  });

  // pdf.js defaults `maxImageSize` to -1, meaning unlimited, and the page-level
  // cap cannot substitute: it reads the viewport, which comes from the declared
  // MediaBox. A 612x792 page carrying one `/Width 30000 /Height 30000` image
  // clears that cap and still asks the worker for ~3.6 GB.
  it('bounds the pixels pdf.js will decode for a single embedded image', async () => {
    render(<DocumentPreview file={documentFile('pdf')} resolvedTheme="light" isResizing={false} />);
    await screen.findByLabelText('preview.pdf PDF preview');

    const { maxImageSize } = getPdfDocument.mock.calls[0][0];
    expect(maxImageSize).toEqual(expect.any(Number));
    expect(maxImageSize).toBeGreaterThan(0);
    // Above a 600 DPI full-page scan (~35 MP), which this panel has to render,
    // and far below the gigapixel decode the cap exists to refuse.
    expect(maxImageSize).toBeGreaterThanOrEqual(35_000_000);
    expect(maxImageSize).toBeLessThanOrEqual(100_000_000);
  });

  it('renders Word documents as pages fitted to the preview width', async () => {
    render(
      <DocumentPreview file={documentFile('docx')} resolvedTheme="light" isResizing={false} />
    );

    const page = await screen.findByText('Genome report');
    expect(page).toBeInTheDocument();
    await waitFor(() => expect(page).toHaveAttribute('data-preview-scale', '0.7125'));
    expect(renderDocx).toHaveBeenCalledOnce();
  });

  it('renders workbook sheets in a script-free frame', async () => {
    render(<DocumentPreview file={documentFile('xlsx')} resolvedTheme="dark" isResizing={false} />);

    const frame = await screen.findByLabelText('preview.xlsx, sheet 1');
    await waitFor(() => expect(frame.getAttribute('srcdoc')).toContain('Gene'));
    expect(frame.getAttribute('srcdoc')).not.toContain('<script');
    expect(frame.getAttribute('srcdoc')).toContain('color: rgb(255, 255, 255)');
    expect(frame.getAttribute('srcdoc')).toContain('background-color: rgb(23, 59, 87)');
    expect(frame).toHaveAttribute('sandbox', '');
  });

  it('revokes workbook blob URLs discovered after the preview unmounts', async () => {
    let resolveWorkbook!: (sheets: string[]) => void;
    renderWorkbook.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveWorkbook = resolve;
        })
    );
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, 'revokeObjectURL', {
      configurable: true,
      value: revokeObjectURL,
    });
    const view = render(
      <DocumentPreview file={documentFile('xlsx')} resolvedTheme="light" isResizing={false} />
    );
    await waitFor(() => expect(renderWorkbook).toHaveBeenCalledOnce());

    view.unmount();
    resolveWorkbook([
      '<html><head></head><body><img src="blob:https://preview.test/late-unmount"></body></html>',
    ]);

    await waitFor(() =>
      expect(revokeObjectURL).toHaveBeenCalledWith('blob:https://preview.test/late-unmount')
    );
  });

  it('revokes blob URLs from a workbook conversion invalidated by a theme change', async () => {
    let resolveWorkbook!: (sheets: string[]) => void;
    renderWorkbook.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveWorkbook = resolve;
        })
    );
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, 'revokeObjectURL', {
      configurable: true,
      value: revokeObjectURL,
    });
    const view = render(
      <DocumentPreview file={documentFile('xlsx')} resolvedTheme="light" isResizing={false} />
    );
    await waitFor(() => expect(renderWorkbook).toHaveBeenCalledOnce());

    view.rerender(
      <DocumentPreview file={documentFile('xlsx')} resolvedTheme="dark" isResizing={false} />
    );
    await waitFor(() => expect(renderWorkbook).toHaveBeenCalledTimes(2));
    resolveWorkbook([
      '<html><head></head><body><img src="blob:https://preview.test/late-theme"></body></html>',
    ]);

    await waitFor(() =>
      expect(revokeObjectURL).toHaveBeenCalledWith('blob:https://preview.test/late-theme')
    );
  });

  it('renders PowerPoint slides and disposes the viewer on unmount', async () => {
    const view = render(
      <DocumentPreview file={documentFile('pptx')} resolvedTheme="light" isResizing={false} />
    );

    expect(await screen.findByText('PowerPoint slide')).toBeInTheDocument();
    expect(renderPresentation).toHaveBeenCalledOnce();
    await waitFor(() => {
      expect(document.querySelector('.artifact-pptx-preview')).toHaveAttribute(
        'data-rendered',
        'true'
      );
    });
    view.unmount();
    expect(destroyPresentation).toHaveBeenCalledOnce();
  });

  // The renderer sizes each slide from the width of the element it renders
  // into, read as `clientWidth`, which includes padding. When the gutter sat on
  // that element every slide was as wide as the whole panel: 12px from the left
  // edge and 12px past the right one, clipped, with 24px of sideways scroll.
  // jsdom has no layout, so this pins the structure that makes it impossible:
  // the measured element is unpadded and a separate parent scrolls.
  it('measures PowerPoint slides against an unpadded element inside a separate scroller', async () => {
    render(
      <DocumentPreview file={documentFile('pptx')} resolvedTheme="light" isResizing={false} />
    );
    await waitFor(() => expect(renderPresentation).toHaveBeenCalledOnce());

    const [, measured, options] = renderPresentation.mock.calls[0];
    expect(measured).toHaveClass('artifact-pptx-preview');
    const utilities = measured.className.split(/\s+/);
    expect(utilities.filter((name) => /^(p|px|pl|pr|ps|pe)-/.test(name))).toEqual([]);
    expect(utilities.filter((name) => name.startsWith('overflow-'))).toEqual([]);

    const scroller = options.scrollContainer;
    expect(scroller).not.toBe(measured);
    expect(scroller).toBe(measured.parentElement);
    expect(scroller).toHaveClass('artifact-pptx-scroll', 'artifact-document-scroll');
  });

  it('reserves the scrollbar gutter on both edges of every native-size document', async () => {
    const pdf = render(
      <DocumentPreview file={documentFile('pdf')} resolvedTheme="light" isResizing={false} />
    );
    expect(await screen.findByLabelText('preview.pdf PDF preview')).toHaveClass(
      'artifact-document-scroll'
    );
    pdf.unmount();

    const word = render(
      <DocumentPreview file={documentFile('docx')} resolvedTheme="light" isResizing={false} />
    );
    await screen.findByText('Genome report');
    expect(document.querySelector('.artifact-docx-preview')?.parentElement).toHaveClass(
      'artifact-document-scroll'
    );
    word.unmount();
  });
});

/**
 * The centring itself is asserted at the SOURCE, as `styles/composerFocus.test.ts`
 * does: jsdom has no layout engine, so no component test can measure the gap on
 * either side of a page. Measured in the real app when this landed: a PowerPoint
 * slide sat 12px from the left edge and -12px from the right, now 12/12; with a
 * classic scrollbar Word sat at 12/22 and PowerPoint at 12/-2, now 22/22 for both.
 */
describe('native-size document gutters (authored CSS)', () => {
  const css = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
  const ruleBody = (selector: string) =>
    css.match(new RegExp(`(?:^|\\n)${selector.replace(/[.]/g, '\\.')}\\s*\\{([^}]*)\\}`))?.[1];

  it('mirrors a classic scrollbar onto the left edge of every document scroller', () => {
    expect(ruleBody('.artifact-document-scroll')).toMatch(/scrollbar-gutter:\s*stable both-edges;/);
  });

  it('puts the deck gutter on the scroller, never on the element the renderer measures', () => {
    expect(ruleBody('.artifact-pptx-scroll')).toMatch(/padding:\s*16px 12px;/);
    expect(css).not.toMatch(/\.artifact-pptx-preview[^{,]*\{[^}]*padding/);
  });
});

describe('workbook sheet theming', () => {
  afterEach(() => localStorage.clear());

  async function srcdocFor(family: string, resolvedTheme: 'light' | 'dark') {
    localStorage.setItem('theme_family', family);
    const view = render(
      <ThemeProvider>
        <DocumentPreview
          file={documentFile('xlsx')}
          resolvedTheme={resolvedTheme}
          isResizing={false}
        />
      </ThemeProvider>
    );
    const frame = await screen.findByLabelText('preview.xlsx, sheet 1');
    await waitFor(() => expect(frame.getAttribute('srcdoc')).toContain('Gene'));
    const srcdoc = frame.getAttribute('srcdoc') ?? '';
    view.unmount();
    return srcdoc;
  }

  // The sheet document is sandboxed under `default-src 'none'`, so it cannot
  // load the app stylesheet and must inline literal colours. They used to be a
  // single hardcoded light/dark pair, which pinned every family to Parchment.
  it.each(THEME_FAMILY_IDS)('paints the %s ground, ink and table fill', async (family) => {
    for (const mode of ['light', 'dark'] as const) {
      const { background, foreground, card } = GENERATED_THEMES[family][mode].surface;
      const srcdoc = await srcdocFor(family, mode);
      expect(srcdoc).toContain(`color: ${foreground}`);
      expect(srcdoc).toContain(`background: ${background}`);
      expect(srcdoc).toContain(`table { background: ${card}; }`);
    }
  });

  // Without this, re-hardcoding a colour would leave every per-family
  // assertion above still passing for whichever family the hardcode matched.
  it('gives each family a distinct dark sheet', async () => {
    const rendered: string[] = [];
    for (const family of THEME_FAMILY_IDS) rendered.push(await srcdocFor(family, 'dark'));
    expect(new Set(rendered).size).toBe(THEME_FAMILY_IDS.length);
  });

  // Theming must not have loosened the sandbox that makes this safe to render.
  it('keeps the strict CSP and script stripping', async () => {
    const srcdoc = await srcdocFor('alma-mater', 'dark');
    expect(srcdoc).toContain("default-src 'none'");
    expect(srcdoc).not.toContain('<script');
  });
});
