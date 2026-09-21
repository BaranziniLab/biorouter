import React, {
  useState,
  useEffect,
  useRef,
  memo,
  useMemo,
  createContext,
  useContext,
} from 'react';
import ReactMarkdown, { defaultUrlTransform, type Options } from 'react-markdown';
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
import { Button } from './ui/button';

import { AlertTriangle, Check, Copy, Image as ImageIcon, Play } from './icons/app-icons';
import { wrapHTMLInCodeBlock } from '../utils/htmlSecurity';
import { normalizeExternalHttpUrl } from '../utils/externalUrl';
import { runnableCommandFromCodeBlock } from '../utils/shellCommandBlock';
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
  const [copied, setCopied] = useState(false);
  /**
   * What the last Run click actually achieved, not merely that one happened.
   *
   * `'sent'` is claimed only when the terminal pane accepted the command. It
   * refuses when its shell has exited, and the bytes then vanish into a closed
   * pty — a case that used to render as a tick and the word "Sent".
   */
  const [runOutcome, setRunOutcome] = useState<'idle' | 'sent' | 'unavailable'>('idle');
  const timeoutRef = useRef<number | null>(null);
  const sentTimeoutRef = useRef<number | null>(null);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(children);
      setCopied(true);
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy text: ', err);
    }
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
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
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
          // Overridable, so a surface that restyles the block (the artifact
          // panel's paper) can set its own inset without an !important fight
          // with an inline style. Chat never sets it and keeps 12px.
          padding: 'var(--md-code-pad, 12px)',
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

  return (
    // `bg-background-code`, not `bg-background-muted`: the syntax palette in
    // codeTheme.ts is verified against --background-code (#f5f5f3 / #1b1b19,
    // design.md §5.1) and --background-muted is a different surface (#f4f4f2 /
    // #232320) — so painting muted would put dark code on a ground its palette
    // was never measured on, which is how `comment` once fell to 4.15:1, under
    // AA. The two tokens are close now that the neutrals are shared, but they
    // are still distinct and the generator measures against the code one.
    // The highlighter itself renders transparent, so this div IS the ground the
    // reader sees.
    //
    // `not-prose` is a correction, not decoration. The wrapper's inline-code
    // recipe (`prose-code:bg-background-medium px-1 py-0.5 rounded-sm`) targets
    // every `<code>` under `.prose`, and the highlighter's own `<code>` is one —
    // so each line of every fenced block painted an inline-code chip and the
    // first line sat 4px right of the rest. The typography plugin's element
    // variants skip `.not-prose` subtrees; nothing inside this block wants them.
    //
    // The `biorouter-md-code*` names are hooks for surfaces that restyle the
    // block in authored CSS (the artifact panel's paper, main.css).
    <div className="biorouter-md-code not-prose w-full overflow-hidden bg-background-code">
      {/* Header bar */}
      <div className="biorouter-md-code-head flex items-center justify-between">
        <span className="biorouter-md-code-lang text-[11px] font-medium text-text-muted select-none">
          {label || 'code'}
        </span>
        <div className="flex items-center gap-1">
          {/* Run sits to the LEFT so Copy keeps the position it has always had.
              Copy is never REPLACED: this feature is the step
              CodingAgentInlineCard deliberately stopped short of ("a command
              the user runs, never one Biorouter runs"), and the old path has to
              survive it. Nothing else here is a keyboard shortcut either — a
              deliberate click is the whole consent. */}
          {runnableCommand && (
            <Button
              variant="ghost"
              size="xs"
              onClick={handleRun}
              className="gap-1 text-[11px] text-text-muted hover:text-text-default"
              title={
                runOutcome === 'unavailable'
                  ? 'The terminal below has exited, so nothing was sent'
                  : 'Run in the terminal below'
              }
            >
              {runOutcome === 'sent' ? (
                <Check className="h-3 w-3" />
              ) : runOutcome === 'unavailable' ? (
                <AlertTriangle className="h-3 w-3" />
              ) : (
                <Play className="h-3 w-3" />
              )}
              <span>
                {runOutcome === 'sent'
                  ? 'Sent'
                  : runOutcome === 'unavailable'
                    ? 'Terminal closed'
                    : 'Run'}
              </span>
            </Button>
          )}
          <Button
            variant="ghost"
            size="xs"
            onClick={handleCopy}
            className="gap-1 text-[11px] text-text-muted hover:text-text-default"
            title="Copy code"
          >
            {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
            <span>{copied ? 'Copied' : 'Copy'}</span>
          </Button>
        </div>
      </div>
      {/* Code body */}
      <div
        className="biorouter-md-code-body w-full overflow-x-auto"
        tabIndex={wrapLongLines ? undefined : 0}
        role={wrapLongLines ? undefined : 'region'}
        aria-label={wrapLongLines ? undefined : 'Scrollable code'}
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
const ARTIFACT_LINK_BASE_CLASS = 'inline break-all rounded-sm text-left font-mono text-[13px]';

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
      title={`Preview ${artifact.title} in the side panel`}
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
  const label = alt?.trim() || 'Image unavailable';
  return (
    <span
      role="img"
      aria-label={label}
      title={alt?.trim() ? `Image unavailable: ${alt}` : 'Image unavailable'}
      className="inline-flex items-center gap-1 rounded-sm border border-border-subtle bg-background-medium px-1.5 py-0.5 align-middle text-[12px] text-text-muted"
    >
      <ImageIcon className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
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
        aria-label={alt?.trim() || 'Loading image'}
        className="inline-block h-4 w-24 animate-pulse rounded-sm bg-background-medium align-middle"
      />
    );
  }
  return (
    <img
      src={resolvedSrc}
      alt={alt ?? ''}
      className="mx-auto my-2 h-auto max-w-full rounded-md"
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

const MarkdownContent = memo(function MarkdownContent({
  content,
  className = '',
  onOpenArtifact,
  workingDir,
  knownFilePaths,
  onRunInTerminal,
  variant = 'chat',
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

  return (
    <div
      data-variant={variant}
      className={`biorouter-markdown w-full min-w-0 overflow-x-hidden prose prose-sm text-text-default dark:prose-invert max-w-full word-break font-sans
      prose-pre:p-0 prose-pre:m-0 prose-pre:bg-transparent prose-pre:rounded-none !p-0
      prose-pre:[&:has(>code)]:p-3 prose-pre:[&>code]:p-0
      prose-code:break-words prose-code:whitespace-pre-wrap prose-code:font-mono
      prose-code:text-text-default prose-code:bg-background-medium
      prose-code:rounded-sm prose-code:px-1 prose-code:py-0.5
      prose-code:text-[13px] prose-code:font-normal prose-code:not-italic
      prose-code:before:content-none prose-code:after:content-none
      prose-a:break-all prose-a:font-medium prose-a:underline
      prose-a:decoration-text-accent/40 prose-a:underline-offset-2
      prose-table:table prose-table:w-full prose-table:text-[13px]
      prose-th:tabular-nums prose-td:tabular-nums
      [&_blockquote_p:first-of-type]:before:content-none
      [&_blockquote_p:last-of-type]:after:content-none ${className}`}
    >
      <ReactMarkdown
        urlTransform={artifactAwareUrlTransform}
        remarkPlugins={
          variant === 'document' ? SOFT_BREAK_REMARK_PLUGINS : HARD_BREAK_REMARK_PLUGINS
        }
        rehypePlugins={[
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
        ]}
        components={{
          a: ({ href, children: linkChildren, node: _node, ...props }) => {
            if (!href) return <>{linkChildren}</>;
            const children = (
              <InsideLinkContext.Provider value={true}>{linkChildren}</InsideLinkContext.Provider>
            );
            if (isLocalFileReference(href)) {
              // A link to a sibling/local file. If there is a panel to open it in,
              // preview it there; otherwise render it as styled, inert text with a
              // tooltip rather than an <a> that would dead-navigate the renderer.
              const resolved = resolveFileLink(href, workingDir, knownFilePaths);
              if (
                onOpenArtifact &&
                looksLikePreviewableFile(href) &&
                resolved.kind === 'resolved'
              ) {
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
                  onClick={() =>
                    onOpenArtifact({ kind: 'externalUrl', title: href, url: externalUrl })
                  }
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
          },
          img: ({ src, alt, node: _node }) => (
            <MarkdownImage
              src={typeof src === 'string' ? src : undefined}
              alt={typeof alt === 'string' ? alt : undefined}
              workingDir={workingDir}
            />
          ),
          pre: ({ children }) => (
            <InsideCodeBlockContext.Provider value={true}>
              <div className="biorouter-md-pre">{children}</div>
            </InsideCodeBlockContext.Provider>
          ),
          code: ({ node: _node, ...props }) => (
            <MarkdownCode
              {...props}
              onOpenArtifact={onOpenArtifact}
              workingDir={workingDir}
              knownFilePaths={knownFilePaths}
              onRunInTerminal={onRunInTerminal}
              variant={variant}
            />
          ),
          p: ({ node: _node, ...props }) => (
            <MarkdownParagraph
              {...props}
              onOpenArtifact={onOpenArtifact}
              workingDir={workingDir}
              knownFilePaths={knownFilePaths}
            />
          ),
          li: ({ children, node: _node, ...props }) => (
            <li {...props}>
              {linkifyFilePaths(children, onOpenArtifact, workingDir, knownFilePaths)}
            </li>
          ),
          table: ({ node: _node, ...props }) => (
            <div
              className="biorouter-md-table-scroll"
              role="region"
              aria-label="Scrollable table"
              tabIndex={0}
            >
              <table {...props} />
            </div>
          ),
          td: ({ children, node: _node, ...props }) => (
            <td {...props}>
              {linkifyFilePaths(children, onOpenArtifact, workingDir, knownFilePaths)}
            </td>
          ),
          th: ({ children, node: _node, ...props }) => (
            <th {...props}>
              {linkifyFilePaths(children, onOpenArtifact, workingDir, knownFilePaths)}
            </th>
          ),
        }}
      >
        {processedContent}
      </ReactMarkdown>
    </div>
  );
});

export default MarkdownContent;
