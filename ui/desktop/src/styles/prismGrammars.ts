/**
 * Grammar additions for the code the artifact panel is actually shown.
 *
 * A side-effect module, imported once from `codeTheme.ts` so every highlighter
 * that reads the palette gets the grammars too. It registers on the shared
 * refractor instance: `PrismLight.registerLanguage` writes into
 * `refractor/core`, the same module-level object the full `Prism` build
 * highlights with, so every `Prism` import sees the result without this repo
 * depending on refractor directly. refractor skips a `displayName` it already
 * holds, and every mutation below is guarded, so the module is idempotent.
 *
 * 1. `csv-table` / `tsv-table` — the RAW view of a table. Prism's bundled `csv`
 *    grammar has two token kinds (value, comma), so a results table rendered in
 *    one colour, and `tsv` is not a Prism language at all (0 token spans). The
 *    header row, quoted strings and missing values each take a palette stop
 *    and the delimiter recedes. Numbers deliberately take NO token: a results
 *    table is mostly digits, and colouring them painted a wall of amber.
 *
 * 2. R and Python CALLS. Prism's R grammar has no `function` token at all, and
 *    its Python grammar only marks the name after `def`. In an analysis script
 *    the calls are most of the structure (`DESeq(dds)`, `pd.read_csv(…)`), and
 *    without them a file reads as ink with the odd keyword.
 *
 * 3. Log noise. The `log` grammar's `property` token matches prose labels
 *    (`Module rseqc:`, `Completed at:`) — aliased here so the theme can keep them
 *    ink while JSON/TOML keys take the function hue — and a Nextflow task hash
 *    (`[4f/a1c2e9]`) was split into number-coloured digit speckle.
 */
import { PrismLight } from 'react-syntax-highlighter';

type Grammar = Record<string, unknown>;
type PrismHost = {
  languages: Record<string, Grammar> & {
    insertBefore: (inside: string, before: string, insert: Grammar) => Grammar;
  };
};
type Syntax = ((prism: PrismHost) => void) & { displayName: string; aliases: string[] };

function syntax(displayName: string, body: (prism: PrismHost) => void): Syntax {
  const fn = body as Syntax;
  fn.displayName = displayName;
  fn.aliases = [];
  return fn;
}

function delimited(name: string, delimiter: ',' | '\t'): Syntax {
  const d = delimiter === ',' ? ',' : '\\t';
  return syntax(name, (prism) => {
    if (name in prism.languages) return;
    prism.languages[name] = {
      // No `m` flag: `^` anchors to the start of the file, so only line one is
      // the header. It runs first, over the unsplit text.
      header: {
        pattern: /^[^\r\n]+/,
        alias: 'property',
        inside: { punctuation: new RegExp(d) },
      },
      string: { pattern: /"(?:[^"]|"")*"/, greedy: true },
      nil: {
        pattern: new RegExp(
          `(?<=^|${d})[ ]*(?:NA|N/A|NaN|nan|NULL|null|None)[ ]*(?=${d}|\\r?$)`,
          'm'
        ),
        alias: 'comment',
      },
      punctuation: new RegExp(d),
    };
  });
}

PrismLight.registerLanguage('csv-table', delimited('csv-table', ','));
PrismLight.registerLanguage('tsv-table', delimited('tsv-table', '\t'));

PrismLight.registerLanguage(
  'biorouter-grammar-additions',
  syntax('biorouter-grammar-additions', (prism) => {
    const { languages } = prism;
    if (languages.r && !('function' in languages.r)) {
      // Before `operator`, so after `keyword`: `if (`, `for (` and `function (`
      // are matched as keywords first and stay keywords.
      languages.insertBefore('r', 'operator', {
        namespace: /\b[A-Za-z][\w.]*(?=::)/,
        function: /\b[A-Za-z.][\w.]*(?=\s*\()/,
      });
    }
    if (languages.python && !('call' in languages.python)) {
      // Before `boolean`, so after `builtin`: `print(` keeps the builtin hue.
      languages.insertBefore('python', 'boolean', {
        call: { pattern: /\b[A-Za-z_]\w*(?=\s*\()/, alias: 'function' },
      });
    }
    const log = languages.log as (Grammar & { property?: Grammar }) | undefined;
    if (log) {
      if (log.property && !('alias' in log.property)) log.property.alias = 'log-label';
      if (!('task-hash' in log)) {
        languages.insertBefore('log', 'hash', { 'task-hash': /\b[0-9a-f]{2}\/[0-9a-f]{6}\b/ });
      }
    }
  })
);
