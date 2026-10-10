import React, {
  useState,
  useEffect,
  useRef,
  memo,
  useMemo,
  createContext,
  useContext,
} from 'react';
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
  type ExtraProps,
  type Options,
} from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkBreaks from 'remark-breaks';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { useResolvedTheme, useThemeFamily } from '../contexts/ThemeContext';
import {
  CODE_FONT_FAMILY,
  CODE_FONT_SIZE,
  CODE_LINE_HEIGHT,
  codeThemesByFamily,
} from '../styles/codeTheme';
import { AlertTriangle, Check, Copy, Image as ImageIcon, Play } from './icons/app-icons';
import { TranscriptIconButton } from './transcript/TranscriptIconButton';
import { scrollRegionProps, useOverflowsSideways } from './transcript/useOverflowsSideways';
import { transcriptCopy } from './transcript/copy';
import { wrapHTMLInCodeBlock } from '../utils/htmlSecurity';
import { normalizeExternalHttpUrl } from '../utils/externalUrl';
import { runnableCommandFromCodeBlock } from '../utils/shellCommandBlock';
import { copyToClipboard } from '../utils/clipboard';
import { useTransientValue } from '../hooks/useTransientFlag';
import type { ArtifactFilePreview, ArtifactSource } from './artifacts/artifactTypes';
import {
  imageSourceForPreview,
  looksLikePreviewableFile,
  normalizeCodeLanguage,
  resolveMarkdownImageSource,
} from './artifacts/artifactUtils';
import {
  isLocalFileReference,
  localFileBasename,
  resolveFileLink,
  type KnownFilePaths,
} from './artifacts/artifactFileLinks';
import { isOpenableFileLink, useFileLinkExistence } from './artifacts/fileLinkStatus';

const codeCopy = transcriptCopy.code;

interface CodeProps extends React.ClassAttributes<HTMLElement>, React.HTMLAttributes<HTMLElement> {
  inline?: boolean;
  onOpenArtifact?: (artifact: ArtifactSource) => void;
  workingDir?: string;
  knownFilePaths?: KnownFilePaths;
  onRunInTerminal?: (command: string) => boolean;
  variant?: 'chat' | 'document';
}

interface MarkdownContentProps {
  content: string;
  className?: string;
  onOpenArtifact?: (artifact: ArtifactSource) => void;
  workingDir?: string;
  knownFilePaths?: KnownFilePaths;
  /**
   * Offer "Run" on shell code blocks, handing the command to this chat's
   * in-app terminal. OPT-IN, and it has to be: eleven surfaces mount
   * MarkdownContent — the announcement modal, tool-call arguments, the artifact
   * panel, a notebook preview, a workflow warning, a knowledge node — and only
   * one of them is a live chat with a terminal under it. An always-on button
   * would put a shell affordance in a modal.
   *
   * Must be referentially stable: both this component and CodeBlock are memo'd,
   * and a fresh closure each render defeats that for every block in the
   * transcript on every streaming frame.
   */
  onRunInTerminal?: (command: string) => boolean;
  /**
   * `chat` (the default) renders a message; `document` renders a FILE the user
   * opened in the artifact panel — a report, an R Markdown source, a notebook's
   * markdown cell. One switch, because a file differs from a message in three
   * ways that must not drift apart:
   *
   * - Line breaks. Chat keeps `remark-breaks` on purpose: a model's single
   *   newline is a line it meant. Authors hard-wrap a markdown file at 80-100
   *   columns, and turning every wrap into `<br>` rendered the panel's reports
   *   ragged at the source's width. A document follows CommonMark: a single
   *   newline is a space.
   * - Fenced code. Chat soft-wraps a long line to the bubble. A document keeps
   *   the line and scrolls inside its well, as a code viewer does.
   * - The hook. The root carries `data-variant`, for authored CSS that styles a
   *   document without a caller-chosen class name.
   */
  variant?: 'chat' | 'document';
  /**
   * A document whose front matter already shows its title: its first `# H1`
   * takes the h2 step, so the page does not carry two titles (spec 2.1).
   */
  demoteFirstHeading?: boolean;
}

// Memoized CodeBlock component to prevent re-rendering when props haven't changed
const CodeBlock = memo(function CodeBlock({
  language,
  label,
  fenceLanguage,
  children,
  onRunInTerminal,
  wrapLongLines = true,
}: {
  /** The Prism grammar: the fence id, normalised (`normalizeCodeLanguage`). */
  language: string;
  /**
   * The header label: the fence id AS WRITTEN (`r` for ```{r setup}, `Python3`
   * for ```Python3). Both variants show it as authored.
   */
  label: string;
  /**
   * The WHOLE fence identifier, which `language` is not.
   *
   * `language` comes from MarkdownCode's `/language-\{?(\w+)/` and drives the
   * header label and the highlighter; `\w` stops at a hyphen, so a
   * ```shell-session fence arrives there as `shell`. The runnable decision must
   * see `shell-session` — see utils/shellCommandBlock.ts.
   */
  fenceLanguage: string | null;
  children: string;
  onRunInTerminal?: (command: string) => boolean;
  /**
   * Chat soft-wraps a long line to the bubble. A document (`variant="document"`)
   * keeps the line and lets the block scroll sideways, which is what a code
   * viewer does and what keeps indentation honest.
   */
  wrapLongLines?: boolean;
}) {
  /**
   * What the last Copy click achieved. `failed` is shown, never swallowed: a
   * refused write used to reach only `console.error`, so the button went on
   * saying "Copy" and the only way to learn nothing was copied was to paste.
   */
  const [copyOutcome, markCopyOutcome] = useTransientValue<'copied' | 'failed'>(2000);
  const rootRef = useRef<HTMLDivElement>(null);
  /**
   * What the last Run click actually achieved, not merely that one happened.
   *
   * `'sent'` is claimed only when the terminal pane accepted the command. It
   * refuses when its shell has exited, and the bytes then vanish into a closed
   * pty — a case that used to render as a tick and the word "Sent".
   */
  const [runOutcome, setRunOutcome] = useState<'idle' | 'sent' | 'unavailable'>('idle');
  const sentTimeoutRef = useRef<number | null>(null);

  // The shared path (utils/clipboard.ts): a refusal is retried, then copied
  // through the document's own selection, and only then reported as failed.
  const handleCopy = async () => {
    markCopyOutcome((await copyToClipboard(children, rootRef.current)) ? 'copied' : 'failed');
  };

  // Null unless this block is a shell COMMAND the caller can run — see
  // utils/shellCommandBlock.ts for what that excludes and why (transcripts,
  // prose, an empty block, a fence long enough to be a file rather than a
  // command).
  const runnableCommand = useMemo(
    () => (onRunInTerminal ? runnableCommandFromCodeBlock(fenceLanguage, children) : null),
    [onRunInTerminal, fenceLanguage, children]
  );

  const handleRun = () => {
    if (!runnableCommand || !onRunInTerminal) return;
    setRunOutcome(onRunInTerminal(runnableCommand) ? 'sent' : 'unavailable');
    if (sentTimeoutRef.current) window.clearTimeout(sentTimeoutRef.current);
    sentTimeoutRef.current = window.setTimeout(() => setRunOutcome('idle'), 2000);
  };

  useEffect(() => {
    return () => {
      if (sentTimeoutRef.current) window.clearTimeout(sentTimeoutRef.current);
    };
  }, []);

  const codeStyle = codeThemesByFamily[useThemeFamily()][useResolvedTheme()];

  const memoizedSyntaxHighlighter = useMemo(() => {
    return (
      <SyntaxHighlighter
        style={codeStyle}
        language={language}
        PreTag="div"
        customStyle={{
          margin: 0,
          // The body (`.br-md-code-body`) owns the 10px 12px inset, Crew's, so
          // the highlighter adds none of its own.
          padding: 0,
          background: 'transparent',
          // A kept line needs the block to be as wide as its longest line, so
          // the body (`overflow-x: auto`) scrolls it; a wrapped one fits.
          width: wrapLongLines ? '100%' : 'max-content',
          minWidth: '100%',
          maxWidth: wrapLongLines ? '100%' : 'none',
        }}
        codeTagProps={{
          style: {
            ...(wrapLongLines
              ? { whiteSpace: 'pre-wrap', wordBreak: 'break-word', overflowWrap: 'break-word' }
              : { whiteSpace: 'pre' }),
            fontFamily: CODE_FONT_FAMILY,
            fontSize: CODE_FONT_SIZE,
            lineHeight: CODE_LINE_HEIGHT,
          },
        }}
        showLineNumbers={false}
        wrapLines={false}
        lineProps={undefined}
      >
        {children}
      </SyntaxHighlighter>
    );
  }, [codeStyle, language, children, wrapLongLines]);

  const [measureBody, bodyOverflow] = useOverflowsSideways<HTMLDivElement>();

  const copyLabel =
    copyOutcome === 'copied'
      ? codeCopy.copied
      : copyOutcome === 'failed'
        ? codeCopy.copyFailed
        : codeCopy.copy;
  const copyTip =
    copyOutcome === 'copied'
      ? codeCopy.copied
      : copyOutcome === 'failed'
        ? codeCopy.copyFailedTip
        : codeCopy.copyTip;
  const runLabel =
    runOutcome === 'sent'
      ? codeCopy.sent
      : runOutcome === 'unavailable'
        ? codeCopy.terminalClosed
        : codeCopy.run;
  const runTip =
    runOutcome === 'sent'
      ? codeCopy.sentTip
      : runOutcome === 'unavailable'
        ? codeCopy.terminalClosedTip
        : codeCopy.runTip;

  return (
    // Crew's code block (`.br-md-code*`, main.css): the well, a hairline frame,
    // a head row over a hairline on the same ground (no filled slab), and a
    // 10px 12px mono body that fades at its right edge while it scrolls. The
    // syntax palettes are measured against the well (`themes/*.theme.mjs`).
    //
    // `not-prose` is a correction, not decoration. The typography plugin's
    // inline-code rules target every `<code>` under `.prose`, and the
    // highlighter's own `<code>` is one; its element variants skip `.not-prose`
    // subtrees, and nothing inside this block wants them.
    //
    // The `biorouter-md-code*` names stay beside the `br-md-code*` recipe as
    // hooks: the preview's paper and its tests address the block by them.
    <div ref={rootRef} className="br-md-code biorouter-md-code not-prose w-full">
      <div className="br-md-code-head biorouter-md-code-head">
        <span className="br-md-code-lang biorouter-md-code-lang text-supporting text-text-muted select-none">
          {label || codeCopy.plain}
        </span>
        <div className="flex items-center gap-0.5">
          {/* Run sits to the LEFT so Copy keeps the position it has always had.
              Copy is never REPLACED: this feature is the step
              CodingAgentInlineCard deliberately stopped short of ("a command
              the user runs, never one Biorouter runs"), and the old path has to
              survive it. Nothing else here is a keyboard shortcut either: a
              deliberate click is the whole consent. Both answer a press in
              their tooltip, so neither changes width. */}
          {runnableCommand && (
            <TranscriptIconButton
              label={runLabel}
              tip={runTip}
              holdTip={runOutcome !== 'idle'}
              tone={runOutcome === 'unavailable' ? 'warning' : 'default'}
              onClick={handleRun}
              icon={
                runOutcome === 'sent' ? (
                  <Check aria-hidden="true" />
                ) : runOutcome === 'unavailable' ? (
                  <AlertTriangle aria-hidden="true" />
                ) : (
                  <Play aria-hidden="true" />
                )
              }
            />
          )}
          <TranscriptIconButton
            label={copyLabel}
            tip={copyTip}
            holdTip={copyOutcome !== null}
            tone={copyOutcome === 'failed' ? 'warning' : 'default'}
            onClick={handleCopy}
            icon={
              copyOutcome === 'copied' ? (
                <Check aria-hidden="true" />
              ) : copyOutcome === 'failed' ? (
                <AlertTriangle aria-hidden="true" />
              ) : (
                <Copy aria-hidden="true" />
              )
            }
          />
        </div>
      </div>
      {/* A region a keyboard can reach only while it scrolls (Crew's rule), and
          the right-edge fade while there is more to its right. */}
      <div
        ref={measureBody}
        className="br-md-code-body biorouter-md-code-body w-full"
        {...scrollRegionProps(bodyOverflow, codeCopy.region)}
      >
        {memoizedSyntaxHighlighter}
      </div>
    </div>
  );
});

const LOOPBACK_URL_RE =
  /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])(?::\d+)?(?:[/?#]|$)/i;

function artifactAwareUrlTransform(value: string) {
  if (isLocalFileReference(value) && looksLikePreviewableFile(value)) {
    return value;
  }
  return defaultUrlTransform(value);
}

function previewableExternalUrl(href: string): string | null {
  try {
    return normalizeExternalHttpUrl(href);
  } catch {
    return null;
  }
}

function artifactSourceFromMarkdownValue(
  value: string,
  workingDir?: string,
  knownFilePaths?: KnownFilePaths
): ArtifactSource | null {
  const candidate = value.trim();
  if (!candidate || candidate.includes('\n') || candidate.includes('\r')) return null;
  if (LOOPBACK_URL_RE.test(candidate)) {
    return { kind: 'externalUrl', title: candidate, url: candidate };
  }
  if (!looksLikePreviewableFile(candidate)) return null;
  const resolved = resolveFileLink(candidate, workingDir, knownFilePaths);
  if (resolved.kind === 'unresolved') return null;
  return {
    kind: 'file',
    title: localFileBasename(resolved.path),
    path: resolved.path,
    ...(resolved.line ? { line: resolved.line } : {}),
  };
}

// The ONE link treatment in this renderer (design spec "The markdown layer,
// rebuilt"). Plain `<a>` gets it from the typography plugin, whose
// `--tw-prose-links` main.css points at `--text-accent`; the two <button>-based
// links below are not `<a>`, so the plugin cannot reach them and they restate it
// here. Previously these were three different treatments (plugin accent,
// `decoration-border-strong` with default ink, and accent-with-neutral-underline).
const LINK_CLASS =
  'cursor-pointer font-medium text-text-accent underline decoration-text-accent/40 underline-offset-2 transition-colors hover:decoration-text-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus';

// Both variants are mono at 13px — the same size as a fenced block
// (CODE_FONT_SIZE) and inline code. The sole difference is the inline-code
// fill, which is the only thing `inlineCode` should mean; the two variants
// used to also disagree on font-size (0.9em vs 0.95em) for no stated reason.
const ARTIFACT_LINK_BASE_CLASS = 'inline break-all rounded-inner text-left font-mono text-code';

function ArtifactLinkButton({
  artifact,
  children,
  onOpenArtifact,
  inlineCode = false,
  workingDir,
}: {
  artifact: ArtifactSource;
  children: React.ReactNode;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  inlineCode?: boolean;
  workingDir?: string;
}) {
  // Only a file has an existence to check. An external URL reports `unchecked`
  // and keeps the link treatment it has always had.
  const existence = useFileLinkExistence(
    artifact.kind === 'file' ? artifact.path : null,
    workingDir
  );

  // A path the assistant only *named* — a script it described, a `/tmp` tree
  // that has since been cleaned up — is not a destination, so it stops looking
  // like one: no accent ink, no underline, no pointer, no focus stop, and not a
  // <button> at all. It is DECOLORED to `text-text-default` rather than muted;
  // the ask was that it read as ordinary prose, not that it be de-emphasised.
  //
  // This is also the state a link starts in whenever the check is available, so
  // a dead path is never clickable for even one frame — the link treatment is
  // an upgrade applied once existence is confirmed, never a default walked back.
  if (!isOpenableFileLink(existence)) {
    return inlineCode ? (
      // Same fill, padding, family and size as the inline-code recipe, so the
      // text is unchanged and only its role is.
      <span
        className={`${ARTIFACT_LINK_BASE_CLASS} biorouter-inline-code bg-background-medium px-1 py-0.5 text-text-default`}
      >
        {children}
      </span>
    ) : (
      // Prose keeps its own family: an unlinkable path in a sentence already
      // renders as a plain string when `resolveFileLink` refuses it (see
      // `linkifyFilePaths`), and this matches that precedent exactly.
      <span className="break-all text-text-default">{children}</span>
    );
  }

  return (
    <button
      type="button"
      className={`${ARTIFACT_LINK_BASE_CLASS} ${LINK_CLASS} ${
        inlineCode ? 'biorouter-inline-code bg-background-medium px-1 py-0.5' : ''
      }`}
      onClick={() => onOpenArtifact(artifact)}
      title={codeCopy.previewInPanel(artifact.title)}
    >
      {children}
    </button>
  );
}

// A local image whose bytes could not be read (denied by the main-process
// allowlist, missing, or not an image) and a remote image that failed to load
// both collapse to this inline placeholder instead of a dead <img> with a
// busted src. The alt text stays legible so the reader still knows what was
// meant to be here.
function BrokenImage({ alt }: { alt?: string }) {
  const label = alt?.trim() || codeCopy.imageUnavailable;
  return (
    <span
      role="img"
      aria-label={label}
      title={alt?.trim() ? codeCopy.imageUnavailableNamed(alt) : codeCopy.imageUnavailable}
      className="inline-flex items-center gap-1 rounded-inner border border-border-subtle bg-background-medium px-1.5 py-0.5 align-middle text-supporting text-text-muted"
    >
      <ImageIcon className="size-3.5 shrink-0" aria-hidden="true" />
      {label}
    </span>
  );
}

// Markdown images in the preview. Remote (`http(s)`/`data:`) srcs render
// directly — the same reach chat already has. A LOCAL image (relative to the
// previewed file, absolute, `~`, or `file://`) can't be loaded by the renderer
// from disk, so it is read through the existing allowlisted `readArtifactFile`
// IPC and inlined as a `data:` URI (CSP-safe). Anything the allowlist denies, or
// that traverses out of the file's directory, degrades to `BrokenImage`.
const MarkdownImage = memo(function MarkdownImage({
  src,
  alt,
  workingDir,
}: {
  src?: string;
  alt?: string;
  workingDir?: string;
}) {
  const source = useMemo(
    () => resolveMarkdownImageSource(src ?? '', workingDir),
    [src, workingDir]
  );
  const [resolvedSrc, setResolvedSrc] = useState<string | null>(
    source.kind === 'remote' ? source.url : null
  );
  const [failed, setFailed] = useState(source.kind === 'blocked');

  useEffect(() => {
    if (source.kind === 'remote') {
      setResolvedSrc(source.url);
      setFailed(false);
      return;
    }
    if (source.kind === 'blocked') {
      setResolvedSrc(null);
      setFailed(true);
      return;
    }

    let cancelled = false;
    // A large image arrives as bytes and becomes a `blob:` URL that has to be
    // revoked, so the cleanup below owns whatever this effect minted.
    let revokeSrc: (() => void) | null = null;
    setResolvedSrc(null);
    setFailed(false);
    const read = window.electron?.readArtifactFile;
    if (!read) {
      setFailed(true);
      return;
    }
    void read(source.path)
      .then((preview: ArtifactFilePreview) => {
        if (cancelled) return;
        if (preview && preview.kind === 'image') {
          const { src, revoke } = imageSourceForPreview(preview);
          if (!src) {
            setFailed(true);
            revoke();
            return;
          }
          revokeSrc = revoke;
          setResolvedSrc(src);
        } else {
          setFailed(true);
        }
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
      revokeSrc?.();
    };
  }, [source]);

  if (failed) return <BrokenImage alt={alt} />;
  if (!resolvedSrc) {
    return (
      <span
        aria-label={alt?.trim() || codeCopy.loadingImage}
        className="inline-block h-4 w-24 animate-pulse rounded-inner bg-background-medium align-middle"
      />
    );
  }
  return (
    <img
      src={resolvedSrc}
      alt={alt ?? ''}
      className="mx-auto my-2 h-auto max-w-full rounded-element"
      onError={() => setFailed(true)}
    />
  );
});

// External links open in the SYSTEM browser through the existing IPC — never by
// navigating the renderer/panel (a top-frame navigation would drop the CSP and
// keep the preload bridge). target=_blank alone leans on the main process's
// window-open handler; calling openExternal here makes it explicit and testable.
function openExternalLink(event: React.MouseEvent<HTMLAnchorElement>, href: string) {
  if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
    return;
  }
  const opener = window.electron?.openExternal;
  if (!opener) return;
  event.preventDefault();
  void opener(href);
}

// Inside a markdown link, the link IS the destination. Inline code in its text
// (`[\`results.csv\`](results.csv)`) used to become a second file-link button
// nested inside the link's own button — invalid HTML that React warns about, and
// two click targets for one link. Code under a link renders as plain inline code.
const InsideLinkContext = createContext(false);
const InsideCodeBlockContext = createContext(false);

const MarkdownCode = memo(
  React.forwardRef(function MarkdownCode(
    {
      inline,
      className,
      children,
      onOpenArtifact,
      workingDir,
      knownFilePaths,
      onRunInTerminal,
      variant,
      ...props
    }: CodeProps,
    ref: React.Ref<HTMLElement>
  ) {
    // `\{?` admits R Markdown / Quarto chunk headers (```{r setup}), which
    // reach here as `language-{r`; `\w` alone rejected them, so every chunk
    // rendered as an unhighlighted plain block. The name is normalised (case,
    // kernel aliases) before it reaches Prism, whose registry is case-sensitive.
    const match = /language-\{?(\w+)/.exec(className || '');
    // The same identifier, unabridged. `\w` stops at a hyphen, so `match[1]` is
    // `shell` for BOTH ```shell and ```shell-session — fine for a header label
    // and a Prism alias, wrong for deciding whether a block may be executed,
    // since the second one's body is a prompt character followed by output.
    // Kept as a separate read so highlighting and the header keep byte-identical
    // behaviour.
    const fenceMatch = /language-([\w.+-]+)/.exec(className || '');
    const text = String(children);
    const insideLink = useContext(InsideLinkContext);
    const insideCodeBlock = useContext(InsideCodeBlockContext);
    const artifact =
      !match && !insideLink
        ? artifactSourceFromMarkdownValue(text, workingDir, knownFilePaths)
        : null;
    return !inline && (insideCodeBlock || match) ? (
      <CodeBlock
        language={match ? normalizeCodeLanguage(match[1]) : 'text'}
        label={match ? match[1] : 'text'}
        fenceLanguage={fenceMatch ? fenceMatch[1] : null}
        onRunInTerminal={onRunInTerminal}
        wrapLongLines={variant !== 'document'}
      >
        {text.replace(/\n$/, '')}
      </CodeBlock>
    ) : artifact && onOpenArtifact ? (
      <ArtifactLinkButton
        artifact={artifact}
        onOpenArtifact={onOpenArtifact}
        workingDir={workingDir}
        inlineCode
      >
        {children}
      </ArtifactLinkButton>
    ) : (
      // Fill/size/padding come from the `prose-code:*` list on the wrapper —
      // the single inline-code recipe. This used to also carry `bg-inline-code`,
      // a second, competing fill that only won via a specificity ladder in
      // main.css.
      <code
        ref={ref}
        {...props}
        className="biorouter-inline-code break-all whitespace-pre-wrap font-mono"
      >
        {children}
      </code>
    );
  })
);

// File paths the assistant mentions in prose aren't markdown links, so
// ReactMarkdown renders them as plain text. Keep the match narrow: an absolute,
// home-relative, or multi-segment relative path with a filename extension.
const FILE_PATH_RE =
  /(?<![^\s([{])((?:file:\/\/|~\/|\/|[A-Za-z]:[\\/]|\\\\[\p{L}\p{N}\p{M}_.\-+@%$]+\\[\p{L}\p{N}\p{M}_.\-+@%$]+\\|(?:[\p{L}\p{N}\p{M}.\-+@%]+[\\/])+)[^\s)\]}\x60"'<>]*\.[A-Za-z0-9]{1,12}(?::\d+|#L\d+|%[^\s)\]}\x60"'<>.,!?;]*)?)(?=$|[\s)\]},;]|[.!?](?=$|[\s)\]},;]))/gu;

function linkifyFilePaths(
  children: React.ReactNode,
  onOpenArtifact?: (artifact: ArtifactSource) => void,
  workingDir?: string,
  knownFilePaths?: KnownFilePaths
): React.ReactNode {
  if (!onOpenArtifact) return children;
  return React.Children.map(children, (child) => {
    if (typeof child !== 'string' || !/[\\/]/.test(child)) return child;
    const out: React.ReactNode[] = [];
    let last = 0;
    let match: RegExpExecArray | null;
    FILE_PATH_RE.lastIndex = 0;
    while ((match = FILE_PATH_RE.exec(child)) !== null) {
      const filePath = match[1];
      if (match.index > last) out.push(child.slice(last, match.index));
      const artifact = artifactSourceFromMarkdownValue(filePath, workingDir, knownFilePaths);
      if (!artifact) {
        out.push(filePath);
        last = match.index + filePath.length;
        continue;
      }
      out.push(
        <ArtifactLinkButton
          key={match.index}
          artifact={artifact}
          onOpenArtifact={onOpenArtifact}
          workingDir={workingDir}
        >
          {filePath}
        </ArtifactLinkButton>
      );
      last = match.index + filePath.length;
    }
    if (last === 0) return child;
    if (last < child.length) out.push(child.slice(last));
    return out;
  });
}

const MarkdownParagraph = ({
  children,
  onOpenArtifact,
  workingDir,
  knownFilePaths,
  ...props
}: React.HTMLAttributes<globalThis.HTMLParagraphElement> & {
  onOpenArtifact?: (artifact: ArtifactSource) => void;
  workingDir?: string;
  knownFilePaths?: KnownFilePaths;
}) => {
  const childArray = React.Children.toArray(children);
  const meaningfulChildren = childArray.filter(
    (child) => !(typeof child === 'string' && child.trim() === '')
  );
  const isDisplayMath =
    meaningfulChildren.length === 1 &&
    React.isValidElement(meaningfulChildren[0]) &&
    typeof (meaningfulChildren[0] as React.ReactElement<{ className?: string }>).props
      ?.className === 'string' &&
    (meaningfulChildren[0] as React.ReactElement<{ className?: string }>).props.className!.includes(
      'katex'
    );
  // Centring, margin and overflow for math live in ONE place: the
  // `.katex-display` block in main.css. This wrapper used to restate all three
  // as `flex justify-center my-3 overflow-x-auto`.
  //
  // Note it never actually reached *display* math: remark-math emits `$$…$$` as
  // a block sibling, so `.katex-display` is a direct child of the prose root and
  // is never inside a <p>. The duplicate styling only ever landed on a paragraph
  // holding nothing but INLINE math — which `.katex-display` does not style, so
  // the flex/margin were wrong there too. What this branch is genuinely for is
  // keeping `linkifyFilePaths` off KaTeX's output.
  if (isDisplayMath) {
    return <p {...props}>{children}</p>;
  }
  return <p {...props}>{linkifyFilePaths(children, onOpenArtifact, workingDir, knownFilePaths)}</p>;
};

// Module-level so ReactMarkdown sees the same array identity on every render.
// Chat (`variant="chat"`) keeps `remark-breaks`; a document does not.
const HARD_BREAK_REMARK_PLUGINS: NonNullable<Options['remarkPlugins']> = [
  remarkGfm,
  remarkBreaks,
  [remarkMath, { singleDollarTextMath: false }],
];
const SOFT_BREAK_REMARK_PLUGINS: NonNullable<Options['remarkPlugins']> = [
  remarkGfm,
  [remarkMath, { singleDollarTextMath: false }],
];

const REHYPE_PLUGINS: NonNullable<Options['rehypePlugins']> = [
  [
    rehypeKatex,
    {
      throwOnError: false,
      // KaTeX takes a raw colour string, not a CSS var. Keep it in step
      // with --text-danger (light).
      errorColor: '#b3261e',
      strict: false,
    },
  ],
];

/**
 * What the element renderers below need from the MarkdownContent that mounted
 * them. It travels by context, not by closure, so the renderers can be defined
 * ONCE, at module scope — see `MARKDOWN_COMPONENTS`.
 */
interface MarkdownRenderOptions {
  onOpenArtifact?: (artifact: ArtifactSource) => void;
  workingDir?: string;
  knownFilePaths?: KnownFilePaths;
  onRunInTerminal?: (command: string) => boolean;
  variant: 'chat' | 'document';
}

const MarkdownRenderContext = createContext<MarkdownRenderOptions>({ variant: 'chat' });

type SlotProps<Tag extends keyof React.JSX.IntrinsicElements> = React.JSX.IntrinsicElements[Tag] &
  ExtraProps;

function MarkdownAnchor({ href, children: linkChildren, node: _node, ...props }: SlotProps<'a'>) {
  const { onOpenArtifact, workingDir, knownFilePaths } = useContext(MarkdownRenderContext);
  if (!href) return <>{linkChildren}</>;
  const children = (
    <InsideLinkContext.Provider value={true}>{linkChildren}</InsideLinkContext.Provider>
  );
  if (isLocalFileReference(href)) {
    // A link to a sibling/local file. If there is a panel to open it in,
    // preview it there; otherwise render it as styled, inert text with a
    // tooltip rather than an <a> that would dead-navigate the renderer.
    const resolved = resolveFileLink(href, workingDir, knownFilePaths);
    if (onOpenArtifact && looksLikePreviewableFile(href) && resolved.kind === 'resolved') {
      return (
        <ArtifactLinkButton
          artifact={{
            kind: 'file',
            title: localFileBasename(resolved.path),
            path: resolved.path,
            ...(resolved.line ? { line: resolved.line } : {}),
          }}
          onOpenArtifact={onOpenArtifact}
          workingDir={workingDir}
        >
          {children}
        </ArtifactLinkButton>
      );
    }
    return (
      <span
        className="cursor-default font-medium text-text-muted underline decoration-dotted decoration-text-muted/40 underline-offset-2"
        title={resolved.kind === 'unresolved' ? resolved.reason : resolved.path}
      >
        {children}
      </span>
    );
  }
  const externalUrl = previewableExternalUrl(href);
  if (externalUrl && onOpenArtifact) {
    return (
      <button
        type="button"
        className={`inline break-all text-left ${LINK_CLASS}`}
        onClick={() => onOpenArtifact({ kind: 'externalUrl', title: href, url: externalUrl })}
      >
        {children}
      </button>
    );
  }
  return (
    <a
      href={href}
      {...props}
      target="_blank"
      rel="noopener noreferrer"
      onClick={(event) => openExternalLink(event, href)}
    >
      {children}
    </a>
  );
}

function MarkdownImageSlot({ src, alt }: SlotProps<'img'>) {
  const { workingDir } = useContext(MarkdownRenderContext);
  return (
    <MarkdownImage
      src={typeof src === 'string' ? src : undefined}
      alt={typeof alt === 'string' ? alt : undefined}
      workingDir={workingDir}
    />
  );
}

function MarkdownPre({ children }: SlotProps<'pre'>) {
  return (
    <InsideCodeBlockContext.Provider value={true}>
      <div className="biorouter-md-pre">{children}</div>
    </InsideCodeBlockContext.Provider>
  );
}

function MarkdownCodeSlot({ node: _node, ...props }: SlotProps<'code'>) {
  const { onOpenArtifact, workingDir, knownFilePaths, onRunInTerminal, variant } =
    useContext(MarkdownRenderContext);
  return (
    <MarkdownCode
      {...props}
      onOpenArtifact={onOpenArtifact}
      workingDir={workingDir}
      knownFilePaths={knownFilePaths}
      onRunInTerminal={onRunInTerminal}
      variant={variant}
    />
  );
}

function MarkdownParagraphSlot({ node: _node, ...props }: SlotProps<'p'>) {
  const { onOpenArtifact, workingDir, knownFilePaths } = useContext(MarkdownRenderContext);
  return (
    <MarkdownParagraph
      {...props}
      onOpenArtifact={onOpenArtifact}
      workingDir={workingDir}
      knownFilePaths={knownFilePaths}
    />
  );
}

function MarkdownListItem({ children, node: _node, ...props }: SlotProps<'li'>) {
  const { onOpenArtifact, workingDir, knownFilePaths } = useContext(MarkdownRenderContext);
  return (
    <li {...props}>{linkifyFilePaths(children, onOpenArtifact, workingDir, knownFilePaths)}</li>
  );
}

/**
 * Crew's table (`.br-md-table`, main.css): a hairline frame at radius 8, top
 * rules only, a muted header row, 13px tabular cells. The scroll box is a
 * region a keyboard can reach only while the table is wider than the column,
 * and fades at its right edge while there is more to see.
 */
function MarkdownTable({ node: _node, className, ...props }: SlotProps<'table'>) {
  const [measure, overflow] = useOverflowsSideways<HTMLDivElement>();
  return (
    <div
      ref={measure}
      className="br-md-table-scroll biorouter-md-table-scroll"
      {...scrollRegionProps(overflow, codeCopy.tableRegion)}
    >
      <table {...props} className={['br-md-table', className].filter(Boolean).join(' ')} />
    </div>
  );
}

function MarkdownTableCell({ children, node: _node, ...props }: SlotProps<'td'>) {
  const { onOpenArtifact, workingDir, knownFilePaths } = useContext(MarkdownRenderContext);
  return (
    <td {...props}>{linkifyFilePaths(children, onOpenArtifact, workingDir, knownFilePaths)}</td>
  );
}

function MarkdownTableHeader({ children, node: _node, ...props }: SlotProps<'th'>) {
  const { onOpenArtifact, workingDir, knownFilePaths } = useContext(MarkdownRenderContext);
  return (
    <th {...props}>{linkifyFilePaths(children, onOpenArtifact, workingDir, knownFilePaths)}</th>
  );
}

/**
 * The element renderers, defined ONCE. ⚠ Never build this map inline in
 * MarkdownContent's render.
 *
 * ReactMarkdown uses each entry as a component TYPE. An inline map made a new
 * `pre` and `code` function on every render, and React unmounts a subtree whose
 * type changed, so every fenced block in the transcript was torn down and
 * rebuilt each time its MarkdownContent re-rendered. During a streamed reply
 * that was every 50-100 ms for every message (a fresh `knownFilePaths` per
 * chunk defeated the memo): a click pressed on one Copy button and released on
 * its replacement, so no `click` fired and nothing was copied, and a "Copied"
 * that did land was thrown away with its button. Measured live: 13 of 15
 * clicks on a finished message's Copy failed while the next reply streamed.
 *
 * What the renderers need from their MarkdownContent reaches them through
 * `MarkdownRenderContext`, which changes a consumer's props, never its type.
 */
const MARKDOWN_COMPONENTS: Components = {
  a: MarkdownAnchor,
  img: MarkdownImageSlot,
  pre: MarkdownPre,
  code: MarkdownCodeSlot,
  p: MarkdownParagraphSlot,
  li: MarkdownListItem,
  table: MarkdownTable,
  td: MarkdownTableCell,
  th: MarkdownTableHeader,
};

const MarkdownContent = memo(function MarkdownContent({
  content,
  className = '',
  onOpenArtifact,
  workingDir,
  knownFilePaths,
  onRunInTerminal,
  variant = 'chat',
  demoteFirstHeading = false,
}: MarkdownContentProps) {
  const [processedContent, setProcessedContent] = useState(content);

  useEffect(() => {
    try {
      const processed = wrapHTMLInCodeBlock(content);
      setProcessedContent(processed);
    } catch (error) {
      console.error('Error processing content:', error);
      setProcessedContent(content);
    }
  }, [content]);

  const renderOptions = useMemo<MarkdownRenderOptions>(
    () => ({ onOpenArtifact, workingDir, knownFilePaths, onRunInTerminal, variant }),
    [onOpenArtifact, workingDir, knownFilePaths, onRunInTerminal, variant]
  );

  return (
    <div
      data-variant={variant}
      // The inline-code chip, the table and the heading steps are authored in
      // main.css (`.biorouter-markdown*`, `.br-md-*`), not as `prose-*:`
      // utilities here: a size that depends on Tailwind having scanned a new
      // class string can silently not exist (CLAUDE.md, BIOROUTER_NO_HMR).
      data-demote-h1={demoteFirstHeading ? '' : undefined}
      className={`biorouter-markdown w-full min-w-0 overflow-x-hidden prose prose-sm text-text-default dark:prose-invert max-w-full word-break font-sans
      prose-pre:p-0 prose-pre:m-0 prose-pre:bg-transparent prose-pre:rounded-none !p-0
      prose-pre:[&:has(>code)]:p-3 prose-pre:[&>code]:p-0
      prose-code:break-words prose-code:whitespace-pre-wrap prose-code:font-mono
      prose-code:text-text-default prose-code:font-normal prose-code:not-italic
      prose-code:before:content-none prose-code:after:content-none
      prose-a:break-all prose-a:font-medium prose-a:underline
      prose-a:decoration-text-accent/40 prose-a:underline-offset-2
      [&_blockquote_p:first-of-type]:before:content-none
      [&_blockquote_p:last-of-type]:after:content-none ${className}`}
    >
      <MarkdownRenderContext.Provider value={renderOptions}>
        <ReactMarkdown
          urlTransform={artifactAwareUrlTransform}
          remarkPlugins={
            variant === 'document' ? SOFT_BREAK_REMARK_PLUGINS : HARD_BREAK_REMARK_PLUGINS
          }
          rehypePlugins={REHYPE_PLUGINS}
          components={MARKDOWN_COMPONENTS}
        >
          {processedContent}
        </ReactMarkdown>
      </MarkdownRenderContext.Provider>
    </div>
  );
});

export default MarkdownContent;
