import ts from 'typescript';

/**
 * Every string the renderer shows a person, pulled out of the source with the
 * TypeScript parser rather than a regex.
 *
 * **Why an AST and not a grep.** A raw grep over `src/` cannot answer a question
 * about COPY, because each of these words is overwhelmingly more common as
 * something else. Measured on `main` at f350cdfc: `color`/`colour` 126/106
 * matches, `center`/`centre` 49/35, `cancelled` 204 — and almost all of those
 * are Tailwind class strings (`text-center`), CSS properties, web-platform
 * identifiers (`scrollBehavior`, `role="dialog"`), API enum values
 * (`status === 'cancelled'`) and prose inside comments. A convention for the
 * words a user reads cannot be decided — or enforced — from a number dominated
 * by words no user ever sees.
 *
 * **Two tiers, because copy is not written in one shape here.** An earlier
 * draft of this file read a pure allow-list of carriers (the attributes the
 * handoff named: `title`, `aria-label`, `placeholder`, `description`). Audited
 * against the tree, it missed most of the app's sentences: the provider notes
 * in `providerOrdering.ts`, the greetings array, `formatSupport.ts`'s
 * suggestions, `artifactFileLinks.ts`'s refusal reasons, and every conditional
 * sentence written as a `{…}` JSX child. So:
 *
 * 1. A **known copy carrier** — JSX text, a `{…}` JSX child, one of
 *    `COPY_ATTRIBUTES`, one of `COPY_PROPERTIES`, a toast/alert argument —
 *    contributes its string whatever its length, so a one-word `label` counts.
 * 2. Any **other** string that reads as a sentence (three or more words, one of
 *    them four letters or longer, no code punctuation) counts too.
 *
 * Both tiers skip `DENIED_*`: class strings, `cn()`/`clsx()`/`cva()`,
 * `console.*`/`log.*` (developer logs, not copy) and thrown `Error`s
 * (developer-facing, and a stated non-goal of the copy convention).
 */
export interface CopyString {
  /** Path as the caller supplied it, so a failure names a file a reader can open. */
  file: string;
  /** 1-indexed, pointing at the line the literal starts on. */
  line: number;
  text: string;
  /** The carrier this string was written into, for the failure message. */
  carrier: string;
  /** Which tier admitted it — `carrier` for tier 1, `prose` for tier 2. */
  tier: 'carrier' | 'prose';
}

/**
 * JSX attributes whose value a person reads — as visible text, as a tooltip, or
 * through a screen reader.
 *
 * `aria-labelledby`/`aria-describedby`/`aria-controls` are deliberately ABSENT:
 * their values are element ids, not prose, and including them is how a guard
 * starts failing on an identifier.
 */
const COPY_ATTRIBUTES = new Set([
  'alt',
  'aria-label',
  'aria-placeholder',
  'aria-roledescription',
  'aria-valuetext',
  'cancelLabel',
  'caption',
  'confirmLabel',
  'description',
  'emptyMessage',
  'emptyText',
  'errorMessage',
  'fieldLabel',
  'heading',
  'helpText',
  'helperText',
  'hint',
  'label',
  'message',
  'placeholder',
  'secondaryLabel',
  'subtitle',
  'title',
  'tooltip',
  'tooltipText',
]);

/**
 * The same names as object-literal properties, plus the shapes this codebase
 * actually uses for a sentence it will show: `msg`, `reason`, `suggestion`,
 * `note`, `warning`. Section descriptors, menu templates, toast option bags and
 * `dialog.showMessageBox` all carry copy this way rather than as JSX.
 */
const COPY_PROPERTIES = new Set([
  'body',
  'cancelLabel',
  'caption',
  'confirmLabel',
  'description',
  'detail',
  'emptyMessage',
  'emptyText',
  'errorMessage',
  'heading',
  'helpText',
  'helperText',
  'hint',
  'label',
  'message',
  'msg',
  'note',
  'placeholder',
  'reason',
  'subtitle',
  'suggestion',
  'summary',
  'title',
  'tooltip',
  'warning',
]);

/** Callees whose string arguments are shown verbatim. */
const COPY_CALLEES = new Set([
  'alert',
  'confirm',
  'notify',
  'showToast',
  'toast',
  'toast.custom',
  'toast.error',
  'toast.info',
  'toast.loading',
  'toast.message',
  'toast.success',
  'toast.warning',
]);

/** Attributes that carry an identifier, a class list, a URL or a token. */
const DENIED_ATTRIBUTES = new Set([
  'aria-controls',
  'aria-describedby',
  'aria-labelledby',
  'aria-owns',
  'class',
  'className',
  'color',
  'href',
  'htmlFor',
  'id',
  'key',
  'name',
  'path',
  'role',
  'src',
  'style',
  'testId',
  'to',
  'type',
  'value',
  'variant',
]);

/**
 * Calls whose string arguments are never copy: the class-name helpers, and the
 * developer logs. A log line is prose, and it is prose no user reads — sweeping
 * 350 of them would bury the copy this guard exists to protect.
 */
const DENIED_CALLEES = [
  /^cn$/,
  /^clsx$/,
  /^cva$/,
  /^twMerge$/,
  /^console\./,
  /^log\./,
  /^logger\./,
  /^window\.electron\.log/,
  /logInfo$|logError$|logWarn$/,
];

function calleeName(node: ts.CallExpression | ts.NewExpression, sf: ts.SourceFile): string {
  return node.expression.getText(sf).split('\n')[0];
}

/**
 * The carrier a literal belongs to: walk up through the wrappers a string can
 * sit inside (a ternary, a `??` fallback, an array, a template) until something
 * that names the string's purpose is reached.
 */
function carrierOf(
  literal: ts.Node,
  sf: ts.SourceFile
): { name: string; denied: boolean; copy: boolean } {
  let node: ts.Node | undefined = literal.parent;
  while (node) {
    if (
      ts.isConditionalExpression(node) ||
      ts.isBinaryExpression(node) ||
      ts.isParenthesizedExpression(node) ||
      ts.isArrayLiteralExpression(node) ||
      ts.isTemplateExpression(node) ||
      ts.isTemplateSpan(node) ||
      ts.isAsExpression(node)
    ) {
      node = node.parent;
      continue;
    }
    if (ts.isJsxAttribute(node)) {
      const name = node.name.getText(sf);
      return {
        name: `attr:${name}`,
        denied: DENIED_ATTRIBUTES.has(name),
        copy: COPY_ATTRIBUTES.has(name),
      };
    }
    if (ts.isJsxExpression(node)) {
      // An attribute's `{…}` — keep walking so the attribute names it. A CHILD
      // `{…}` (its parent is the element) is copy in its own right.
      if (ts.isJsxAttribute(node.parent)) {
        node = node.parent;
        continue;
      }
      return { name: 'jsx-child', denied: false, copy: true };
    }
    if (ts.isPropertyAssignment(node)) {
      const name =
        ts.isIdentifier(node.name) || ts.isStringLiteral(node.name) ? node.name.text : '?';
      return { name: `prop:${name}`, denied: false, copy: COPY_PROPERTIES.has(name) };
    }
    if (ts.isCallExpression(node)) {
      const name = calleeName(node, sf);
      return {
        name: `call:${name}`,
        denied: DENIED_CALLEES.some((pattern) => pattern.test(name)),
        copy: COPY_CALLEES.has(name),
      };
    }
    if (ts.isNewExpression(node)) {
      const name = calleeName(node, sf);
      // A thrown Error is developer-facing; the copy convention is a stated
      // non-goal for it, and including it would sweep 113 strings no user sees.
      return { name: `new:${name}`, denied: /Error$/.test(name), copy: false };
    }
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      return { name: 'import', denied: true, copy: false };
    }
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
      return { name: `var:${node.name.text}`, denied: false, copy: false };
    }
    if (ts.isReturnStatement(node)) return { name: 'return', denied: false, copy: false };
    if (
      ts.isJsxElement(node) ||
      ts.isJsxFragment(node) ||
      ts.isBlock(node) ||
      ts.isSourceFile(node)
    ) {
      return { name: ts.SyntaxKind[node.kind], denied: false, copy: false };
    }
    node = node.parent;
  }
  return { name: 'unknown', denied: false, copy: false };
}

/**
 * Reject a "string" that is really an identifier, a token, a path or a class
 * list.
 *
 * The lone-lowercase-word rule is the one that earns its place: this app
 * carries the daemon's own discriminants in copy-shaped properties —
 * `reason: 'cancelled'` in `useIngestStream`, `{ kind: 'cancelled' }` in
 * `SecretRequestCard` — and those are WIRE VALUES that must keep the server's
 * spelling. Copy shown to a person is capitalised or more than one word, so
 * dropping a bare lowercase token loses no sentence and stops the guard
 * demanding that an API value be re-spelled.
 */
function looksLikeToken(text: string): boolean {
  const trimmed = text.trim();
  if (!/[A-Za-z]{2}/.test(trimmed)) return true;
  if (!/\s/.test(trimmed) && /[_./:@-]/.test(trimmed)) return true;
  if (!/\s/.test(trimmed) && trimmed === trimmed.toLowerCase()) return true;
  return false;
}

/** Tier 2: does this read as a sentence rather than as code? */
export function readsAsProse(text: string): boolean {
  const trimmed = text.trim();
  if (/[{}<>$=|\\]/.test(trimmed)) return false;
  if (/^https?:\/\//.test(trimmed)) return false;
  const words = trimmed.split(/\s+/);
  if (words.length < 3) return false;
  if (!words.some((word) => /^[A-Za-z]{4,}$/.test(word))) return false;
  // A run of Tailwind-ish tokens is multi-word but is not prose.
  const tokenish = words.filter((word) => /[:/[\]]/.test(word) || /^[a-z]+-[a-z0-9-]+$/.test(word));
  return tokenish.length < words.length / 2;
}

export function extractUserVisibleStrings(source: string, file: string): CopyString[] {
  const isTsx = file.endsWith('.tsx');
  const sourceFile = ts.createSourceFile(
    isTsx ? 'source.tsx' : 'source.ts',
    source,
    ts.ScriptTarget.Latest,
    true,
    isTsx ? ts.ScriptKind.TSX : ts.ScriptKind.TS
  );
  const found: CopyString[] = [];
  const lineOf = (position: number) => sourceFile.getLineAndCharacterOfPosition(position).line + 1;

  const visit = (node: ts.Node) => {
    if (ts.isJsxText(node)) {
      const text = node.text.trim();
      if (text && !looksLikeToken(text)) {
        found.push({
          file,
          line: lineOf(node.getStart(sourceFile)),
          text,
          carrier: 'jsx-text',
          tier: 'carrier',
        });
      }
    } else if (
      ts.isStringLiteral(node) ||
      ts.isNoSubstitutionTemplateLiteral(node) ||
      ts.isTemplateHead(node) ||
      ts.isTemplateMiddle(node) ||
      ts.isTemplateTail(node)
    ) {
      const text = node.text.trim();
      if (text && !looksLikeToken(text)) {
        const carrier = carrierOf(node, sourceFile);
        if (!carrier.denied) {
          const tier = carrier.copy ? 'carrier' : readsAsProse(text) ? 'prose' : undefined;
          if (tier) {
            found.push({
              file,
              line: lineOf(node.getStart(sourceFile)),
              text,
              carrier: carrier.name,
              tier,
            });
          }
        }
      }
    }
    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return found;
}
