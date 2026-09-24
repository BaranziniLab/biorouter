import {
  memo,
  useId,
  useLayoutEffect,
  useMemo,
  useState,
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
} from 'react';
import ReactMarkdown, { type Components, type Options } from 'react-markdown';
import remarkBreaks from 'remark-breaks';
import remarkGfm from 'remark-gfm';
import { Button } from '../../ui/button';
import { ChevronDown, ChevronUp, Image as ImageIcon } from '../../icons/app-icons';
import { CLAMP_MAX_HEIGHT_PX, describeMessageLength } from '../../../utils/messageClamp';
import { timelineCopy } from './copy';
import { CopyIconButton } from './TimelineCopy';

/**
 * A Crew message body: markdown, so an agent's answer reads as formatted text
 * instead of showing its backticks (baseline critique, F5) — but markdown with
 * NOTHING ACTIVE in it, because every body here was written by someone else:
 * another member of the workspace, or their agent.
 *
 * It is the parser and plugins the app's chat renderer uses (`react-markdown`,
 * `remark-gfm`, `remark-breaks`, so a single newline stays a line), with a
 * stricter element set than `MarkdownContent`, which renders the viewer's OWN
 * assistant and has reach this surface must not have:
 *
 * - **No image is ever fetched.** `![](https://…)` becomes a link the person may
 *   choose to open. An `<img>` would load the moment the message is drawn — a
 *   read receipt and an IP address for whoever posted it, and a way for text an
 *   agent was steered into writing to carry a private workspace's contents out
 *   in a URL. `MarkdownContent` also reads local image paths through
 *   `readArtifactFile`; nothing here touches the viewer's disk.
 * - **Links are http, https or mailto**, opened in the system browser. Any other
 *   scheme (and a relative path, which here could only name the viewer's own
 *   files) renders as plain text.
 * - **Raw HTML is text.** react-markdown turns an HTML node into a text node
 *   unless `rehype-raw` is installed, and it is not — so `<svg onload>` or
 *   `<script>` shows as the characters typed.
 * - **No math, no syntax highlighting, no "Run".** Math would bring KaTeX's
 *   `\href`; highlighting is decoration; running a teammate's command is a
 *   decision for a terminal, not a click.
 * - **Headings are bold lines, not `<h1>`–`<h6>`**: a message's `# Title` must
 *   not become a heading of the page, beside the channel's own `<h1>`.
 *
 * A long body folds by the chat's rule (`utils/messageClamp.ts`): above ten
 * lines or 600 characters, behind "Show more" with its size stated.
 *
 * A table or a code block that is wider than the column scrolls sideways in its
 * own box, and only then is that box a named, focusable region, so a keyboard
 * can scroll it. One that fits is a plain box: every agent table used to be a
 * Tab stop of its own, overflowing or not, which put ten stops between the log
 * and the composer (Q2-12). A table's region is named for its header cells
 * ("Table: Sample, od600_t0"), so two tables never share a name (Q2-57).
 */

const REMARK_PLUGINS: NonNullable<Options['remarkPlugins']> = [remarkGfm, remarkBreaks];

const ALLOWED_PROTOCOLS = new Set(['http:', 'https:', 'mailto:']);

/** The URL, normalized, when it is an absolute http(s) or mailto link; otherwise null. */
export function safeExternalHref(url: unknown, allowMailto = true): string | null {
  if (typeof url !== 'string') return null;
  const trimmed = url.trim();
  if (!/^(?:https?|mailto):/i.test(trimmed)) return null;
  try {
    const parsed = new URL(trimmed);
    if (!ALLOWED_PROTOCOLS.has(parsed.protocol)) return null;
    if (!allowMailto && parsed.protocol === 'mailto:') return null;
    return parsed.href;
  } catch {
    return null;
  }
}

/** Every URL react-markdown emits passes this first; a refused one becomes empty. */
function urlTransform(url: string): string {
  return safeExternalHref(url) ?? '';
}

/** Open in the system browser, never by navigating the renderer. */
function openExternally(event: MouseEvent<HTMLAnchorElement>, href: string) {
  if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
    return;
  }
  const open = window.electron?.openExternal;
  if (!open) return;
  event.preventDefault();
  void open(href);
}

interface HastLike {
  type?: string;
  value?: unknown;
  tagName?: string;
  properties?: { className?: unknown };
  children?: unknown[];
}

function hastText(node: unknown): string {
  if (!node || typeof node !== 'object') return '';
  const current = node as HastLike;
  if (current.type === 'text' && typeof current.value === 'string') return current.value;
  return Array.isArray(current.children) ? current.children.map(hastText).join('') : '';
}

function codeChild(node: unknown): HastLike | null {
  const children = (node as HastLike | undefined)?.children;
  if (!Array.isArray(children)) return null;
  const code = children.find(
    (child) => (child as HastLike)?.type === 'element' && (child as HastLike).tagName === 'code'
  );
  return (code as HastLike | undefined) ?? null;
}

function fenceLanguage(code: HastLike | null): string {
  const className = code?.properties?.className;
  const classes = Array.isArray(className) ? className : [];
  const language = classes
    .map(String)
    .find((name) => name.startsWith('language-'))
    ?.slice('language-'.length);
  return language && /^[\w.+-]{1,32}$/.test(language) ? language : '';
}

/**
 * Whether an element is wider inside than it is shown, now and after any resize of it or of its
 * first child (the table or the code). Where there is no `ResizeObserver` (jsdom) it is measured
 * once. The answer decides only whether the box is a scrollable region a keyboard can reach.
 */
function useOverflowsSideways<T extends HTMLElement>(): [(node: T | null) => void, boolean] {
  const [node, setNode] = useState<T | null>(null);
  const [overflows, setOverflows] = useState(false);
  useLayoutEffect(() => {
    if (!node) return;
    const measure = () => setOverflows(node.scrollWidth > node.clientWidth + 1);
    measure();
    if (typeof ResizeObserver !== 'function') return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    if (node.firstElementChild) observer.observe(node.firstElementChild);
    return () => observer.disconnect();
  }, [node]);
  return [setNode, overflows];
}

/** The attributes that make a scrolling box a region a keyboard can reach, when it scrolls. */
function scrollRegion(overflows: boolean, name: string) {
  return overflows ? { role: 'region', 'aria-label': name, tabIndex: 0 } : {};
}

function CodeBlock({ text, language }: { text: string; language: string }) {
  const [measure, overflows] = useOverflowsSideways<HTMLPreElement>();
  return (
    <div className="crew-md-code">
      <div className="crew-md-code-head">
        <span className="crew-md-code-lang text-supporting text-text-muted">
          {language || timelineCopy.code}
        </span>
        <CopyIconButton text={text} label={timelineCopy.copyCode} />
      </div>
      <pre
        ref={measure}
        className="crew-md-code-body"
        {...scrollRegion(overflows, timelineCopy.codeRegion(language))}
      >
        <code>{text}</code>
      </pre>
    </div>
  );
}

/** The text of a table's header cells, from its markdown tree. */
function headerCells(node: unknown): string[] {
  const cells: string[] = [];
  const visit = (current: unknown) => {
    if (!current || typeof current !== 'object') return;
    const element = current as HastLike;
    if (element.type === 'element' && element.tagName === 'th') {
      const text = hastText(element).replace(/\s+/g, ' ').trim();
      if (text) cells.push(text);
      return;
    }
    if (Array.isArray(element.children)) element.children.forEach(visit);
  };
  visit(node);
  return cells;
}

function TableScroll({ node, children }: { node: unknown; children?: ReactNode }) {
  const [measure, overflows] = useOverflowsSideways<HTMLDivElement>();
  const name = timelineCopy.tableNamed(headerCells(node).join(', '));
  return (
    <div ref={measure} className="crew-md-table-scroll" {...scrollRegion(overflows, name)}>
      <table className="crew-md-table">{children}</table>
    </div>
  );
}

function Heading({ children }: { children?: ReactNode }) {
  return <p className="crew-md-heading">{children}</p>;
}

const COMPONENTS: Components = {
  p: ({ children }) => <p className="crew-md-p">{children}</p>,
  a: ({ href, children }) => {
    const safe = safeExternalHref(href);
    if (!safe) return <span className="crew-md-unlinked">{children}</span>;
    return (
      <a
        href={safe}
        target="_blank"
        rel="noopener noreferrer"
        className="crew-md-link"
        // Where it really goes, whatever its words say.
        title={safe}
        onClick={(event) => openExternally(event, safe)}
      >
        {children}
      </a>
    );
  },
  img: ({ src, alt }) => {
    const name = typeof alt === 'string' ? alt.trim() : '';
    const label = name ? timelineCopy.imageNamed(name) : timelineCopy.image;
    const safe = safeExternalHref(src, false);
    const content = (
      <>
        <ImageIcon aria-hidden className="crew-md-image-icon" />
        {label}
      </>
    );
    return safe ? (
      <a
        href={safe}
        target="_blank"
        rel="noopener noreferrer"
        className="crew-md-link crew-md-image"
        title={safe}
        onClick={(event) => openExternally(event, safe)}
      >
        {content}
      </a>
    ) : (
      <span className="crew-md-image">{content}</span>
    );
  },
  pre: ({ node }) => {
    const code = codeChild(node);
    return <CodeBlock text={hastText(code).replace(/\n$/, '')} language={fenceLanguage(code)} />;
  },
  code: ({ children }) => <code className="crew-md-code-inline">{children}</code>,
  h1: Heading,
  h2: Heading,
  h3: Heading,
  h4: Heading,
  h5: Heading,
  h6: Heading,
  ul: ({ children, className }) => (
    <ul
      className="crew-md-list"
      data-ordered="false"
      data-tasks={className?.includes('contains-task-list') ? 'true' : undefined}
    >
      {children}
    </ul>
  ),
  ol: ({ children, start }) => (
    <ol className="crew-md-list" data-ordered="true" start={start}>
      {children}
    </ol>
  ),
  li: ({ children }) => <li className="crew-md-item">{children}</li>,
  input: ({ checked }) => (
    <input
      type="checkbox"
      className="crew-md-task-box"
      checked={checked === true}
      disabled
      readOnly
    />
  ),
  blockquote: ({ children }) => <blockquote className="crew-md-quote">{children}</blockquote>,
  hr: () => <hr className="crew-md-rule" />,
  table: ({ node, children }) => <TableScroll node={node}>{children}</TableScroll>,
  th: ({ children, style }) => (
    <th className="crew-md-cell" data-head="true" style={style}>
      {children}
    </th>
  ),
  td: ({ children, style }) => (
    <td className="crew-md-cell" style={style}>
      {children}
    </td>
  ),
};

/** The markdown alone, unfolded. Memoized on the text: a timeline re-renders often. */
export const CrewMarkdown = memo(function CrewMarkdown({ text }: { text: string }) {
  return (
    <div className="crew-md text-body text-text-default">
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        urlTransform={urlTransform}
        components={COMPONENTS}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});

/**
 * The body with the long-message fold. The control sits below the text and
 * states the size, because the size is what tells you whether to expand; the
 * cut is faded with a mask, so it reads right on any ground (a hovered row).
 */
export function MessageBody({ body }: { body: string }) {
  const text = typeof body === 'string' ? body : '';
  const { shouldClamp, label } = useMemo(() => describeMessageLength(text), [text]);
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  const clamped = shouldClamp && !open;
  const style = clamped
    ? ({ '--crew-clamp-height': `${CLAMP_MAX_HEIGHT_PX}px` } as CSSProperties)
    : undefined;

  return (
    <>
      <div
        id={bodyId}
        className="crew-message-body"
        data-clamped={clamped ? 'true' : undefined}
        style={style}
      >
        <CrewMarkdown text={text} />
      </div>
      {shouldClamp && (
        <div className="crew-message-fold">
          <Button
            type="button"
            variant="ghost"
            size="xs"
            className="text-text-muted"
            aria-expanded={open}
            aria-controls={bodyId}
            onClick={() => setOpen((value) => !value)}
          >
            {open ? <ChevronUp aria-hidden /> : <ChevronDown aria-hidden />}
            {open ? timelineCopy.showLess : timelineCopy.showMore}
          </Button>
          <span className="text-supporting text-text-muted tabular-nums">{label}</span>
        </div>
      )}
    </>
  );
}
