import { readFileSync } from 'node:fs';
import { render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from '../../styles/themes.generated';
import NotebookPreview from './NotebookPreview';
import type { ArtifactFilePreview } from './artifactTypes';
import { PREVIEW_COPY } from './copy';
import { PreviewBar, PreviewBarScope } from './PreviewBar';

function notebookFile(text: string) {
  return {
    kind: 'text',
    title: 'analysis.ipynb',
    path: '/work/analysis.ipynb',
    mimeType: 'application/x-ipynb+json',
    text,
    size: text.length,
    found: true,
  } satisfies Extract<ArtifactFilePreview, { kind: 'text' | 'html' }>;
}

describe('NotebookPreview', () => {
  it('renders markdown, code, execution output, images, and sandboxed HTML', () => {
    const text = JSON.stringify({
      metadata: { kernelspec: { display_name: 'Python 3', language: 'python' } },
      cells: [
        { cell_type: 'markdown', source: ['# Expression analysis\n', 'A notebook summary.'] },
        {
          cell_type: 'code',
          execution_count: 4,
          source: ['print("ready")\n'],
          outputs: [
            { output_type: 'stream', text: ['ready\n'] },
            { output_type: 'display_data', data: { 'image/png': 'aGVsbG8=' } },
            { output_type: 'display_data', data: { 'text/html': '<strong>HTML result</strong>' } },
          ],
        },
      ],
    });

    render(
      <ThemeProvider>
        {/* The panel's one bar names the file; the notebook adds to it. */}
        <PreviewBarScope>
          <PreviewBar name="analysis.ipynb" path="/work/analysis.ipynb" />
          <NotebookPreview file={notebookFile(text)} resolvedTheme="light" />
        </PreviewBarScope>
      </ThemeProvider>
    );

    // The bar: the name, the cell count, and the kernel as the count's help
    // (spec 3.8) rather than a second label.
    const bar = screen.getByTestId('artifact-status-strip');
    expect(within(bar).getByText('analysis.ipynb')).toBeInTheDocument();
    expect(within(bar).getByText('2 cells')).toBeInTheDocument();
    expect(
      within(bar).getByRole('button', { name: `About ${PREVIEW_COPY.kernel}` })
    ).toHaveAccessibleDescription(PREVIEW_COPY.kernelHelp('Python 3'));
    expect(screen.getByRole('heading', { name: 'Expression analysis' })).toBeInTheDocument();
    expect(screen.getByText('ready')).toBeInTheDocument();
    expect(screen.getByAltText('Notebook output')).toHaveAttribute(
      'src',
      'data:image/png;base64,aGVsbG8='
    );
    const htmlOutput = screen.getByTitle('HTML notebook output');
    expect(htmlOutput).toHaveAttribute('sandbox', '');
    expect(htmlOutput.getAttribute('srcdoc')).toContain('<strong>HTML result</strong>');
    expect(htmlOutput.getAttribute('srcdoc')).toContain("default-src 'none'");
  });

  // IRkernel declares its language as `R`; Prism's registry is case-sensitive,
  // so these cells used to render as plain text.
  it('highlights an R-kernel notebook', () => {
    const { container } = render(
      <ThemeProvider>
        <NotebookPreview
          file={notebookFile(
            JSON.stringify({
              metadata: { kernelspec: { language: 'R' }, language_info: { name: 'R' } },
              cells: [{ cell_type: 'code', source: ['x <- TRUE\n', 'y <- 15'], outputs: [] }],
            })
          )}
          resolvedTheme="light"
        />
      </ThemeProvider>
    );
    // Plain text renders no token spans; the R grammar does (`TRUE`, `15`).
    expect(container.querySelector('section[aria-label="Code cell 1"] code .token')).not.toBeNull();
    // No cell cards: the source sits in the paper well, the prompt in the margin.
    expect(
      container.querySelector('section[aria-label="Code cell 1"] .br-paper-well')
    ).not.toBeNull();
  });

  it('restates the light selection tint inside the sandboxed output', () => {
    render(
      <ThemeProvider>
        <NotebookPreview
          file={notebookFile(
            JSON.stringify({
              cells: [
                {
                  cell_type: 'code',
                  source: ['df'],
                  outputs: [
                    {
                      output_type: 'execute_result',
                      data: { 'text/html': '<table border="1"></table>' },
                    },
                  ],
                },
              ],
            })
          )}
          resolvedTheme="light"
        />
      </ThemeProvider>
    );
    const srcdoc = screen.getByTitle('HTML notebook output').getAttribute('srcdoc') ?? '';
    expect(srcdoc).toContain('::selection{background:rgba(207,109,71,0.24)}');
  });

  it('shows a readable error for malformed notebook JSON', () => {
    render(
      <ThemeProvider>
        <NotebookPreview file={notebookFile('{broken')} resolvedTheme="light" />
      </ThemeProvider>
    );

    expect(screen.getByText(/could not preview this notebook/i)).toBeInTheDocument();
  });
});

describe('NotebookPreview HTML output theming', () => {
  afterEach(() => localStorage.clear());

  const htmlOutputNotebook = JSON.stringify({
    cells: [
      {
        cell_type: 'code',
        source: ['df.head()\n'],
        outputs: [{ output_type: 'display_data', data: { 'text/html': '<table></table>' } }],
      },
    ],
  });

  function srcdocFor(family: string, resolvedTheme: 'light' | 'dark') {
    localStorage.setItem('theme_family', family);
    const view = render(
      <ThemeProvider>
        <NotebookPreview file={notebookFile(htmlOutputNotebook)} resolvedTheme={resolvedTheme} />
      </ThemeProvider>
    );
    const srcdoc = screen.getByTitle('HTML notebook output').getAttribute('srcdoc') ?? '';
    view.unmount();
    return srcdoc;
  }

  // A notebook's `text/html` output is sandboxed under `default-src 'none'`, so
  // it cannot read the app stylesheet and has to be handed literal hexes. Those
  // hexes used to be one hardcoded light/dark pair, which painted every family
  // Parchment. Assert they come from the ACTIVE family's generated tokens.
  it.each(THEME_FAMILY_IDS)('paints the %s ground and ink in both modes', (family) => {
    for (const mode of ['light', 'dark'] as const) {
      const { background, foreground, border } = GENERATED_THEMES[family][mode].surface;
      const srcdoc = srcdocFor(family, mode);
      expect(srcdoc).toContain(`background:${background}`);
      expect(srcdoc).toContain(`color:${foreground}`);
      expect(srcdoc).toContain(`1px solid ${border}`);
    }
  });

  // The regression guard with teeth: the three families must not agree. If a
  // future edit re-hardcodes a colour, every family renders identically and the
  // per-family assertions above would still pass for whichever family that
  // hardcode happened to match.
  //
  // It used to be asserted on the GROUND. That no longer works and shouldn't:
  // neutrals are shared infrastructure, so all three dark previews paint the
  // same #1b1b19 on purpose. The probe is the INK instead — the axis a family
  // still owns — plus the whole rendered srcdoc, which differs only because of
  // it. A re-hardcoded preview fails both.
  it('gives each family a distinct dark ink, on the one shared ground', () => {
    const inks = THEME_FAMILY_IDS.map((family) => GENERATED_THEMES[family].dark.surface.foreground);
    expect(new Set(inks).size).toBe(THEME_FAMILY_IDS.length);

    const grounds = THEME_FAMILY_IDS.map(
      (family) => GENERATED_THEMES[family].dark.surface.background
    );
    expect(new Set(grounds).size, 'neutrals are shared; a per-family ground is the bug').toBe(1);

    const rendered = THEME_FAMILY_IDS.map((family) => srcdocFor(family, 'dark'));
    expect(new Set(rendered).size).toBe(THEME_FAMILY_IDS.length);
  });

  it('keeps the strict sandbox and CSP while theming', () => {
    const frame = (() => {
      localStorage.setItem('theme_family', 'roche-limit');
      render(
        <ThemeProvider>
          <NotebookPreview file={notebookFile(htmlOutputNotebook)} resolvedTheme="dark" />
        </ThemeProvider>
      );
      return screen.getByTitle('HTML notebook output');
    })();

    expect(frame).toHaveAttribute('sandbox', '');
    const srcdoc = frame.getAttribute('srcdoc') ?? '';
    expect(srcdoc).toContain("default-src 'none'");
    expect(srcdoc).toContain("style-src 'unsafe-inline'");
    expect(srcdoc).not.toContain('script-src');
    // A separate document: the renderer's orange selection and the app's one
    // table (spec section 0, filled under R-27: a subtle fill with a transparent
    // border, top rules only between rows) are restated inside it, since it
    // cannot load main.css. pandas writes `<table border="1">`: the table's own
    // border replaces it, and every cell is cleared before its top rule. The
    // dark tint is the renderer's: #e8895f at 24%.
    expect(srcdoc).toContain('::selection{background:rgba(232,137,95,0.24)}');
    expect(srcdoc).toContain('table{border:1px solid transparent;background:');
    expect(srcdoc).toContain('td,th{border:0;border-top:1px solid');
    expect(srcdoc).toContain('font-weight:400');
  });

  // R-27 (fills before lines): the DataFrame is a filled box, not a framed one,
  // exactly as `.br-md-table` is in the app. The sandbox cannot read the token,
  // so the fill is a 4% wash of the ink over the page; this measures that wash
  // against the real `--background-fill` in main.css for every family and mode,
  // so the two cannot drift apart.
  it.each(THEME_FAMILY_IDS)('draws the table on the fill step, not a frame, in %s', (family) => {
    const css = readFileSync('src/styles/main.css', 'utf-8');
    const token = (scope: ':root' | '.dark') => {
      const block = new RegExp(`(?:^|\\n)${scope.replace('.', '\\.')} \\{[\\s\\S]*?\\n\\}`).exec(
        css
      );
      const hex = /--background-fill:\s*(#[0-9a-f]{6})/i.exec(block?.[0] ?? '');
      expect(hex, `--background-fill under ${scope}`).not.toBeNull();
      return hex![1];
    };
    const channels = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
    const luminance = (hex: string) => {
      const [r, g, b] = channels(hex)
        .map((v) => v / 255)
        .map((v) => (v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4)));
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    for (const mode of ['light', 'dark'] as const) {
      const { background, foreground } = GENERATED_THEMES[family][mode].surface;
      const srcdoc = srcdocFor(family, mode);
      expect(srcdoc).toContain(
        `background:color-mix(in srgb, ${foreground} 4%, ${background});background-clip:padding-box`
      );
      // Measured: the wash lands within 3% (contrast) of the fill token.
      const [f, g] = [channels(foreground), channels(background)];
      const wash = `#${f
        .map((v, i) => Math.round(v * 0.04 + g[i] * 0.96))
        .map((v) => v.toString(16).padStart(2, '0'))
        .join('')}`;
      const fill = token(mode === 'light' ? ':root' : '.dark');
      const [a, b] = [luminance(wash), luminance(fill)];
      expect((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05), `${family}.${mode}`).toBeLessThan(
        1.03
      );
      // No second ground on the header, and the edge survives forced colours.
      expect(srcdoc).toContain('thead th{color:');
      expect(srcdoc).not.toMatch(/thead th\{background/);
      expect(srcdoc).toContain('@media (forced-colors:active){table{border-color:CanvasText}}');
    }
  });
});
