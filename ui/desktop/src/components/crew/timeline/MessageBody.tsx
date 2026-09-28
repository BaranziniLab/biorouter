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
import { revealHiddenCharacters, stripHiddenCharacters } from '../../../utils/untrustedText';
import { bodyNodeText, rehypeCrewBodyText } from './bodyText';
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
 * - **A link is a public http or https address**, opened in the system browser
 *   after the desktop's own confirmation. Any other scheme (and a relative path,
 *   which here could only name the viewer's own files) renders as plain text. So
 *   does a mailto link, or an address the desktop never opens (a literal IP,
 *   localhost, a name that is never public): text with its address beside it,
 *   rather than a link whose click does nothing ({@link openableHref}).
 * - **Raw HTML is text.** react-markdown turns an HTML node into a text node
 *   unless `rehype-raw` is installed, and it is not — so `<svg onload>` or
 *   `<script>` shows as the characters typed. The body step (`bodyText.ts`)
 *   makes that text first, so its hidden characters are shown like any other's.
 * - **No math, no syntax highlighting, no "Run".** Math would bring KaTeX's
 *   `\href`; highlighting is decoration; running a teammate's command is a
 *   decision for a terminal, not a click.
 * - **Headings are bold lines, not `<h1>`–`<h6>`**: a message's `# Title` must
 *   not become a heading of the page, beside the channel's own `<h1>`.
 * - **Nothing hidden reorders or disguises the words** (QA M3, SEC-9): a bidi
 *   override, an isolate, a control character or a zero-width character inside a
 *   name is drawn as its escape (`bodyText.ts`), and a copy still gives the bytes
 *   that were sent.
 * - **Each block takes its own direction** (QA M13): paragraphs, list items,
 *   quotes and table cells are `dir="auto"`, so a Hebrew paragraph is laid out
 *   right to left beside an English one. Code stays left to right.
 * - **A mention of the viewer is marked** (QA M2): see `bodyText.ts`.
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

/**
 * Names that never resolve to a public address: RFC 6761's `localhost`, `test`, `invalid` and
 * `example`, mDNS's `local` (RFC 6762), `home.arpa` (RFC 8375) and `internal` (ICANN, 2024).
 */
const NEVER_PUBLIC_NAMES = [
  'localhost',
  'local',
  'internal',
  'home.arpa',
  'test',
  'invalid',
  'example',
];
const IPV4_LITERAL = /^\d{1,3}(?:\.\d{1,3}){3}$/;

/**
 * Whether the desktop will open `href` (a {@link safeExternalHref} result) in the system browser.
 *
 * The main process opens only a public http(s) address (`utils/externalBrowserNavigation.ts`,
 * `utils/embeddedBrowserPolicy.ts`): never mailto, a URL carrying a user name or password, a
 * literal IP, `localhost`, or a host that resolves to a private address, and it refuses them
 * with nothing on screen. Everything but the DNS lookup is known here, so those addresses are not
 * drawn as links. A single-label name (`http://printer/`) and a name under a never-public suffix
 * count as private: neither resolves publicly. A public-looking name that resolves to a private
 * address on the lab's network can only be found by the lookup the main process makes.
 */
export function openableHref(href: string): boolean {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return false;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;
  if (url.username || url.password) return false;
  const host = url.hostname
    .replace(/^\[|\]$/g, '')
    .replace(/\.$/, '')
    .toLowerCase();
  // No dot: a single-label name, or an IPv6 literal (the URL parser writes every IPv4-mapped one
  // in hex).
  if (!host.includes('.') || IPV4_LITERAL.test(host)) return false;
  return !NEVER_PUBLIC_NAMES.some((name) => host === name || host.endsWith(`.${name}`));
}

/** What a link that is not opened shows of its address: a mailto's address, else the URL. */
function shownAddress(href: string): string {
  try {
    const url = new URL(href);
    return url.protocol === 'mailto:' ? url.pathname : url.href;
  } catch {
    return href;
  }
}

/**
 * A link the desktop will not open, as text: its words, then its address in brackets unless the
 * words already are the address (an autolinked URL or email address).
 */
function UnopenedLink({
  href,
  text,
  children,
}: {
  href: string;
  text: string;
  children?: ReactNode;
}) {
  const address = shownAddress(href);
  const words = text.trim();
  const same = [address, href, href.replace(/\/$/, ''), `mailto:${address}`].includes(words);
  return (
    <span
      className="crew-md-unlinked"
      title={
        href.startsWith('mailto:')
          ? timelineCopy.linkNotOpenedEmail
          : timelineCopy.linkNotOpenedPrivate
      }
    >
      {children}
      {same ? null : ` (${address})`}
    </span>
  );
}

/** A host name as compared: lower case, no trailing dot, no leading `www.`. */
function comparableHost(host: string): string {
  return host
    .toLowerCase()
    .replace(/\.$/, '')
    .replace(/^www\./, '');
}

/**
 * What the words hold beyond the shared drop set that is never drawn either (a variation selector,
 * a Hangul filler): removed first, then the shared set, so the words are read as the eye reads them.
 */
const UNDRAWN_BEYOND_DROP_SET = /\p{Default_Ignorable_Code_Point}/gu;
/** A slash as the eye takes it: the URL parser reads a backslash as one, the others look like one. */
const SLASH_LOOKALIKE = /[\\\u{FF0F}\u{2044}\u{2215}\u{29F8}]/gu;
/**
 * A dot as the eye takes it in a host name: the full stops the URL parser reads as dots (`。`, `．`,
 * `｡`, `․`, `﹒`), and the characters Unicode lists as confusable with a full stop, which are drawn
 * as one (`ꓸ` U+A4F8, the Arabic-Indic zeros U+0660 and U+06F0, the Syriac, Vai and Kharoshthi
 * full stops, the musical augmentation dot). `www{U+A4F8}ucsf{U+A4F8}edu` reads `www.ucsf.edu`, so
 * it is read as that name. The augmentation dot is a combining mark, so it stands outside the
 * class: in one it would read as combined with the character before it.
 */
const DOT_LOOKALIKE =
  /[\u{3002}\u{FF0E}\u{FF61}\u{2024}\u{FE52}\u{A4F8}\u{0660}\u{06F0}\u{0701}\u{0702}\u{A60E}\u{10A50}]|\u{1D16D}/gu;
/**
 * A colon as the eye takes it: the characters Unicode lists as confusable with one (`˸` U+02F8,
 * `ː` U+02D0, `∶` U+2236, `ꓽ` U+A4FD, `꞉` U+A789, the Armenian, Hebrew, Syriac, Runic and Mongolian
 * marks, the Devanagari and Gujarati visarga, which are combining marks and so stand outside the
 * class). NFKC has already made the full-width and small colons plain ones. `https˸//www.ucsf.edu`
 * reads as an address, so its scheme is read as one.
 */
const COLON_LOOKALIKE =
  /[\u{02D0}\u{02F8}\u{0589}\u{05C3}\u{0703}\u{0704}\u{16EC}\u{1803}\u{1809}\u{205A}\u{2236}\u{A4FD}\u{A789}]|\u{0903}|\u{0A83}/gu;
/** A character of a scheme. Never a dot: `ucsf.edu:` is a host. */
const SCHEME_CHARACTER = /^[A-Za-z0-9+-]$/;
/** The letters a scheme starts with. */
const SCHEME_LETTER = /^[A-Za-z]$/;
/**
 * A scheme the URL parser reads a host after with no slash at all (`https:ucsf`), at the end of the
 * letters before a colon, since the eye reads it there whatever is glued before it (`2https:ucsf`).
 */
const SLASHLESS_SCHEME_END = /https?$/i;
/** What a word holds before its first letter or digit: brackets, quotes, slashes. Anchored. */
const LEADING_NON_WORD = /^[^\p{L}\p{N}]+/u;
/** What a host name's ends may be: a letter, a mark or a digit, not the punctuation around it. */
const HOST_END = /^[\p{L}\p{M}\p{N}]$/u;
/** A run of what a host name may hold: letters, marks, digits, hyphens and dots. */
const HOST_RUN = /[^\p{L}\p{M}\p{N}.-]+/u;
/** One label of a host name. Anchored, so it is tried once per label. */
const HOST_LABEL = /^[\p{L}\p{N}](?:[\p{L}\p{M}\p{N}-]*[\p{L}\p{M}\p{N}])?$/u;
/** The letters a top-level label starts with (`edu` of `edu-login`), or a punycode label. */
const TOP_LEVEL = /^(?:xn--[a-z0-9-]+$|\p{L}[\p{L}\p{M}]*)/iu;
const IPV4_WORDS = /^\d{1,3}(?:\.\d{1,3}){3}$/;

/** `part` without the dots and hyphens at its ends, by a loop: `[.-]+$` would rescan every start. */
function trimDotsAndHyphens(part: string): string {
  let start = 0;
  let end = part.length;
  while (start < end && (part[start] === '.' || part[start] === '-')) start += 1;
  while (end > start && (part[end - 1] === '.' || part[end - 1] === '-')) end -= 1;
  return part.slice(start, end);
}

/**
 * The host names in a run of host characters: `www.ucsf.edu` of `www.ucsf.edu.`, `ucsf.edu` of
 * `ucsf.edu-login`, an IPv4 address; nothing for `Fig.2`, `e.g` or a single word.
 */
function hostsInRun(run: string): string[] {
  const hosts: string[] = [];
  for (const part of run.split(/\.{2,}/)) {
    const candidate = trimDotsAndHyphens(part);
    if (!candidate.includes('.')) continue;
    if (IPV4_WORDS.test(candidate)) {
      hosts.push(candidate);
      continue;
    }
    const labels = candidate.split('.');
    const top = TOP_LEVEL.exec(labels[labels.length - 1])?.[0] ?? '';
    if (top.length < 2) continue;
    // The labels before it, back to the first one a host could not have.
    let first = labels.length - 1;
    while (first > 0 && HOST_LABEL.test(labels[first - 1])) first -= 1;
    if (first === labels.length - 1) continue;
    hosts.push([...labels.slice(first, -1), top].join('.'));
  }
  return hosts;
}

/**
 * Where the part of a word that starts at `from` ends: at its first `/`, `?` or `#`, which end an
 * address's host part and start its path, query or fragment; else at the word's end. By a loop
 * from `from`, so reading a word part by part reads each character once.
 */
function partEnd(word: string, from: number): number {
  let end = from;
  while (end < word.length && word[end] !== '/' && word[end] !== '?' && word[end] !== '#') end += 1;
  return end;
}

/**
 * Where the scheme that ends at `colon` starts, or -1 when the characters before the colon, back
 * to `from`, are not one. A scheme is the letters, digits, `+` and `-` before the colon, from the
 * first letter among them. A digit glued before it is not part of it, as the eye does not read it
 * so: `½https://` reads as `1/2https://` once its compatibility form and its fraction slash are
 * read, and its address starts after `https:`. There is no scheme after a dot: `ucsf.edu:/login`
 * has a host before its colon, and the colon starts its port. By a loop, not `[a-z…]*$`, which
 * would rescan every start.
 */
function schemeStart(word: string, from: number, colon: number): number {
  let run = colon;
  while (run > from && SCHEME_CHARACTER.test(word[run - 1])) run -= 1;
  if (run > 0 && word[run - 1] === '.') return -1;
  for (let start = run; start < colon; start += 1) {
    if (SCHEME_LETTER.test(word[start])) return start;
  }
  return -1;
}

interface SchemeAt {
  /** Where the scheme starts. */
  scheme: number;
  /** Where its colon is. */
  colon: number;
}

/**
 * The schemes an address starts after in the part `word[from, end)` (`end` is the part's `/`, `?`
 * or `#`, or the word's end). `slashed`: a colon right before the part's slash (`https://`,
 * `(https://`, `visit:https://`). `slashless`: the first other colon after `http` or `https`, which
 * need no slash (`https:ucsf`). A part can hold both (`https::ucsf:/intranet`), and each address is
 * read, since either may be the one the eye takes. By a loop to `end`, not `indexOf(':')`, which
 * would read past the part to the word's end.
 */
function schemesInPart(
  word: string,
  from: number,
  end: number
): { slashed: SchemeAt | null; slashless: SchemeAt | null } {
  const slashedColon = end > from && word[end - 1] === ':' && word[end] === '/' ? end - 1 : -1;
  let slashless: SchemeAt | null = null;
  for (let colon = from; colon < end && !slashless; colon += 1) {
    if (word[colon] !== ':' || colon === slashedColon) continue;
    const scheme = schemeStart(word, from, colon);
    // At most the five letters of `https`, so a long run before the colon is not read again.
    if (scheme >= 0 && SLASHLESS_SCHEME_END.test(word.slice(Math.max(scheme, colon - 5), colon))) {
      slashless = { scheme, colon };
    }
  }
  const slashedScheme = slashedColon >= 0 ? schemeStart(word, from, slashedColon) : -1;
  return {
    slashed: slashedScheme >= 0 ? { scheme: slashedScheme, colon: slashedColon } : null,
    slashless,
  };
}

/**
 * The host an address's authority names, as the eye reads it: after any user name, before any
 * port, without the punctuation at its ends (`www.ucsf.edu` of `www.ucsf.edu).`); an IPv6 literal
 * with its brackets. Empty when it names none.
 */
function authorityHost(authority: string): string {
  let host = authority.slice(authority.lastIndexOf('@') + 1);
  if (host.startsWith('[')) {
    const close = host.indexOf(']');
    return close > 0 ? host.slice(0, close + 1) : host;
  }
  const port = host.indexOf(':');
  if (port >= 0) host = host.slice(0, port);
  // By character, not code unit, so a letter outside the Basic Multilingual Plane at an end stays.
  const characters = Array.from(host);
  let start = 0;
  let end = characters.length;
  while (start < end && !HOST_END.test(characters[start])) start += 1;
  while (end > start && !HOST_END.test(characters[end - 1])) end -= 1;
  return characters.slice(start, end).join('');
}

/**
 * What a link's words say about where it goes (QA M4): every host they name, and whether an
 * address in them carries a user name (`https://www.ucsf.edu@evil.example.net/`, which names
 * evil.example.net and reads as ucsf.edu).
 *
 * An address after a scheme, or after a `//`, names its host whatever that host looks like:
 * `https://intranet`, `https://ucsf` and `https://www{U+A4F8}ucsf{U+A4F8}edu` each name a host,
 * with or without a dot the code can see, and one the URL parser refuses is a difference, not
 * nothing. Elsewhere a host is a dotted name (`ucsf.edu`, not `Fig.2` or `e.g.`), since a single
 * word is not one.
 *
 * Every address-shaped part counts, wherever it is in the words and whatever surrounds it: a
 * sentence's final period (`ucsf.edu.`), a comma or a bracket (`(https://www.ucsf.edu)`), quotes,
 * other words (`Go to ucsf.edu`, `https://www.ucsf.edu login`), leading slashes (`//ucsf.edu`),
 * backslashes or look-alike slashes for the scheme's (`https:\\www.ucsf.edu`), characters drawn as
 * dots or colons, characters that draw nothing, and words glued before the address by a slash, a
 * question mark or a hash (`Login/https://www.ucsf.edu`, `Portal/www.ucsf.edu`).
 *
 * A word is read part by part, each part ending at its first `/`, `?` or `#`, until a part names a
 * host. What follows that host is its path, query or fragment, which names nothing: `index.html` in
 * a path is not a host, nor is an address in a query (`ucsf.edu/sso?return=https://portal` names
 * ucsf.edu). A part that names no host has no path, so what follows it is read in turn. Each
 * character of a word is read a bounded number of times, and each test is anchored or a loop, so a
 * long link costs no more than its length.
 */
function addressesInWords(words: string): { hosts: string[]; userinfo: boolean } {
  const hosts: string[] = [];
  let userinfo = false;
  const dottedHosts = (part: string) => {
    for (const run of part.split(HOST_RUN)) hosts.push(...hostsInRun(run));
  };
  // The address that starts at `from` in `word`, however many slashes lead it (the parser takes
  // `https:/x` as `https://x`): its host, whatever it looks like, and any dotted names in it.
  // Returns where its host part ends.
  const address = (word: string, from: number): number => {
    let start = from;
    while (word[start] === '/') start += 1;
    const end = partEnd(word, start);
    const authority = word.slice(start, end);
    if (authority.includes('@')) userinfo = true;
    const named = authorityHost(authority);
    if (named) hosts.push(named);
    dottedHosts(authority);
    return end;
  };
  // A break or a tab still parts two words: made a space before the controls are dropped. Then
  // compatibility forms are read as what they stand for (`ｕｃｓｆ.ｅｄｕ`, `ucsf․edu`), as the URL
  // parser reads them.
  const text = stripHiddenCharacters(
    words.replace(/[\t\n\v\f\r]/g, ' ').replace(UNDRAWN_BEYOND_DROP_SET, '')
  )
    .normalize('NFKC')
    .replace(SLASH_LOOKALIKE, '/')
    .replace(DOT_LOOKALIKE, '.')
    .replace(COLON_LOOKALIKE, ':');
  for (const spaced of text.split(/\s+/)) {
    // Past its brackets, quotes and slashes (`(https://…`, `//ucsf.edu`).
    const word = spaced.replace(LEADING_NON_WORD, '');
    let from = 0;
    while (from < word.length) {
      const before = hosts.length;
      const end = partEnd(word, from);
      const { slashed, slashless } = schemesInPart(word, from, end);
      const first = slashless ?? slashed;
      let next = end + 1;
      if (first) {
        // The words glued before the scheme (`visit:` of `visit:https://…`), then the address after
        // a scheme with no slash, which ends with the part, then the one after the part's slash.
        dottedHosts(word.slice(from, first.scheme));
        if (slashless) next = address(word, slashless.colon + 1);
        if (slashed) next = address(word, slashed.colon + 1);
      } else {
        dottedHosts(word.slice(from, end));
        // The part names no host, and a `//` follows it: an address, as the eye reads one,
        // whatever stands where its scheme would (`https;//intranet`).
        if (hosts.length === before && word[end] === '/' && word[end + 1] === '/') {
          next = address(word, end);
        }
      }
      // A host: the rest of the word is its path, query or fragment.
      if (hosts.length > before) break;
      // None, not even after a scheme (`https:?www.ucsf.edu`): the next part is read. `next` is
      // past `from` in every branch, so the loop ends.
      from = next;
    }
  }
  return { hosts, userinfo };
}

/** A host name as the URL parser writes it (`xn--` for a look-alike), or null for none it takes. */
function parsedHost(host: string): string | null {
  try {
    return new URL(`https://${host}/`).hostname || null;
  } catch {
    return null;
  }
}

/**
 * The host a link really opens, when its words name a different one (QA M4): the link
 * `[https://www.ucsf.edu](https://evil.example.net/login)` read "https://www.ucsf.edu", with the
 * target only in a hover title that keyboard and screen-reader users never get. Null when the words
 * name no host, or name only the one the link opens (`www.` aside). A look-alike name in the words
 * is turned to its `xn--` form by the parser, so it never matches the real one; a name the parser
 * refuses matches nothing either. Words whose address carries a user name always get the host.
 */
export function mismatchedLinkHost(words: string, href: string): string | null {
  let target: string;
  try {
    target = new URL(href).hostname;
  } catch {
    return null;
  }
  const named = addressesInWords(words);
  if (named.userinfo) return target;
  const opened = comparableHost(target);
  const differs = named.hosts.some((host) => {
    const parsed = parsedHost(host);
    return parsed === null || comparableHost(parsed) !== opened;
  });
  return differs ? target : null;
}

/** The real host after a link's words, inside the link so it is read as part of its name. */
function RealHost({ host }: { host: string | null }) {
  if (!host) return null;
  return (
    <>
      {' '}
      <span className="crew-md-link-host">{timelineCopy.linkRealHost(host)}</span>
    </>
  );
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

/**
 * Words another person wrote, drawn with their hidden characters as escapes (`bodyText.ts`): for
 * text that does not go through the markdown step, a code block's and an agent's tool updates.
 */
export function VisibleText({ text }: { text: string }) {
  return (
    <>
      {revealHiddenCharacters(text).map((segment, index) =>
        segment.kind === 'text' ? (
          segment.text
        ) : (
          <span
            key={index}
            className="crew-md-hidden-char"
            dir="ltr"
            title={timelineCopy.hiddenCharacter(segment.codePoint)}
            data-hidden-char={segment.codePoint}
          >
            {segment.escape}
          </span>
        )
      )}
    </>
  );
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
 * once. The answer decides whether the box is a scrollable region a keyboard can reach.
 *
 * `more`: there is content past its right edge right now — it overflows and is not scrolled to
 * the end. That is the one measurement, taken again on every scroll, behind the box's
 * `data-overflow` and so its right-edge fade: macOS hides overlay scrollbars, so a table that
 * ended at "Growth ratio (od600_t4 / od60" gave no sign it went on (Q3-18). At the end the fade
 * goes, so the last column is never dimmed.
 */
function useOverflowsSideways<T extends HTMLElement>(): [
  (node: T | null) => void,
  { overflows: boolean; more: boolean },
] {
  const [node, setNode] = useState<T | null>(null);
  const [state, setState] = useState({ overflows: false, more: false });
  useLayoutEffect(() => {
    if (!node) return;
    const measure = () => {
      const overflows = node.scrollWidth > node.clientWidth + 1;
      const more = overflows && node.scrollLeft + node.clientWidth < node.scrollWidth - 1;
      setState((current) =>
        current.overflows === overflows && current.more === more ? current : { overflows, more }
      );
    };
    measure();
    node.addEventListener('scroll', measure, { passive: true });
    if (typeof ResizeObserver !== 'function') {
      return () => node.removeEventListener('scroll', measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    if (node.firstElementChild) observer.observe(node.firstElementChild);
    return () => {
      observer.disconnect();
      node.removeEventListener('scroll', measure);
    };
  }, [node]);
  return [setNode, state];
}

/**
 * The attributes of a box that may scroll sideways: when it scrolls, a region a keyboard can
 * reach, named; while there is more to its right, `data-overflow` for the fade (`timeline.css`).
 */
function scrollRegion({ overflows, more }: { overflows: boolean; more: boolean }, name: string) {
  return {
    ...(overflows ? { role: 'region', 'aria-label': name, tabIndex: 0 } : {}),
    'data-overflow': more ? 'true' : undefined,
  };
}

function CodeBlock({ text, language }: { text: string; language: string }) {
  const [measure, overflow] = useOverflowsSideways<HTMLPreElement>();
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
        {...scrollRegion(overflow, timelineCopy.codeRegion(language))}
      >
        {/* Drawn with its hidden characters shown; Copy code hands out `text`, the raw bytes. */}
        <code>
          <VisibleText text={text} />
        </code>
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
      const text = bodyNodeText(element).replace(/\s+/g, ' ').trim();
      if (text) cells.push(text);
      return;
    }
    if (Array.isArray(element.children)) element.children.forEach(visit);
  };
  visit(node);
  return cells;
}

function TableScroll({ node, children }: { node: unknown; children?: ReactNode }) {
  const [measure, overflow] = useOverflowsSideways<HTMLDivElement>();
  const name = timelineCopy.tableNamed(headerCells(node).join(', '));
  return (
    <div ref={measure} className="crew-md-table-scroll" {...scrollRegion(overflow, name)}>
      <table className="crew-md-table">{children}</table>
    </div>
  );
}

function Heading({ children }: { children?: ReactNode }) {
  return (
    <p className="crew-md-heading" dir="auto">
      {children}
    </p>
  );
}

const COMPONENTS: Components = {
  p: ({ children }) => (
    <p className="crew-md-p" dir="auto">
      {children}
    </p>
  ),
  a: ({ href, children, node }) => {
    const safe = safeExternalHref(href);
    if (!safe) return <span className="crew-md-unlinked">{children}</span>;
    if (!openableHref(safe))
      return (
        <UnopenedLink href={safe} text={bodyNodeText(node, true)}>
          {children}
        </UnopenedLink>
      );
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
        <RealHost host={mismatchedLinkHost(bodyNodeText(node, true), safe)} />
      </a>
    );
  },
  img: ({ src, alt }) => {
    const name = typeof alt === 'string' ? alt.trim() : '';
    const label = name ? timelineCopy.imageNamed(name) : timelineCopy.image;
    const safe = safeExternalHref(src, false);
    // The alt text is a property, not a text node, so the body step never saw it.
    const content = (
      <>
        <ImageIcon aria-hidden className="crew-md-image-icon" />
        <VisibleText text={label} />
      </>
    );
    if (safe && !openableHref(safe))
      return (
        <span className="crew-md-image" title={timelineCopy.linkNotOpenedPrivate}>
          {content}
          {` (${safe})`}
        </span>
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
        <RealHost host={mismatchedLinkHost(name, safe)} />
      </a>
    ) : (
      <span className="crew-md-image">{content}</span>
    );
  },
  pre: ({ node }) => {
    const code = codeChild(node);
    return (
      <CodeBlock
        text={bodyNodeText(code, true).replace(/\n$/, '')}
        language={fenceLanguage(code)}
      />
    );
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
  li: ({ children }) => (
    <li className="crew-md-item" dir="auto">
      {children}
    </li>
  ),
  input: ({ checked }) => (
    <input
      type="checkbox"
      className="crew-md-task-box"
      checked={checked === true}
      disabled
      readOnly
    />
  ),
  blockquote: ({ children }) => (
    <blockquote className="crew-md-quote" dir="auto">
      {children}
    </blockquote>
  ),
  hr: () => <hr className="crew-md-rule" />,
  table: ({ node, children }) => <TableScroll node={node}>{children}</TableScroll>,
  th: ({ children, style }) => (
    <th className="crew-md-cell" data-head="true" dir="auto" style={style}>
      {children}
    </th>
  ),
  td: ({ children, style }) => (
    <td className="crew-md-cell" dir="auto" style={style}>
      {children}
    </td>
  ),
};

export interface CrewMarkdownProps {
  text: string;
  /** The viewer's username, whose `@mentions` are marked; absent or null marks none. */
  mention?: string | null;
  /** The ID of the hidden "mentions you" label the row names itself by, when it wants one. */
  mentionLabelId?: string | null;
}

/** The markdown alone, unfolded. Memoized on its props: a timeline re-renders often. */
export const CrewMarkdown = memo(function CrewMarkdown({
  text,
  mention = null,
  mentionLabelId = null,
}: CrewMarkdownProps) {
  const rehypePlugins = useMemo<NonNullable<Options['rehypePlugins']>>(
    // The step reads and writes only the node fields it declares; the cast is to unified's tree.
    () => [[rehypeCrewBodyText as never, { mention, mentionLabelId }]],
    [mention, mentionLabelId]
  );
  return (
    <div className="crew-md text-body text-text-default">
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        rehypePlugins={rehypePlugins}
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
export function MessageBody({
  body,
  mention = null,
  mentionLabelId = null,
}: {
  body: string;
  mention?: string | null;
  mentionLabelId?: string | null;
}) {
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
        <CrewMarkdown text={text} mention={mention} mentionLabelId={mentionLabelId} />
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
