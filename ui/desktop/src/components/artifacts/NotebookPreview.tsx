import { useMemo } from 'react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { useThemeFamily, type ThemeFamily } from '../../contexts/ThemeContext';
import { CODE_FONT_FAMILY, codeThemesByFamily } from '../../styles/codeTheme';
import { cn } from '../../utils';
import MarkdownContent from '../MarkdownContent';
import type { ArtifactFilePreview } from './artifactTypes';
import {
  normalizeCodeLanguage,
  sandboxedSurface,
  splitPathForStrip,
  STRIP_IDENT_CLASS,
  STRIP_LABEL_CLASS,
  STRIP_META_CLASS,
} from './artifactUtils';

type NotebookFile = Extract<ArtifactFilePreview, { kind: 'text' | 'html' }>;

/**
 * The renderer's `::selection` tint (main.css: `--selection-hue` at
 * `--selection-alpha`), restated for the sandboxed output document, which
 * cannot load the stylesheet. #cf6d47 / #e8895f at 24% in both modes.
 * Family-invariant on purpose: Alma Mater re-points the coral scale to teal,
 * and selection is Biorouter orange in every family.
 */
const SELECTION_TINT = {
  light: 'rgba(207,109,71,0.24)',
  dark: 'rgba(232,137,95,0.24)',
} as const;

type NotebookOutput = {
  output_type?: string;
  text?: string | string[];
  data?: Record<string, unknown>;
  ename?: string;
  evalue?: string;
  traceback?: string[];
};

type NotebookCell = {
  cell_type?: string;
  source?: string | string[];
  execution_count?: number | null;
  outputs?: NotebookOutput[];
};

type Notebook = {
  cells: NotebookCell[];
  metadata?: {
    kernelspec?: { display_name?: string; language?: string };
    language_info?: { name?: string };
  };
};

function joined(value: unknown) {
  if (typeof value === 'string') return value;
  if (Array.isArray(value) && value.every((item) => typeof item === 'string')) {
    return value.join('');
  }
  return '';
}

/**
 * Wrap a notebook's `text/html` output in a locked-down document.
 *
 * The CSP is deliberately the strictest thing that can still show a rendered
 * table: no scripts, no network, inline styles only, and images restricted to
 * `data:`/`blob:`. That is also why the colours are literal hexes rather than
 * `var(--text-default)` — under `default-src 'none'` this document cannot reach
 * the app stylesheet, so a custom property would resolve to nothing. They come
 * from the ACTIVE FAMILY (see `sandboxedSurface`), not a fixed light/dark pair.
 */
function safeNotebookHtml(html: string, resolvedTheme: 'light' | 'dark', themeFamily: ThemeFamily) {
  const { background, foreground, border } = sandboxedSurface(themeFamily, resolvedTheme);
  // Paper, restated for a separate document: the page font stack (--font-body)
  // and a booktabs table — `border:0` on the table too, because pandas emits
  // `<table border="1">` and the attribute alone drew a full grid. `::selection`
  // is restated for the same reason: the renderer-wide Biorouter-orange
  // selection in main.css cannot reach into this document (SELECTION_TINT).
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:"><style>html,body{margin:0;color:${foreground};background:${background};font:13px/1.5 Arial,"Helvetica Neue",Helvetica,ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}body{padding:12px 0}img,svg{max-width:100%;height:auto}table{border:0;border-collapse:collapse;font-variant-numeric:tabular-nums}td,th{border:0;border-bottom:1px solid ${border};padding:5px 16px 5px 0;text-align:right}th{font-weight:600}::selection{background:${SELECTION_TINT[resolvedTheme]}}</style></head><body>${html}</body></html>`;
}

function NotebookOutputView({
  output,
  resolvedTheme,
  themeFamily,
}: {
  output: NotebookOutput;
  resolvedTheme: 'light' | 'dark';
  themeFamily: ThemeFamily;
}) {
  if (output.output_type === 'error') {
    const traceback =
      output.traceback?.join('\n') || [output.ename, output.evalue].filter(Boolean).join(': ');
    return (
      <pre className="br-paper-nb-error overflow-auto whitespace-pre-wrap font-mono text-code">
        {traceback || 'Notebook execution failed.'}
      </pre>
    );
  }

  const stream = joined(output.text);
  if (stream) {
    return <pre className="br-paper-nb-stream overflow-auto font-mono text-code">{stream}</pre>;
  }

  const data = output.data ?? {};
  const png = joined(data['image/png']);
  if (png) {
    return (
      <div className="br-paper-nb-figure overflow-auto">
        <img src={`data:image/png;base64,${png.replace(/\s/g, '')}`} alt="Notebook output" />
      </div>
    );
  }

  const jpeg = joined(data['image/jpeg']);
  if (jpeg) {
    return (
      <div className="br-paper-nb-figure overflow-auto">
        <img src={`data:image/jpeg;base64,${jpeg.replace(/\s/g, '')}`} alt="Notebook output" />
      </div>
    );
  }

  const svg = joined(data['image/svg+xml']);
  if (svg) {
    return (
      <div className="br-paper-nb-figure overflow-auto">
        <img
          src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`}
          alt="Notebook output"
        />
      </div>
    );
  }

  const html = joined(data['text/html']);
  if (html) {
    return (
      <iframe
        title="HTML notebook output"
        srcDoc={safeNotebookHtml(html, resolvedTheme, themeFamily)}
        sandbox=""
        // The frame's own document paints the family ground; this only covers
        // the moment before it loads, so it must agree — `bg-white` flashed
        // white over every dark theme.
        className="min-h-40 w-full bg-background-default"
      />
    );
  }

  const markdown = joined(data['text/markdown']);
  if (markdown) {
    return <MarkdownContent content={markdown} className="br-paper-prose" variant="document" />;
  }

  const plain = joined(data['text/plain']);
  if (plain) {
    return <pre className="br-paper-nb-stream overflow-auto font-mono text-code">{plain}</pre>;
  }

  const json = data['application/json'];
  if (json !== undefined) {
    // Structured data is code-like, so it is highlighted like the cell above it
    // — on the paper, not in a well: an output is the notebook's answer, and
    // the well is reserved for the source that produced it.
    return (
      <SyntaxHighlighter
        style={codeThemesByFamily[themeFamily][resolvedTheme]}
        language="json"
        PreTag="div"
        customStyle={{ margin: 0, padding: '2px 16px', background: 'transparent' }}
        codeTagProps={{ style: { fontFamily: CODE_FONT_FAMILY, whiteSpace: 'pre-wrap' } }}
      >
        {typeof json === 'string' ? json : JSON.stringify(json, null, 2)}
      </SyntaxHighlighter>
    );
  }

  return null;
}

export default function NotebookPreview({
  file,
  resolvedTheme,
}: {
  file: NotebookFile;
  resolvedTheme: 'light' | 'dark';
}) {
  const themeFamily = useThemeFamily();
  const parsed = useMemo(() => {
    try {
      const value = JSON.parse(file.text) as Partial<Notebook>;
      if (!Array.isArray(value.cells)) throw new Error('Notebook cells are missing.');
      return { notebook: value as Notebook, error: null };
    } catch (cause) {
      return {
        notebook: null,
        error: cause instanceof Error ? cause.message : 'Could not read this notebook.',
      };
    }
  }, [file.text]);

  if (!parsed.notebook) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-body text-text-muted">
        Could not preview this notebook: {parsed.error}
      </div>
    );
  }

  const notebook = parsed.notebook;
  // Normalised: an IRkernel notebook declares `R`, and Prism's registry is
  // case-sensitive, so its cells used to render as unhighlighted text.
  const language = normalizeCodeLanguage(
    notebook.metadata?.language_info?.name || notebook.metadata?.kernelspec?.language || 'python'
  );
  const kernel = notebook.metadata?.kernelspec?.display_name;
  const { directory, name } = splitPathForStrip(file.path);
  const cellCount = notebook.cells.length;

  return (
    // Paper, like every other text preview (see TextFilePreview): no cell cards.
    // A cell is vertical rhythm on the page; a code cell's source sits in the
    // same faint well a fenced block uses, its prompt hanging in the margin.
    <div className="br-paper flex h-full min-h-0 flex-col">
      {/* The one status strip (design spec H): a 34px row with a bottom
          hairline — the same strip every other file preview uses, so the
          notebook reads in the panel's voice instead of its own. */}
      <div
        data-testid="artifact-status-strip"
        className="flex h-[34px] flex-shrink-0 items-center gap-2.5 border-b border-border-subtle px-3.5"
      >
        <span className={cn(STRIP_LABEL_CLASS, 'shrink-0')}>Notebook</span>
        <span className={cn(STRIP_IDENT_CLASS, 'min-w-0 truncate')} title={file.path}>
          <span className="text-text-subtle">{directory}</span>
          <span className="text-text-default">{name}</span>
        </span>
        <span className={cn(STRIP_IDENT_CLASS, 'shrink-0 tabular-nums')}>
          {cellCount.toLocaleString()} cell{cellCount === 1 ? '' : 's'}
        </span>
        {kernel && (
          <span className={cn(STRIP_META_CLASS, 'min-w-0 shrink truncate')} title={kernel}>
            {kernel}
          </span>
        )}
      </div>
      <div className="br-paper-scroll min-h-0 flex-1 overflow-auto">
        {/* PROVISIONAL `br-paper-measure` — see MarkdownDocument.tsx. */}
        <div className="br-paper-measure br-paper-nb">
          {notebook.cells.map((cell, index) => {
            const source = joined(cell.source);
            const executionLabel = cell.execution_count ?? ' ';
            if (cell.cell_type === 'markdown') {
              return (
                <section
                  key={index}
                  aria-label={`Markdown cell ${index + 1}`}
                  className="br-paper-nb-cell"
                  data-kind="markdown"
                >
                  <MarkdownContent content={source} className="br-paper-prose" variant="document" />
                </section>
              );
            }

            if (cell.cell_type === 'code') {
              return (
                <section
                  key={index}
                  aria-label={`Code cell ${index + 1}`}
                  className="br-paper-nb-cell"
                  data-kind="code"
                >
                  {/* The execution prompt: mono metadata, `tabular-nums` so the
                      counter does not shift as it widens. */}
                  <div className="br-paper-nb-prompt" aria-hidden="true">
                    [{executionLabel}]
                  </div>
                  <div className="br-paper-well br-paper-nb-source">
                    <SyntaxHighlighter
                      style={codeThemesByFamily[themeFamily][resolvedTheme]}
                      language={language}
                      PreTag="div"
                      customStyle={{ margin: 0, padding: '12px 16px', background: 'transparent' }}
                      codeTagProps={{
                        style: { fontFamily: CODE_FONT_FAMILY, whiteSpace: 'pre' },
                      }}
                    >
                      {source.replace(/\r?\n$/, '')}
                    </SyntaxHighlighter>
                  </div>
                  {(cell.outputs?.length ?? 0) > 0 && (
                    <div className="br-paper-nb-outputs">
                      {cell.outputs?.map((output, outputIndex) => (
                        <div key={outputIndex} className="br-paper-nb-output">
                          <NotebookOutputView
                            output={output}
                            resolvedTheme={resolvedTheme}
                            themeFamily={themeFamily}
                          />
                        </div>
                      ))}
                    </div>
                  )}
                </section>
              );
            }

            return (
              <section
                key={index}
                aria-label={`Raw cell ${index + 1}`}
                className="br-paper-nb-cell"
                data-kind="raw"
              >
                <pre className="br-paper-nb-stream overflow-auto font-mono text-code">{source}</pre>
              </section>
            );
          })}
        </div>
      </div>
    </div>
  );
}
