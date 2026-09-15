import type { EmbeddedResource, RawResource, ResourceContents } from '../../api';
import type { ThemeFamily } from '../../contexts/ThemeContext';
import { GENERATED_THEMES } from '../../styles/themes.generated';
import { IMAGE_EXTENSIONS } from '../../utils/imageFormats';
import { sanitizeArtifactTitle } from '../../utils/untrustedText';
import type { ArtifactSource } from './artifactTypes';
import { localFileBasename, parseFileLink, resolveLocalFilePath } from './artifactFileLinks';

const TEXT_EXTENSIONS = new Set([
  'bash',
  'c',
  'cc',
  'conf',
  'cpp',
  'cs',
  'css',
  'csv',
  'go',
  'h',
  'hpp',
  'html',
  'java',
  'js',
  'json',
  'jsx',
  'log',
  'md',
  'py',
  'qmd',
  'r',
  'rmd',
  'rs',
  'sh',
  'sql',
  'toml',
  'ts',
  'tsv',
  'tsx',
  'txt',
  'xml',
  'yaml',
  'yml',
]);

const HTML_EXTENSIONS = new Set(['htm', 'html']);
const DOCUMENT_EXTENSIONS = new Set(['docx', 'ipynb', 'pdf', 'pptx', 'xlsx']);
const MAX_BROWSER_URL_BYTES = 8 * 1024;
const GENERIC_UI_TITLE_PARTS = new Set([
  'chart',
  'diagram',
  'graph',
  'interactive',
  'preview',
  'visualization',
]);

export function basenameFromPath(value: string): string {
  const isUri = /^[a-z][a-z\d+.-]*:\/\//i.test(value);
  const clean = isUri ? value.split(/[?#]/)[0] : value;
  const parts = clean.split(/[\\/]/).filter(Boolean);
  const base = parts[parts.length - 1] || clean || 'Artifact';
  // Filesystem paths are already decoded; decoding again changes literal `%` names.
  try {
    return isUri ? decodeURIComponent(base) : base;
  } catch {
    return base;
  }
}

export function extensionFromPath(value: string): string {
  const name = basenameFromPath(value);
  const dot = name.lastIndexOf('.');
  return dot > -1 ? name.slice(dot + 1).toLowerCase() : '';
}

// Extension -> Prism language. Anything missing falls through to the extension
// itself (Prism knows `r`, `sql`, `go`, `json`, …), then to plain text.
//
// The bioinformatics rows are the ones an agent in this app actually writes and
// that the extension alone got wrong: a Nextflow `.nf` reached Prism as `nf`
// (unregistered, so plain) and a `.log` was forced to `text` although refractor
// ships a `log` grammar. CSV and TSV map to the structured raw-view grammars in
// styles/prismGrammars.ts; Prism's own `csv` has two token kinds and there is
// no `tsv` at all.
const PRISM_LANGUAGES: Record<string, string> = {
  bash: 'bash',
  bat: 'batch',
  cc: 'cpp',
  cfg: 'ini',
  cjs: 'javascript',
  cs: 'csharp',
  conf: 'ini',
  csv: 'csv-table',
  cts: 'typescript',
  cwl: 'yaml',
  env: 'bash',
  h: 'c',
  hpp: 'cpp',
  htm: 'html',
  jl: 'julia',
  js: 'javascript',
  jsonc: 'json',
  jsonl: 'json',
  jsx: 'jsx',
  kt: 'kotlin',
  log: 'log',
  markdown: 'markdown',
  md: 'markdown',
  mjs: 'javascript',
  mk: 'makefile',
  mts: 'typescript',
  // Nextflow is a Groovy DSL; Prism has no grammar of its own for it.
  nf: 'groovy',
  pl: 'perl',
  pm: 'perl',
  ps1: 'powershell',
  py: 'python',
  // R Markdown / Quarto are markdown with fenced R chunks.
  qmd: 'markdown',
  rb: 'ruby',
  rmd: 'markdown',
  rs: 'rust',
  sh: 'bash',
  // Snakemake rules are Python with a few keywords on top.
  smk: 'python',
  svg: 'xml',
  tex: 'latex',
  toml: 'toml',
  ts: 'typescript',
  tsv: 'tsv-table',
  tsx: 'tsx',
  txt: 'text',
  yml: 'yaml',
  zsh: 'bash',
};

// Files named by convention rather than by extension (case-insensitive). Checked
// before the extension, because these names have none to go on.
const PRISM_BASENAMES: Record<string, string> = {
  dockerfile: 'docker',
  gnumakefile: 'makefile',
  justfile: 'makefile',
  makefile: 'makefile',
  snakefile: 'python',
};

/**
 * The code view's line-number gutter width, at the 13px code size. Mirrored by
 * `--paper-code-gutter: calc(3.5 * 13px)` in main.css, which hangs the gutter
 * exactly this far left of the paper column's edge so code text lands on it.
 */
export const PAPER_GUTTER_EM = '3.5em';

/**
 * The Prism language for a file: its conventional basename, then its extension
 * (mapped, then raw — Prism's own registry knows `r`, `sql`, `go`, …), then its
 * MIME type, then plain text.
 */
export function languageFromPath(value: string, mimeType?: string): string {
  const ext = extensionFromPath(value);
  const byName = PRISM_BASENAMES[basenameFromPath(value).toLowerCase()];
  if (byName) return byName;
  const mapped = PRISM_LANGUAGES[ext];
  if (mapped) return mapped;
  if (ext) return ext;
  if (mimeType?.includes('json')) return 'json';
  if (mimeType?.includes('html')) return 'html';
  if (mimeType?.includes('xml')) return 'xml';
  return 'text';
}

// A timestamp or a level word at the start of a line — the two shapes every
// pipeline runner, scheduler and aligner in this domain prints.
const LOG_LINE_RE =
  /^\s*(?:\[?\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}|\[?\d{2}:\d{2}:\d{2}|\[?(?:TRACE|DEBUG|INFO|NOTICE|WARN(?:ING)?|ERROR|FATAL|CRITICAL)\b)/;

/**
 * True when a plain-text file is really a log: at least HALF of its first 40
 * non-empty lines open with a timestamp or a level, and at least three do.
 * Agents write run output to `.txt` as often as to `.log`, and a log reads far
 * better with its levels and dates picked out. Prose `.txt` stays plain — the
 * `log` grammar colours stray numbers and quotes, which is noise in a paragraph
 * — and a third was too low a bar: notes that quote a few log lines crossed it.
 */
export function looksLikeLog(text: string): boolean {
  const lines = text
    .split(/\r?\n/, 400)
    .filter((line) => line.trim() !== '')
    .slice(0, 40);
  const hits = lines.filter((line) => LOG_LINE_RE.test(line)).length;
  return hits >= 3 && hits / lines.length >= 1 / 2;
}

/** The Prism language for a text file, using its CONTENT where the name is not enough. */
export function languageForText(path: string, mimeType: string | undefined, text: string): string {
  const language = languageFromPath(path, mimeType);
  return language === 'text' && looksLikeLog(text) ? 'log' : language;
}

// Fence / kernel names that are not Prism names. Prism's own aliases already
// cover `py`, `sh`, `shell`, `yml`, `js`; this is only what they miss.
const FENCE_LANGUAGE_ALIASES: Record<string, string> = {
  console: 'shell-session',
  ipython: 'python',
  ipython3: 'python',
  ir: 'r',
  jl: 'julia',
  nextflow: 'groovy',
  nf: 'groovy',
  plain: 'text',
  plaintext: 'text',
  python3: 'python',
  rscript: 'r',
  snakemake: 'python',
  txt: 'text',
  zsh: 'bash',
};

/**
 * Normalise a fenced-block info string or a notebook kernel language to a Prism
 * language name.
 *
 * Two real misses this closes: R Markdown / Quarto chunks are written
 * ```` ```{r setup, include=FALSE} ````, which reaches the renderer as
 * `language-{r` and never matched `/language-(\w+)/`; and an IRkernel notebook
 * declares its language as `R`, and Prism's registry is case-sensitive.
 */
export function normalizeCodeLanguage(raw: string | null | undefined): string {
  const name = (raw ?? '')
    .trim()
    .replace(/^\{/, '')
    .split(/[\s,}]/)[0]
    .toLowerCase();
  if (!name) return 'text';
  return FENCE_LANGUAGE_ALIASES[name] ?? name;
}

/**
 * Split a leading YAML front-matter block off a markdown document.
 *
 * R Markdown, Quarto and most static-site markdown open with one. Unsplit, the
 * `---` fences read as a setext heading rule and every `key: value` line became
 * a bold heading.
 */
export function splitFrontMatter(text: string): { frontMatter: string | null; body: string } {
  const match = /^\uFEFF?---[ \t]*\r?\n([\s\S]*?)\r?\n(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/.exec(text);
  if (!match) return { frontMatter: null, body: text };
  return { frontMatter: match[1], body: text.slice(match[0].length) };
}

/**
 * The document-header fields of a front-matter block, and whatever is left.
 *
 * Only top-level SCALAR `title` / `subtitle` / `author` / `date` are lifted; a
 * list-valued author or anything nested stays in `rest`, verbatim, so nothing
 * the file says is ever dropped from the preview.
 */
export function frontMatterFields(yaml: string): {
  title?: string;
  subtitle?: string;
  byline: string[];
  rest: string;
} {
  const fields: Record<string, string> = {};
  const rest: string[] = [];
  const lines = yaml.split(/\r?\n/);
  for (const [index, line] of lines.entries()) {
    const match = /^(title|subtitle|author|date):[ \t]*(\S.*?)[ \t]*$/.exec(line);
    // Keep structured YAML intact in the disclosure instead of separating its
    // key from a block value or lifting a collection as a literal heading.
    const structured =
      match && (/^[>|[\]{}&*!#]/.test(match[2]) || /^\s+\S/.test(lines[index + 1] ?? ''));
    if (match && !structured && !fields[match[1]]) {
      fields[match[1]] = match[2].replace(/^(['"])(.*)\1$/, '$2');
    } else {
      rest.push(line);
    }
  }
  return {
    title: fields.title,
    subtitle: fields.subtitle,
    byline: [fields.author, fields.date].filter((value): value is string => Boolean(value)),
    rest: rest.join('\n').trim(),
  };
}

const MISSING_CELL_VALUES = new Set(['', 'na', 'nan', 'null', 'none', 'n/a', '#n/a']);
const NUMERIC_CELL_RE = /^[-+\u2212]?(?:\d{1,3}(?:,\d{3})+|\d+)?(?:\.\d+)?(?:[eE][-+]?\d+)?%?$/;

/** An empty cell or one of the spellings R, pandas and Excel write for "no value". */
export function isMissingCell(value: string): boolean {
  return MISSING_CELL_VALUES.has(value.trim().toLowerCase());
}

/**
 * Per-column reading hints for a delimited table.
 *
 * `numeric`: at least 90% of the present values parse as a number (thousands
 * separators, exponents and percentages included), so the column is set
 * right-aligned in tabular figures and its digits line up. `prose`: the mean
 * value is longer than 32 characters, so the column reads as sentences and is
 * clipped to 44ch with an ellipsis (the full value is the cell's title). No
 * cell ever wraps: a wrapped cell off to the right set the height of the whole
 * row, and an identifier or an exponent must never break mid-token.
 */
export function analyzeDelimitedColumns(
  header: string[],
  rows: string[][]
): { numeric: boolean; prose: boolean }[] {
  return header.map((_, index) => {
    let present = 0;
    let numeric = 0;
    let length = 0;
    for (const row of rows) {
      const value = (row[index] ?? '').trim();
      if (isMissingCell(value)) continue;
      present++;
      length += value.length;
      if (/\d/.test(value) && NUMERIC_CELL_RE.test(value)) numeric++;
    }
    return {
      numeric: present > 0 && numeric / present >= 0.9,
      prose: present > 0 && length / present > 32,
    };
  });
}

// Names that title-casing gets wrong: acronyms ("Csv") and camel-cased brands
// ("Typescript"). Keyed by the Prism language, or by extension where it differs.
const LANGUAGE_LABELS: Record<string, string> = {
  bash: 'Shell',
  batch: 'Batch',
  cpp: 'C++',
  csharp: 'C#',
  css: 'CSS',
  csv: 'CSV',
  'csv-table': 'CSV',
  docker: 'Dockerfile',
  html: 'HTML',
  ini: 'INI',
  javascript: 'JavaScript',
  json: 'JSON',
  jsx: 'JSX',
  latex: 'LaTeX',
  log: 'Log',
  makefile: 'Makefile',
  markdown: 'Markdown',
  matlab: 'MATLAB',
  powershell: 'PowerShell',
  sas: 'SAS',
  sql: 'SQL',
  text: 'Text',
  toml: 'TOML',
  tsv: 'TSV',
  'tsv-table': 'TSV',
  tsx: 'TSX',
  typescript: 'TypeScript',
  xml: 'XML',
  yaml: 'YAML',
};

// Human label for the language chip above a code preview.
export function languageLabel(value: string, mimeType?: string): string {
  const ext = extensionFromPath(value);
  if (ext === 'r') return 'R';
  if (ext === 'rmd') return 'R Markdown';
  if (ext === 'qmd') return 'Quarto';
  if (ext === 'nf') return 'Nextflow';
  if (ext === 'smk' || basenameFromPath(value).toLowerCase() === 'snakefile') return 'Snakemake';
  const language = languageFromPath(value, mimeType);
  return LANGUAGE_LABELS[language] ?? language.charAt(0).toUpperCase() + language.slice(1);
}

// Three label tokens for the whole artifact panel — the "one status strip"
// typography (design.md §3.2 / D-31), shared by every preview (code, tables,
// markdown, git, notebook) so the strip reads in one voice instead of each
// preview inventing its own sub-header.
//
// D-31 draws the line these three sit on: MONO FOR DATA, SANS FOR CHROME. Mono
// is not decoration here; it is a claim that the glyphs matter (you will read
// this character by character, or the digits must not jitter). A word like
// "Modified" makes no such claim, so `STRIP_META_CLASS` stays sans.
//
// All three are `text-supporting` — the metadata role. The strip states what a
// thing is, where it lives and how big it is; none of that is a section label,
// so none of it takes `text-caps` (the ONE caps style). What separates them is
// the FACE and the ink, not a fourth size.

/**
 * The language chip and legend labels — mono, because a format name ("R",
 * "TSV", "JSON") is read as a token the way a path is, not as prose.
 *
 * This was an uppercase-tracked 11px style of its own, which made it a fourth
 * caps treatment competing with `text-caps`. The strip does not need one: the
 * chip is already distinguished from the path beside it by its ink.
 */
export const STRIP_LABEL_CLASS = 'font-mono text-supporting text-text-subtle';
/**
 * MONO — paths, git refs, and tabular counts.
 *
 * These earn it, which is exactly why D-31 left them alone: a path is read
 * character by character (l vs 1 vs I), and a count carrying `tabular-nums`
 * wants stable digit widths so it does not jitter as it changes.
 */
export const STRIP_IDENT_CLASS = 'font-mono text-supporting text-text-muted';
/** SANS — everything the strip states in prose (statuses, notes). */
export const STRIP_META_CLASS = 'text-supporting text-text-muted';

/** Splits a path so the strip can dim the directory and keep the filename legible. */
export function splitPathForStrip(path: string) {
  const name = basenameFromPath(path);
  return { directory: path.slice(0, path.length - name.length), name };
}

function titleCaseWords(value: string): string {
  return value
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/[-_]+/g, ' ')
    .split(/\s+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/**
 * The resolved colours a CSP-sandboxed preview must inline, for the family and
 * mode the app is actually in.
 *
 * `NotebookPreview` and `DocumentPreview` build their own HTML document and hand
 * it to a `sandbox=""` iframe under `default-src 'none'`. That document cannot
 * load the app stylesheet, so `var(--text-default)` resolves to nothing inside it
 * and the page would paint unstyled. Inlining literal hexes is therefore not a
 * shortcut, it is the only option — but they must come from the generated theme
 * data rather than being picked by hand. They used to be hand-picked light/dark
 * pairs (`#242321` on dark) that belonged to no family at all, while the three
 * families' dark grounds were `#16120c`, navy `#08213f` and `#1b1b19`.
 *
 * Since the neutrals were shared (2026-08-08) the GROUND is the same for every
 * family — `#1b1b19` in dark — so a hardcoded ground would now look right by
 * accident. The INK is not: `--text-default` is still per family, which is what
 * the "distinct ink" test in artifactUtils.test.ts asserts. Read both from here.
 *
 * Falls back to Parchment for an unknown family (a `theme_family` left in
 * localStorage by a build that had one we since removed), exactly as
 * `BioRouterMark` does.
 */
export function sandboxedSurface(family: ThemeFamily, theme: 'light' | 'dark') {
  return (GENERATED_THEMES[family] ?? GENERATED_THEMES.parchment)[theme].surface;
}

// Inject the desktop app's resolved theme as `window.__BR_VIZ_HOST_THEME__` so an
// Auto Visualiser figure/report rendered in a `srcdoc` iframe — which has no query
// string — follows the Biorouter app theme instead of the OS `prefers-color-scheme`.
// This is what keeps the side-panel preview identical to the expanded/opened view
// (both then resolve the same theme). The script must run before the figure's own
// runtime (`{{COMMON}}`), which sits right after `<head>`; a baked tool theme
// (`window.__BR_VIZ_THEME__`) still wins over this host default.
//
// Deliberately carries the light/dark mode ONLY, not the theme family: the figure
// on the other side is generated by the Rust backend
// (`crates/biorouter-mcp/src/autovisualiser/templates/_common.js`) and resolves
// light vs dark with a fixed palette. Injecting a family it cannot read would be
// decoration. See the note at `openArtifactInWindow` in main.ts.
export function withHostTheme(html: string, theme: 'light' | 'dark'): string {
  const tag = `<script>window.__BR_VIZ_HOST_THEME__=${JSON.stringify(theme)};</script>`;
  const marker = '<head>';
  const idx = html.indexOf(marker);
  if (idx === -1) return tag + html;
  return html.slice(0, idx + marker.length) + tag + html.slice(idx + marker.length);
}

export function titleFromResourceUri(uri?: string): string | null {
  if (!uri || uri.length > MAX_BROWSER_URL_BYTES || !uri.startsWith('ui://')) return null;
  let parts: string[];
  try {
    const parsed = new URL(uri);
    parts = [parsed.hostname, ...parsed.pathname.split('/')]
      .map((part) => part.trim())
      .filter(Boolean);
  } catch {
    parts = uri
      .replace(/^ui:\/\//, '')
      .split(/[/?#]/)
      .map((part) => part.trim())
      .filter(Boolean);
  }
  if (parts.length === 0) return null;
  if (parts.some((part) => /\.[a-z0-9]+$/i.test(part))) return null;

  const specific = parts.filter((part) => !GENERIC_UI_TITLE_PARTS.has(part.toLowerCase()));
  if (specific.length > 0) {
    const suffix = [...parts]
      .reverse()
      .find((part) => GENERIC_UI_TITLE_PARTS.has(part.toLowerCase()));
    return [...specific, ...(suffix ? [suffix] : [])].map(titleCaseWords).join(' ');
  }

  const lowerParts = parts.map((part) => part.toLowerCase());
  if (lowerParts.includes('interactive') && lowerParts.includes('chart'))
    return 'Interactive Chart';
  return parts.map(titleCaseWords).join(' ');
}

const MARKDOWN_EXTENSIONS = new Set(['markdown', 'md', 'qmd', 'rmd']);
const DELIMITED_EXTENSIONS = new Set(['csv', 'tsv']);

export function isMarkdownPath(value: string): boolean {
  return MARKDOWN_EXTENSIONS.has(extensionFromPath(value));
}

export function isDelimitedPath(value: string): boolean {
  return DELIMITED_EXTENSIONS.has(extensionFromPath(value));
}

// Parse a CSV/TSV table, honouring quoted fields (which may contain the
// delimiter, escaped quotes, and newlines). Rows are capped by the caller.
export function parseDelimitedTable(text: string, delimiter: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let inQuotes = false;

  for (let i = 0; i < text.length; i++) {
    const char = text[i];

    if (inQuotes) {
      if (char === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          inQuotes = false;
        }
      } else {
        field += char;
      }
      continue;
    }

    if (char === '"' && field === '') {
      inQuotes = true;
    } else if (char === delimiter) {
      row.push(field);
      field = '';
    } else if (char === '\n' || char === '\r') {
      if (char === '\r' && text[i + 1] === '\n') i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
    } else {
      field += char;
    }
  }

  if (field !== '' || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows.filter((r) => r.length > 1 || r[0] !== '');
}

export function looksLikePreviewableFile(value: string): boolean {
  const href = value.trim();
  if (!href || /^(https?|mailto|tel):/i.test(href)) return false;
  const localPath = parseFileLink(href)?.path;
  if (!localPath) return false;
  const trimmedPath = localPath.replace(/[/\\]+$/, '');
  const basename = basenameFromPath(trimmedPath).toLowerCase();
  if (
    !trimmedPath ||
    /^[/\\]+$/.test(localPath) ||
    /^(?:~|\.{1,2}|[a-z]:)$/i.test(trimmedPath) ||
    ['.git', '.hg', '.svn', '.ds_store'].includes(basename)
  ) {
    return false;
  }
  if (/^file:\/\//i.test(href)) return true;
  if (
    localPath.startsWith('/') ||
    localPath.startsWith('~/') ||
    localPath.startsWith('./') ||
    localPath.startsWith('../') ||
    /^[a-z]:[\\/]/i.test(localPath) ||
    localPath.startsWith('\\\\')
  ) {
    return true;
  }
  // Literal URI delimiters may follow a recognized extension in the real name.
  // This is classification only: never strip them from the path sent to the reader.
  const ext = extensionFromPath(localPath).split(/[#:%]/, 1)[0];
  return (
    TEXT_EXTENSIONS.has(ext) ||
    IMAGE_EXTENSIONS.has(ext) ||
    HTML_EXTENSIONS.has(ext) ||
    DOCUMENT_EXTENSIONS.has(ext)
  );
}

export function pathFromArtifactHref(href: string): string {
  if (/^file:\/\//i.test(href)) {
    try {
      return decodeURIComponent(new URL(href).pathname);
    } catch {
      return href.replace(/^file:\/\//i, '');
    }
  }
  return href;
}

const MAX_EMBEDDED_HTML_BYTES = 16 * 1024 * 1024;
const MAX_ENCODED_HTML_BYTES = Math.ceil((MAX_EMBEDDED_HTML_BYTES * 4) / 3) + 4;
const MAX_URI_LIST_BYTES = 64 * 1024;
function isHtmlMime(mimeType?: string): boolean {
  const essence = mimeType?.split(';', 1)[0]?.trim().toLowerCase();
  return essence === 'text/html' || essence === 'application/xhtml+xml';
}

function isUriListMime(mimeType?: string): boolean {
  return mimeType?.split(';', 1)[0]?.trim().toLowerCase() === 'text/uri-list';
}

function externalBrowserUrl(uri?: string): string | null {
  if (!uri || uri.length > MAX_BROWSER_URL_BYTES) return null;
  try {
    const url = new URL(uri);
    const safeProtocol = url.protocol === 'http:' || url.protocol === 'https:';
    return safeProtocol && !url.username && !url.password ? url.toString() : null;
  } catch {
    return null;
  }
}

function decodeResourceText(
  resource: { blob?: string; text?: string },
  maxBytes: number,
  maxEncodedBytes = Math.ceil((maxBytes * 4) / 3) + 4
): string | null {
  if (typeof resource.blob === 'string') {
    if (resource.blob.length > maxEncodedBytes) return null;
    try {
      const bin = atob(resource.blob);
      if (bin.length > maxBytes) return null;
      const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
      return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    } catch {
      return null;
    }
  }
  if (typeof resource.text === 'string') {
    if (resource.text.length > maxBytes) return null;
    return new TextEncoder().encode(resource.text).byteLength <= maxBytes ? resource.text : null;
  }
  return null;
}

export function decodeResourceHtml(resource: { blob?: string; text?: string }): string | null {
  return decodeResourceText(resource, MAX_EMBEDDED_HTML_BYTES, MAX_ENCODED_HTML_BYTES);
}

function browserUrlFromUriList(resource: { blob?: string; text?: string }): string | null {
  const list = decodeResourceText(resource, MAX_URI_LIST_BYTES);
  if (list === null) return null;
  for (const line of list.split(/\r?\n/)) {
    const candidate = line.trim();
    if (!candidate || candidate.startsWith('#')) continue;
    const url = externalBrowserUrl(candidate);
    if (url) return url;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Files the agent creates
//
// A `ui://` resource announces itself, but a file the agent writes with the
// developer extension leaves no trace in the panel unless the assistant happens
// to mention its path in prose. These helpers read the tool call itself, so a
// written report, R script, CSV or generated image opens in the preview the same
// way a visualization does.
// ---------------------------------------------------------------------------

// Tools whose arguments name a file the agent is creating or rewriting.
const FILE_WRITING_TOOLS = new Set([
  'create_file',
  'edit_file',
  'multi_edit',
  'notebook_edit',
  'str_replace_editor',
  'text_editor',
  'write_file',
]);

// `text_editor`-style commands that leave a file changed on disk. `view` and
// `undo_edit` do not produce something new to look at.
const MUTATING_EDITOR_COMMANDS = new Set(['create', 'diff', 'insert', 'str_replace', 'write']);

const PATH_ARGUMENT_KEYS = [
  'path',
  'file_path',
  'filePath',
  'filename',
  'file_name',
  'target_file',
  'absolute_path',
];

// Shell redirections and the conventional output flags. Deliberately narrow:
// scanning all of stdout for path-like text turns an `ls` into a dozen bogus
// artifacts, and the panel auto-opens whatever arrived last.
const SHELL_OUTPUT_RE =
  /(?:>>?|(?:^|\s)(?:--outfile|--output|--out|-out|-o)[=\s]+)\s*(?:"([^"]+)"|'([^']+)'|([^\s;|&>]+))/g;

// Code-execution wrappers: with the `code_execution` extension enabled (the
// default), the model never calls `text_editor`/`shell`/`write_file` directly —
// it calls `code_execution__execute_code` with a `code` string that imports and
// invokes those primitives inside the script. The direct-argument extraction
// then sees only `{ code, tool_graph }` and misses EVERY file the agent writes,
// so nothing surfaces in the artifact panel (observed live: a session that built
// `/tmp/biookf-rebuild/SCHEMA.md` via execute_code could not preview it — the
// path never reached the read IPC). `pathsFromCodeBlob` reaches inside the blob
// and pulls paths out of the embedded calls exactly as we do for direct ones.
const CODE_EXECUTION_TOOLS = new Set(['execute_code', 'run_code']);

// The tool name as the extension exposes it, e.g. `developer__text_editor`.
export function baseToolName(toolName: string): string {
  const delimiter = toolName.lastIndexOf('__');
  return delimiter === -1 ? toolName : toolName.slice(delimiter + 2);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

// Resolve a possibly-relative path against the session's working directory, so
// `results/plot.png` from a shell command becomes something the viewer can read.
export function resolveArtifactPath(rawPath: string, workingDir?: string): string | null {
  const path = pathFromArtifactHref(rawPath.trim().replace(/^["']|["']$/g, ''));
  return resolveLocalFilePath(path, workingDir);
}

function isPreviewableArtifactPath(path: string): boolean {
  const ext = extensionFromPath(path);
  return TEXT_EXTENSIONS.has(ext) || IMAGE_EXTENSIONS.has(ext) || HTML_EXTENSIONS.has(ext);
}

/** The directory portion of a path, without a trailing separator (`''` if none).
 *  Used to anchor a previewed markdown file's relative `![](…)` / `[](…)` links
 *  against the FILE's own directory rather than the app's working directory. */
export function dirnameFromPath(value: string): string {
  const clean = pathFromArtifactHref(value.trim());
  const name = localFileBasename(clean);
  return clean.slice(0, clean.length - name.length).replace(/[/\\]+$/, '');
}

export type MarkdownImageSource =
  | { kind: 'remote'; url: string }
  | { kind: 'local'; path: string }
  | { kind: 'blocked' };

/**
 * Where a markdown `<img>` src should load from inside the artifact preview.
 *   - `http(s)` and `data:` URIs pass through unchanged (rendered as `<img src>`),
 *     i.e. remote images are honoured exactly as chat already does — no new fetch.
 *   - A local reference (relative, absolute, `~`, or `file://`) is resolved
 *     against `baseDir` — the PREVIEWED FILE's directory — and returned as a
 *     `local` path for the caller to read through the main-process allowlisted
 *     read-file IPC and inline as a `data:` URI (CSP-safe).
 *   - A relative traversal that escapes `baseDir` (`../…`) or a bare relative
 *     path with no `baseDir` cannot be resolved and is `blocked` — the renderer
 *     shows a broken-image placeholder rather than a dead `<img>`.
 *
 * The main-process allowlist (`isFilePathAllowedForPreview`) is the authoritative
 * gate on what actually gets read; this helper only classifies the src and
 * anchors relative paths, so it never widens that allowlist.
 */
export function resolveMarkdownImageSource(src: string, baseDir?: string): MarkdownImageSource {
  const value = src.trim();
  if (!value) return { kind: 'blocked' };
  if (/^data:/i.test(value)) return { kind: 'remote', url: value };
  if (/^https?:\/\//i.test(value)) return { kind: 'remote', url: value };
  const resolved = resolveArtifactPath(value, baseDir);
  return resolved ? { kind: 'local', path: resolved } : { kind: 'blocked' };
}

// File paths a tool call is about to create, read off its arguments.
//
// Returns absolute paths only — a relative one without a `workingDir` to anchor
// it would resolve against the wrong directory in the viewer.
export function fileArtifactPathsFromToolCall(
  toolName: string,
  args: unknown,
  workingDir?: string
): string[] {
  const argRecord = asRecord(args);
  if (!argRecord) return [];
  const name = baseToolName(toolName);

  // Unwrap the code-execution wrapper (default config) before anything else —
  // the real tool calls live inside its `code` string, not in `argRecord`.
  if (CODE_EXECUTION_TOOLS.has(name)) {
    const code = argRecord.code;
    return typeof code === 'string' ? pathsFromCodeBlob(code, workingDir) : [];
  }

  if (name === 'shell' || name === 'bash') {
    const command = argRecord.command;
    const override = argRecord.working_directory;
    const shellDir =
      typeof override === 'string'
        ? (resolveArtifactPath(override, workingDir) ?? undefined)
        : workingDir;
    return typeof command === 'string' ? shellRedirectPaths(command, shellDir) : [];
  }

  if (!FILE_WRITING_TOOLS.has(name)) return [];

  // `text_editor` multiplexes read and write behind a `command` argument, so a
  // `view` must not open a preview of a file the agent only looked at.
  const command = argRecord.command;
  if (typeof command === 'string' && !MUTATING_EDITOR_COMMANDS.has(command)) return [];

  for (const key of PATH_ARGUMENT_KEYS) {
    const value = argRecord[key];
    if (typeof value !== 'string' || !value.trim()) continue;
    const resolved = resolveArtifactPath(value, workingDir);
    return resolved ? [resolved] : [];
  }
  return [];
}

// Output targets named by a shell command's redirections / `-o`|`--output`
// flags, anchored against `workingDir` and filtered to previewable files.
function shellRedirectPaths(command: string, workingDir?: string): string[] {
  const segments: { text: string; quoted: boolean[]; followedByAnd: boolean }[] = [];
  let start = 0;
  let quote = '';
  let quoted: boolean[] = [];
  let ambiguousScope = false;
  for (let index = 0; index < command.length; index += 1) {
    const char = command[index];
    if (char === '\\' && quote !== "'") {
      quoted[index - start] = true;
      quoted[index + 1 - start] = true;
      index += 1;
      continue;
    }
    if (quote) {
      quoted[index - start] = true;
      if (char === quote) quote = '';
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      quoted[index - start] = true;
      continue;
    }
    const and = command.slice(index, index + 2) === '&&';
    if (and || char === ';' || char === '\n') {
      segments.push({ text: command.slice(start, index), quoted, followedByAnd: and });
      if (and) index += 1;
      start = index + 1;
      quoted = [];
    } else if ('(){}|&'.includes(char) && command[index - 1] !== '>') {
      ambiguousScope = true;
    }
  }
  if (quote) return [];
  segments.push({ text: command.slice(start), quoted, followedByAnd: false });
  if (segments.some(({ text }) => /^\s*(?:if|for|while|until|case|function)\b/.test(text))) {
    ambiguousScope = true;
  }

  const paths: string[] = [];
  let cwd = ambiguousScope ? undefined : workingDir;
  for (const segment of segments) {
    for (const match of segment.text.matchAll(SHELL_OUTPUT_RE)) {
      if (segment.quoted[match.index]) continue;
      const candidate = match[1] ?? match[2] ?? match[3];
      const isQuoted = match[1] !== undefined || match[2] !== undefined;
      if (!candidate || (match[2] === undefined && /[$`]/.test(candidate))) continue;
      if (!isQuoted && /[*?[\]\\]/.test(candidate)) continue;
      if (candidate.startsWith('~') && (isQuoted || !candidate.startsWith('~/'))) continue;
      const resolved = resolveArtifactPath(candidate, cwd);
      if (resolved && isPreviewableArtifactPath(resolved)) paths.push(resolved);
    }

    if (/^\s*cd(?:\s|$)/.test(segment.text)) {
      const literal = /^\s*cd\s+(?:--\s+)?(?:"([^"$`]+)"|'([^']+)'|([^\s'"$`]+))\s*$/.exec(
        segment.text
      );
      const directory = literal && (literal[1] ?? literal[2] ?? literal[3]);
      const quotedDirectory = literal && (literal[1] !== undefined || literal[2] !== undefined);
      const literalDirectory =
        directory &&
        !directory.startsWith('-') &&
        !directory.includes('\\') &&
        (quotedDirectory || !/[*?[\]]/.test(directory)) &&
        (!directory.startsWith('~') || (!quotedDirectory && directory.startsWith('~/')));
      cwd =
        !ambiguousScope && segment.followedByAnd && directory && literalDirectory
          ? (resolveArtifactPath(directory, cwd) ?? undefined)
          : undefined;
    } else if (/^\s*(?:builtin|command|eval|source|\.)(?:\s|$)/.test(segment.text)) {
      cwd = undefined;
    }
  }
  return paths;
}

// --- code_execution unwrapping ----------------------------------------------
//
// The `code` string is JavaScript source, so extraction is textual and
// deliberately conservative — it mirrors the direct-call rules (mutating
// `text_editor` commands, `*_file` writes, `shell` redirect targets) applied to
// the calls found embedded in the blob, and resolves `` `${dir}/x.md` `` style
// template paths against simple `const dir = "…"` bindings. A path it cannot
// resolve (computed interpolation, a python-heredoc write) is skipped rather
// than guessed — no false artifacts, same posture as the direct path.

/** `const/let/var NAME = "literal"` string bindings, so a template path like
 *  `` `${dir}/SCHEMA.md` `` in an embedded call can be resolved. Only plain
 *  string / interpolation-free backtick right-hand sides are captured. */
function scanStringBindings(code: string): Map<string, string> {
  const bindings = new Map<string, string>();
  const re =
    /\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|`([^`$]*)`)/g;
  for (const m of code.matchAll(re)) {
    bindings.set(m[1], (m[2] ?? m[3] ?? m[4] ?? '').replace(/\\(["'`\\])/g, '$1'));
  }
  return bindings;
}

/** Resolve one JS string-literal token (quoted or template, optionally
 *  `String.raw`) to its concrete value, substituting `${ident}` from `bindings`.
 *  Returns null if it isn't a string literal or carries an unresolvable
 *  interpolation (a computed expression or an unknown identifier). */
function resolveStringLiteral(token: string, bindings: Map<string, string>): string | null {
  const t = token.trim().replace(/^String\.raw\s*/, '');
  if ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith("'") && t.endsWith("'"))) {
    return t.slice(1, -1).replace(/\\(.)/g, '$1');
  }
  if (t.startsWith('`') && t.endsWith('`')) {
    let out = '';
    for (const part of t.slice(1, -1).split(/(\$\{[^}]*\})/)) {
      if (part.startsWith('${')) {
        const id = /^\$\{\s*([A-Za-z_$][\w$]*)\s*\}$/.exec(part);
        if (!id || !bindings.has(id[1])) return null;
        out += bindings.get(id[1]);
      } else {
        out += part;
      }
    }
    return out;
  }
  return null;
}

/** Source text of each `callName({ … })` argument object found in `code`, via a
 *  string-aware balanced-brace scan (so braces inside string/template literals
 *  don't confuse nesting). */
function embeddedCallObjects(code: string, callName: string): string[] {
  const objects: string[] = [];
  const opener = new RegExp(`\\b${callName}\\s*\\(\\s*\\{`, 'g');
  let match: RegExpExecArray | null;
  while ((match = opener.exec(code))) {
    const start = code.indexOf('{', match.index);
    if (start === -1) continue;
    let depth = 0;
    let quote: string | null = null;
    let i = start;
    for (; i < code.length; i++) {
      const c = code[i];
      if (quote) {
        if (c === '\\') i++;
        else if (c === quote) quote = null;
        continue;
      }
      if (c === '"' || c === "'" || c === '`') quote = c;
      else if (c === '{') depth++;
      else if (c === '}' && --depth === 0) {
        i++;
        break;
      }
    }
    objects.push(code.slice(start, i));
  }
  return objects;
}

/** The string-literal token assigned to `key` in an object-literal source (only
 *  at a property boundary, so `file_text: "path: …"` can't spoof a `path`). */
function literalValueForKey(objectSrc: string, key: string): string | null {
  const re = new RegExp(
    `(?:^\\s*\\{\\s*|[{,]\\s*)${key}\\s*:\\s*` +
      `(String\\.raw\\s*\`[^\`]*\`|\`[^\`]*\`|"(?:[^"\\\\]|\\\\.)*"|'(?:[^'\\\\]|\\\\.)*')`
  );
  const m = re.exec(objectSrc);
  return m ? m[1] : null;
}

function pathsFromCodeBlob(code: string, workingDir?: string): string[] {
  const bindings = scanStringBindings(code);
  const seen = new Set<string>();
  const out: string[] = [];
  const add = (resolved: string | null) => {
    const anchored = resolved ? resolveArtifactPath(resolved, workingDir) : null;
    if (anchored && !seen.has(anchored)) {
      seen.add(anchored);
      out.push(anchored);
    }
  };
  const pathArgOf = (objectSrc: string): string | null => {
    for (const key of PATH_ARGUMENT_KEYS) {
      const token = literalValueForKey(objectSrc, key);
      const value = token ? resolveStringLiteral(token, bindings) : null;
      if (value) return value;
    }
    return null;
  };

  // Embedded `text_editor` — mutating commands only, mirroring the direct rule.
  for (const objectSrc of embeddedCallObjects(code, 'text_editor')) {
    const commandToken = literalValueForKey(objectSrc, 'command');
    const command = commandToken ? resolveStringLiteral(commandToken, bindings) : null;
    if (!command || !MUTATING_EDITOR_COMMANDS.has(command)) continue;
    add(pathArgOf(objectSrc));
  }

  // Embedded `write_file`/`create_file`/… — inherently writes, no command gate.
  for (const tool of FILE_WRITING_TOOLS) {
    if (tool === 'text_editor') continue;
    for (const objectSrc of embeddedCallObjects(code, tool)) add(pathArgOf(objectSrc));
  }

  // Embedded `shell`/`bash` — redirect / `-o` targets in the command literal.
  for (const callName of ['shell', 'bash']) {
    for (const objectSrc of embeddedCallObjects(code, callName)) {
      const commandToken = literalValueForKey(objectSrc, 'command');
      const command = commandToken ? resolveStringLiteral(commandToken, bindings) : null;
      if (!command) continue;
      const directoryToken = literalValueForKey(objectSrc, 'working_directory');
      const directory = directoryToken ? resolveStringLiteral(directoryToken, bindings) : null;
      const hasDirectory = /[,{]\s*working_directory\s*:/.test(objectSrc);
      const shellDir = hasDirectory
        ? directory
          ? (resolveArtifactPath(directory, workingDir) ?? undefined)
          : undefined
        : workingDir;
      for (const path of shellRedirectPaths(command, shellDir)) {
        if (!seen.has(path)) {
          seen.add(path);
          out.push(path);
        }
      }
    }
  }

  return out;
}

export function artifactSourceFromResource(
  content: EmbeddedResource & { type: 'resource' },
  fallbackTitle = 'Artifact'
): ArtifactSource | null {
  const resource = content.resource as ResourceContents & {
    uri?: string;
    mimeType?: string;
    blob?: string;
    text?: string;
    _meta?: Record<string, unknown>;
  };
  if (resource.uri && resource.uri.length > MAX_BROWSER_URL_BYTES) return null;
  const title = sanitizeArtifactTitle(
    titleFromResourceUri(resource.uri) ??
      (resource.uri ? basenameFromPath(resource.uri) : fallbackTitle),
    fallbackTitle
  );
  const prefSize = resource._meta?.['mcpui.dev/ui-preferred-frame-size'] as
    | [string, string]
    | undefined;
  const pxOf = (v?: string): number | undefined => {
    if (!v) return undefined;
    const n = parseInt(v, 10);
    return Number.isFinite(n) && /px$/.test(v) ? n : undefined;
  };
  const preferredWidth = pxOf(prefSize?.[0]);
  const preferredHeight = pxOf(prefSize?.[1]);

  const resourceUrl = externalBrowserUrl(resource.uri);
  if (resourceUrl) {
    return { kind: 'externalUrl', title, url: resourceUrl };
  }

  if (isHtmlMime(resource.mimeType)) {
    const html = decodeResourceHtml(resource);
    if (html === null) return null;
    return {
      kind: 'html',
      title,
      html,
      sourceUri: resource.uri,
      preferredWidth,
      preferredHeight,
    };
  }

  if (isUriListMime(resource.mimeType)) {
    const url = browserUrlFromUriList(resource);
    return url ? { kind: 'externalUrl', title, url } : null;
  }

  return { kind: 'mcpResource', title, resource, preferredWidth, preferredHeight };
}

export function artifactSourceFromResourceLink(resource: RawResource): ArtifactSource | null {
  const url = externalBrowserUrl(resource.uri);
  if (!url) return null;
  return {
    kind: 'externalUrl',
    title: sanitizeArtifactTitle(
      resource.title?.trim() || resource.name?.trim() || basenameFromPath(resource.uri)
    ),
    url,
  };
}

/**
 * A displayable `src` for an image preview, plus the cleanup that owns it.
 *
 * The main process sends small images as a data URL and large ones as raw
 * bytes (see `ArtifactFilePreview`'s image variant). Callers must not branch on
 * that themselves — every consumer that did would be a place a `blob:` URL
 * could leak. `revoke` is a no-op for the data-URL case, so callers can always
 * call it unconditionally on unmount.
 */
export function imageSourceForPreview(file: {
  mimeType: string;
  dataUrl?: string;
  bytes?: ArrayBuffer;
}): { src: string; revoke: () => void } {
  if (file.bytes) {
    const url = URL.createObjectURL(new Blob([file.bytes], { type: file.mimeType }));
    return { src: url, revoke: () => URL.revokeObjectURL(url) };
  }
  return { src: file.dataUrl ?? '', revoke: () => {} };
}
