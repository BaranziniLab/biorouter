import { useMemo } from 'react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { useThemeFamily, type ThemeFamily } from '../../contexts/ThemeContext';
import { CODE_FONT_FAMILY, codeThemesByFamily } from '../../styles/codeTheme';
import MarkdownContent from '../MarkdownContent';
import { InfoTip } from '../ui/info-tip';
import type { ArtifactFilePreview } from './artifactTypes';
import { normalizeCodeLanguage, sandboxedSurface } from './artifactUtils';
import { PREVIEW_COPY } from './copy';
import { PreviewBarMeta } from './PreviewBar';

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
  const { background, foreground, border, muted } = sandboxedSurface(themeFamily, resolvedTheme);
  // `--background-muted` is not in the generated surface, so the header ground is
  // mixed the way the token is: a 4% wash of the ink over the page.
  const mutedGround = `color-mix(in srgb, ${foreground} 4%, ${background})`;
  // Paper, restated for a separate document: the page font stack (--font-body)
  // and the app's ONE table (`.br-md-table`, Crew's recipe; spec section 0): a
  // 1px frame at radius 8, top rules only, a 12px muted header on the muted
  // ground, 8px 12px cells, tabular numbers, no bold index. pandas emits
  // `<table border="1">`, whose attribute alone drew a full grid: the table's
  // own `border` replaces it with the frame, and `border:0` on every cell clears
  // the grid before the top rule is drawn. `::selection` is restated because the
  // renderer-wide selection in main.css cannot reach this document.
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:"><style>html,body{margin:0;color:${foreground};background:${background};font:13px/18px Arial,"Helvetica Neue",Helvetica,ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}body{padding:8px 1px}img,svg{max-width:100%;height:auto}table{border:1px solid ${border};border-collapse:separate;border-spacing:0;border-radius:8px;overflow:hidden;font-variant-numeric:tabular-nums}td,th{border:0;border-top:1px solid ${border};padding:8px 12px;text-align:right;font-weight:400}thead tr:first-child>*,table>tbody:first-child>tr:first-child>*{border-top:0}thead th{background:${mutedGround};color:${muted};font-size:12px;line-height:16px;font-weight:600}tbody th{color:${muted}}::selection{background:${SELECTION_TINT[resolvedTheme]}}</style></head><body>${html}</body></html>`;
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
      <div className="br-preview-state">
        <p className="br-preview-state__line text-body text-text-muted">
          {PREVIEW_COPY.notebookUnreadable(parsed.error ?? '')}
        </p>
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
  const cellCount = notebook.cells.length;

  return (
    // Paper, like every other text preview (see TextFilePreview): no cell cards.
    // A cell is vertical rhythm on the page; a code cell's source sits in the
    // same faint well a fenced block uses, its prompt hanging in the margin.
    <div className="br-paper flex h-full min-h-0 flex-col">
      {/* The panel's one bar names the file; the notebook adds its cell count,
          and the kernel is that count's help rather than a second label. */}
      <PreviewBarMeta>
        <span data-testid="artifact-strip-count" className="tabular-nums">
          {cellCount.toLocaleString()} cell{cellCount === 1 ? '' : 's'}
        </span>
        {kernel && <InfoTip label={PREVIEW_COPY.kernel} help={PREVIEW_COPY.kernelHelp(kernel)} />}
      </PreviewBarMeta>
      <div className="br-paper-scroll min-h-0 flex-1 overflow-auto">
        <div className="br-preview-measure br-paper-nb">
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
