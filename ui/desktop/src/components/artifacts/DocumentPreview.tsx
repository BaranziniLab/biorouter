import { useEffect, useRef, useState } from 'react';
import type {
  PDFDocumentLoadingTask,
  PDFDocumentProxy,
  PDFPageProxy,
  RenderTask,
} from 'pdfjs-dist';
import { useThemeFamily, type ThemeFamily } from '../../contexts/ThemeContext';
import { cn } from '../../utils';
import type { ArtifactFilePreview } from './artifactTypes';
import { DOCUMENT_FIDELITY_NOTES } from '../../utils/formatSupport';
import { sandboxedSurface } from './artifactUtils';
import { createPdfWorker } from '../../utils/pdfCompat';

type DocumentFile = Extract<ArtifactFilePreview, { kind: 'document' }>;

type DocumentPreviewProps = {
  file: DocumentFile;
  resolvedTheme: 'light' | 'dark';
  isResizing: boolean;
};

function PreviewStatus({ message }: { message: string }) {
  return (
    <div className="absolute inset-0 flex items-center justify-center bg-background-default text-body text-text-muted">
      {message}
    </div>
  );
}

function releaseCanvasBackingStore(canvas: HTMLCanvasElement) {
  canvas.width = 0;
  canvas.height = 0;
}

function PdfPageCanvas({
  document,
  pageNumber,
  pageCount,
}: {
  document: PDFDocumentProxy;
  pageNumber: number;
  pageCount: number;
}) {
  const wrapperRef = useRef<HTMLElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [availableWidth, setAvailableWidth] = useState(0);
  const [isNearViewport, setIsNearViewport] = useState(
    () => typeof IntersectionObserver === 'undefined'
  );
  const [pageHeight, setPageHeight] = useState<number | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [rendered, setRendered] = useState(false);

  useEffect(() => {
    const wrapper = wrapperRef.current;
    if (!wrapper) return;

    const measure = (width: number) => setAvailableWidth(Math.max(0, Math.floor(width)));
    measure(wrapper.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) measure(entry.contentRect.width);
    });
    observer.observe(wrapper);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const wrapper = wrapperRef.current;
    if (!wrapper || typeof IntersectionObserver === 'undefined') return;

    const observer = new IntersectionObserver(
      (entries) => setIsNearViewport(entries.some((entry) => entry.isIntersecting)),
      { rootMargin: '700px 0px' }
    );
    observer.observe(wrapper);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    if (!isNearViewport || availableWidth === 0) {
      releaseCanvasBackingStore(canvas);
      setRendered(false);
      return;
    }

    let cancelled = false;
    let page: PDFPageProxy | null = null;
    let renderTask: RenderTask | null = null;
    setRendered(false);
    setRenderError(null);

    void document
      .getPage(pageNumber)
      .then((nextPage) => {
        if (cancelled) return;
        page = nextPage;
        const initialViewport = page.getViewport({ scale: 1 });
        const targetWidth = Math.min(4096, availableWidth);
        const viewport = page.getViewport({ scale: targetWidth / initialViewport.width });
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);

        if (
          viewport.width * pixelRatio > 8192 ||
          viewport.height * pixelRatio > 8192 ||
          viewport.width * viewport.height * pixelRatio * pixelRatio > 32_000_000
        ) {
          throw new Error('Page dimensions exceed the safe preview limit.');
        }

        canvas.width = Math.floor(viewport.width * pixelRatio);
        canvas.height = Math.floor(viewport.height * pixelRatio);
        canvas.style.width = `${Math.floor(viewport.width)}px`;
        canvas.style.height = `${Math.floor(viewport.height)}px`;
        setPageHeight(Math.floor(viewport.height));

        renderTask = page.render({
          canvas,
          viewport,
          transform: pixelRatio === 1 ? undefined : [pixelRatio, 0, 0, pixelRatio, 0, 0],
        });
        return renderTask.promise.then(() => {
          if (!cancelled) setRendered(true);
        });
      })
      .catch((cause: unknown) => {
        if (
          !cancelled &&
          !(cause instanceof Error && cause.name === 'RenderingCancelledException')
        ) {
          setRenderError(cause instanceof Error ? cause.message : 'Unknown rendering error');
        }
      });

    return () => {
      cancelled = true;
      renderTask?.cancel();
      page?.cleanup();
      releaseCanvasBackingStore(canvas);
    };
  }, [availableWidth, document, isNearViewport, pageNumber]);

  return (
    <figure
      ref={wrapperRef}
      aria-label={`Page ${pageNumber} of ${pageCount}`}
      className="flex w-full shrink-0 justify-center"
      style={{ minHeight: pageHeight ?? '48vh' }}
    >
      <canvas
        ref={canvasRef}
        data-rendered={rendered ? 'true' : 'false'}
        className={cn('bg-white shadow-md', renderError && 'hidden')}
      />
      {renderError && (
        <div className="flex min-h-48 w-full flex-col items-center justify-center gap-1 bg-background-default px-6 text-center text-body text-text-muted shadow-md">
          <span>Could not render page {pageNumber}</span>
          <span className="text-supporting text-text-subtle">{renderError}</span>
        </div>
      )}
    </figure>
  );
}

/**
 * Where pdf.js finds its runtime assets, copied into `public/pdfjs/` by
 * `vite-plugins/pdfjsAssets.mts`.
 *
 * ⚠ **Every one of these must end in a trailing slash** — pdf.js throws
 * `Invalid factory url: "..." must include trailing slash.` rather than
 * degrading, so a missing slash is a hard failure at load.
 *
 * The one that actually matters day to day is `wasmUrl`: it carries the JPEG
 * 2000 and JBIG2 decoders, without which *scanned and medical PDFs* render with
 * their images silently missing. That was the shipped behaviour before this.
 */
function pdfAssetOptions() {
  const base = new URL('pdfjs/', window.document.baseURI).href;
  return {
    cMapUrl: `${base}cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${base}standard_fonts/`,
    wasmUrl: `${base}wasm/`,
    iccUrl: `${base}iccs/`,
  };
}

/**
 * Total pixels pdf.js may decode for a single embedded image.
 *
 * ⚠ **pdf.js defaults this to `-1`, meaning unlimited**, and the page-level cap
 * above cannot stand in for it: that one reads the `viewport`, which comes from
 * the page's declared MediaBox — the *output* canvas — and says nothing about
 * the resolution of the image XObjects drawn onto it. A one-page PDF sized 612
 * x 792 can carry a `/Width 30000 /Height 30000` image over near-uniform data
 * in a few hundred KB; the page cap passes trivially and the worker allocates
 * ~3.6 GB decoding it before scaling down to 612px.
 *
 * 64 megapixels sits above the legitimate high-water mark this panel exists to
 * serve — a 600 DPI full-page scan is ~35 MP, and even 800 DPI is ~61 MP — while
 * capping one image's decode at ~256 MB of RGBA. Images past it are skipped by
 * pdf.js rather than failing the page.
 */
const PDF_MAX_IMAGE_PIXELS = 64_000_000;

function PdfPreview({ file, isResizing }: Pick<DocumentPreviewProps, 'file' | 'isResizing'>) {
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let loadingTask: PDFDocumentLoadingTask | null = null;
    let pdfWorker: Worker | null = null;
    let pdfjsModule: typeof import('pdfjs-dist/legacy/build/pdf.mjs') | null = null;
    const disposePdfRuntime = async () => {
      const task = loadingTask;
      const worker = pdfWorker;
      loadingTask = null;
      pdfWorker = null;
      await task?.destroy();
      worker?.terminate();
      if (pdfjsModule?.GlobalWorkerOptions.workerPort === worker) {
        pdfjsModule.GlobalWorkerOptions.workerPort = null;
      }
    };
    setDocument(null);
    setError(null);

    void import('pdfjs-dist/legacy/build/pdf.mjs')
      .then((pdfjs) => {
        if (cancelled) return null;
        // `workerPort`, not `workerSrc`. Under `file://` the origin serializes
        // to "null", so pdf.js decides the worker is cross-origin and routes
        // `workerSrc` through a `blob:` wrapper — which the renderer's
        // `worker-src 'self'` forbids, killing every PDF in the packaged app
        // while working fine against the dev server. `workerPort` hands pdf.js
        // a Worker we constructed and never touches that path.
        pdfWorker = createPdfWorker();
        pdfjsModule = pdfjs;
        pdfjs.GlobalWorkerOptions.workerPort = pdfWorker;
        const task = pdfjs.getDocument({
          data: new Uint8Array(file.data.slice(0)),
          maxImageSize: PDF_MAX_IMAGE_PIXELS,
          ...pdfAssetOptions(),
        });
        loadingTask = task;
        return task.promise;
      })
      .then((nextDocument) => {
        if (!nextDocument || cancelled) return;
        if (nextDocument.numPages > 500) {
          nextDocument.cleanup();
          throw new Error('This PDF has too many pages to preview safely.');
        }
        setDocument(nextDocument);
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : 'Could not render this PDF.');
        }
        void disposePdfRuntime();
      });

    return () => {
      cancelled = true;
      void disposePdfRuntime();
    };
  }, [file.data, file.path]);

  if (error) return <PreviewStatus message={error} />;
  if (!document) return <PreviewStatus message="Rendering PDF" />;

  return (
    <div
      aria-label={`${file.title} PDF preview`}
      className={cn(
        'artifact-document-scroll h-full overflow-y-auto bg-background-medium',
        isResizing && 'pointer-events-none'
      )}
    >
      <div className="mx-auto flex w-full max-w-5xl flex-col items-center gap-4 p-4">
        {Array.from({ length: document.numPages }, (_, index) => (
          <PdfPageCanvas
            key={index + 1}
            document={document}
            pageNumber={index + 1}
            pageCount={document.numPages}
          />
        ))}
      </div>
    </div>
  );
}

export function fitWordPages(container: HTMLElement, measuredWidth = container.clientWidth) {
  const availableWidth = Math.max(0, measuredWidth - 24);
  container.querySelectorAll<HTMLElement>('.docx-wrapper > section.docx').forEach((page) => {
    page.style.removeProperty('zoom');
    const cachedWidth = Number(page.dataset.previewNaturalWidth);
    const naturalWidth = cachedWidth || page.getBoundingClientRect().width || page.offsetWidth;
    if (naturalWidth === 0 || availableWidth === 0) return;

    page.dataset.previewNaturalWidth = String(naturalWidth);
    const scale = Math.min(1, availableWidth / naturalWidth);
    page.dataset.previewScale = String(scale);
    page.style.setProperty('zoom', String(scale));
  });
}

function WordPreview({ file }: Pick<DocumentPreviewProps, 'file'>) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rendered, setRendered] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const container = containerRef.current;
    if (!container) return;
    let observer: ResizeObserver | null = null;

    setError(null);
    setRendered(false);
    container.replaceChildren();

    void import('docx-preview')
      .then(({ renderAsync }) =>
        renderAsync(file.data.slice(0), container, container, {
          breakPages: true,
          ignoreLastRenderedPageBreak: false,
          renderHeaders: true,
          renderFooters: true,
          renderFootnotes: true,
          renderEndnotes: true,
          useBase64URL: true,
        })
      )
      .then(() => {
        if (cancelled) return;
        fitWordPages(container);
        if (typeof ResizeObserver !== 'undefined') {
          observer = new ResizeObserver((entries) => {
            const entry = entries[0];
            if (entry) fitWordPages(container, entry.contentRect.width);
          });
          observer.observe(container);
        }
        setRendered(true);
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : 'Could not render this Word document.');
        }
      });

    return () => {
      cancelled = true;
      observer?.disconnect();
      container.replaceChildren();
    };
  }, [file.data, file.path]);

  return (
    <div className="artifact-document-scroll relative h-full overflow-auto bg-background-medium">
      <div ref={containerRef} className="artifact-docx-preview min-h-full" />
      {!rendered && !error && <PreviewStatus message="Rendering Word document" />}
      {error && <PreviewStatus message={error} />}
    </div>
  );
}

function prepareSpreadsheetHtml(
  source: string,
  resolvedTheme: 'light' | 'dark',
  themeFamily: ThemeFamily
) {
  const document = new DOMParser().parseFromString(source, 'text/html');
  document
    .querySelectorAll(
      'script, base, link, iframe, frame, frameset, embed, form, input, button, textarea, select, video, audio, meta[http-equiv]'
    )
    .forEach((element) => element.remove());
  document.querySelectorAll<HTMLElement>('*').forEach((element) => {
    for (const attribute of [...element.attributes]) {
      const name = attribute.name.toLowerCase();
      if (
        name.startsWith('on') ||
        ['href', 'action', 'formaction', 'srcdoc', 'target'].includes(name) ||
        (name === 'src' &&
          !(element instanceof HTMLImageElement && /^(data:image\/|blob:)/i.test(attribute.value)))
      ) {
        element.removeAttribute(attribute.name);
      }
    }
  });
  document.querySelectorAll<HTMLElement>('[style]').forEach((element) => {
    for (const property of ['color', 'backgroundColor'] as const) {
      const value = element.style[property];
      const opaqueValue = value.replace(
        /^rgba\((\d+),\s*(\d+),\s*(\d+),\s*0\)$/,
        'rgb($1, $2, $3)'
      );
      if (opaqueValue !== value) element.style[property] = opaqueValue;
    }
  });
  document.querySelectorAll<HTMLObjectElement>('object[type^="image/"][data]').forEach((object) => {
    const image = document.createElement('img');
    image.src = object.data;
    image.alt = '';
    image.setAttribute('style', object.getAttribute('style') ?? '');
    object.replaceWith(image);
  });

  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content =
    "default-src 'none'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:";
  document.head.prepend(meta);

  // Literal hexes, not `var(--…)`: the CSP above is `default-src 'none'`, so
  // this document cannot load the app stylesheet and a custom property would
  // resolve to nothing. `sandboxedSurface` supplies the ACTIVE FAMILY's values
  // — these were a fixed light/dark pair that painted every family Parchment.
  const surface = sandboxedSurface(themeFamily, resolvedTheme);
  const style = document.createElement('style');
  style.textContent = `
    html, body { min-height: 100%; }
    body {
      margin: 0;
      color: ${surface.foreground};
      background: ${surface.background};
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    table { background: ${surface.card}; }
    td { min-width: 4rem; }
  `;
  document.head.append(style);
  return `<!doctype html>${document.documentElement.outerHTML}`;
}

function SpreadsheetPreview({
  file,
  resolvedTheme,
  isResizing,
}: Pick<DocumentPreviewProps, 'file' | 'resolvedTheme' | 'isResizing'>) {
  const themeFamily = useThemeFamily();
  const [sheets, setSheets] = useState<string[]>([]);
  const [activeSheet, setActiveSheet] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const blobUrls: string[] = [];
    setSheets([]);
    setActiveSheet(0);
    setError(null);

    void import('xlsx-preview')
      .then(({ xlsx2Html }) =>
        xlsx2Html(file.data.slice(0), {
          output: 'string',
          separateSheets: true,
        })
      )
      .then((result) => {
        if (!Array.isArray(result) || !result.every((sheet) => typeof sheet === 'string')) {
          throw new Error('Could not read the workbook sheets.');
        }
        const resultBlobUrls = result.flatMap((sheet) => sheet.match(/blob:[^"'\s<>]+/g) ?? []);
        if (cancelled) {
          for (const url of resultBlobUrls) URL.revokeObjectURL(url);
          return;
        }
        blobUrls.push(...resultBlobUrls);
        setSheets(result.map((sheet) => prepareSpreadsheetHtml(sheet, resolvedTheme, themeFamily)));
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : 'Could not render this workbook.');
        }
      });

    return () => {
      cancelled = true;
      for (const url of blobUrls) URL.revokeObjectURL(url);
    };
    // `themeFamily` belongs here for the same reason `resolvedTheme` does: the
    // sheet HTML is built once and cached in state, so switching family has to
    // re-render it or the workbook keeps the previous family's ground.
  }, [file.data, file.path, resolvedTheme, themeFamily]);

  if (error) return <PreviewStatus message={error} />;
  if (sheets.length === 0) return <PreviewStatus message="Rendering workbook" />;

  return (
    <div className="flex h-full min-h-0 flex-col bg-background-default">
      <iframe
        key={activeSheet}
        name="biorouter-spreadsheet-preview"
        srcDoc={sheets[activeSheet]}
        sandbox=""
        referrerPolicy="no-referrer"
        aria-label={`${file.title}, sheet ${activeSheet + 1}`}
        // The sheet document paints the family ground itself; this only shows
        // before it loads, so it must agree rather than flash white on a dark
        // theme.
        className={cn('min-h-0 flex-1 bg-background-default', isResizing && 'pointer-events-none')}
      />
      {sheets.length > 1 && (
        <div className="flex h-9 shrink-0 items-center gap-1 overflow-x-auto border-t border-border-subtle bg-background-muted px-2">
          {sheets.map((_, index) => (
            <button
              key={index}
              type="button"
              onClick={() => setActiveSheet(index)}
              aria-pressed={activeSheet === index}
              className={cn(
                // A tab in the sheet bar: `rounded-element`, and the active one
                // is a FILL, not a lift — which is why the raised card look
                // (bg-background-default + shadow-sm) becomes the selected tint.
                'h-7 shrink-0 rounded-element px-2.5 text-supporting transition-colors',
                activeSheet === index
                  ? 'bg-overlay-selected text-text-default'
                  : 'text-text-muted hover:bg-overlay-hover hover:text-text-default'
              )}
            >
              Sheet {index + 1}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * The deck renders into `containerRef` and scrolls in `scrollRef`, and the two
 * must stay separate elements.
 *
 * ⚠ `@aiden0z/pptx-renderer` sizes every slide from its container's
 * `clientWidth` under `fitMode: 'contain'` — on open, on resize and in its
 * post-render correction — and `clientWidth` INCLUDES padding. With the gutter
 * on that same element each slide came out as wide as the whole panel: it
 * started after the left gutter and ran the right one off the edge, clipped and
 * scrolling sideways, with the slide labels off centre by the same amount. So
 * the gutter lives on the scroller (`.artifact-pptx-scroll` in main.css) and
 * the measured element carries no padding at all, which makes its `clientWidth`
 * exactly the width a slide may take and lets the library's `margin: 0 auto`
 * centre it.
 */
function PowerPointPreview({ file }: Pick<DocumentPreviewProps, 'file'>) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rendered, setRendered] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    const scroller = scrollRef.current;
    if (!container || !scroller) return;

    const abortController = new AbortController();
    let viewer: { destroy: () => void } | null = null;
    setError(null);
    setRendered(false);
    container.replaceChildren();

    void import('@aiden0z/pptx-renderer')
      .then(async ({ PptxViewer, RECOMMENDED_ZIP_LIMITS }) => {
        viewer = await PptxViewer.open(file.data.slice(0), container, {
          fitMode: 'contain',
          scrollContainer: scroller,
          zipLimits: RECOMMENDED_ZIP_LIMITS,
          lazyMedia: true,
          lazySlides: true,
          pdfjs: false,
          signal: abortController.signal,
          listOptions: {
            windowed: true,
            initialSlides: 4,
            batchSize: 4,
            showSlideLabels: true,
          },
        });
        if (!abortController.signal.aborted) setRendered(true);
      })
      .catch((cause: unknown) => {
        if (!abortController.signal.aborted) {
          setError(
            cause instanceof Error ? cause.message : 'Could not render this PowerPoint deck.'
          );
        }
      });

    return () => {
      abortController.abort();
      viewer?.destroy();
      container.replaceChildren();
    };
  }, [file.data, file.path]);

  return (
    <div className="relative h-full bg-background-medium">
      <div
        ref={scrollRef}
        className="artifact-document-scroll artifact-pptx-scroll h-full overflow-auto"
      >
        <div
          ref={containerRef}
          data-rendered={rendered ? 'true' : 'false'}
          className="artifact-pptx-preview"
        />
      </div>
      {!rendered && !error && <PreviewStatus message="Rendering PowerPoint" />}
      {error && <PreviewStatus message={error} />}
    </div>
  );
}

export default function DocumentPreview({ file, resolvedTheme, isResizing }: DocumentPreviewProps) {
  const body =
    file.format === 'pdf' ? (
      <PdfPreview file={file} isResizing={isResizing} />
    ) : file.format === 'docx' ? (
      <WordPreview file={file} />
    ) : file.format === 'xlsx' ? (
      <SpreadsheetPreview file={file} resolvedTheme={resolvedTheme} isResizing={isResizing} />
    ) : (
      <PowerPointPreview file={file} />
    );

  // Every non-commercial renderer for these formats has the same blind spots.
  // Naming them in the UI is cheaper than a researcher concluding the file is
  // corrupt because a deck's animations or a document's table of contents did
  // not survive. It is one muted line, not a banner — the document is the point.
  const fidelity = DOCUMENT_FIDELITY_NOTES[file.format];

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1">{body}</div>
      {fidelity && file.format !== 'pdf' && (
        <p
          data-testid="document-fidelity-note"
          className="flex-none border-t border-border-subtle bg-background-default px-3 py-1.5 text-supporting text-text-subtle"
        >
          {fidelity}
        </p>
      )}
    </div>
  );
}
