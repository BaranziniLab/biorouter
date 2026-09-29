import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CLAMP_CHAR_THRESHOLD } from '../../../utils/messageClamp';
import { timelineCopy } from './copy';
import { MessageBody, mismatchedLinkHost, safeExternalHref } from './MessageBody';

/**
 * Message bodies are markdown with nothing active in them (supplement: an
 * agent's post must not show raw backticks; nothing a teammate or their agent
 * wrote may run, fetch or navigate).
 */

afterEach(() => {
  vi.restoreAllMocks();
});

describe('markdown', () => {
  it('formats an agent’s answer instead of showing its backticks', () => {
    const { container } = render(
      <MessageBody body={'Posted in `#general`:\n\n**Totals**\n\n- od600 = 1.80\n- cells = 18.0'} />
    );
    expect(container).not.toHaveTextContent('`');
    expect(container.querySelector('code')).toHaveTextContent('#general');
    expect(container.querySelector('strong')).toHaveTextContent('Totals');
    expect(screen.getAllByRole('listitem')).toHaveLength(2);
  });

  it('keeps a single newline as a line break', () => {
    const { container } = render(<MessageBody body={'first line\nsecond line'} />);
    expect(container.querySelector('br')).not.toBeNull();
  });

  it('shows raw HTML as the characters typed, never as elements', () => {
    const { container } = render(
      <MessageBody
        body={'<script>alert(1)</script> <img src=x onerror="alert(2)"> <svg onload="x()"></svg>'}
      />
    );
    expect(container.querySelector('script, img, svg[onload], [onerror]')).toBeNull();
    expect(container).toHaveTextContent('<script>alert(1)</script>');
  });

  it('never fetches an image: it becomes a link to open, named by its alt text', () => {
    const { container } = render(
      <MessageBody body={'![counts plot](https://example.org/leak?secret=1)'} />
    );
    expect(container.querySelector('img')).toBeNull();
    const link = screen.getByRole('link', { name: timelineCopy.imageNamed('counts plot') });
    expect(link).toHaveAttribute('href', 'https://example.org/leak?secret=1');
    expect(link).toHaveAttribute('rel', 'noopener noreferrer');
  });

  it('does not read a local image path either', () => {
    const read = vi.fn();
    Object.assign(window, { electron: { ...(window.electron ?? {}), readArtifactFile: read } });
    const { container } = render(<MessageBody body={'![key](/Users/me/.ssh/id_ed25519)'} />);
    expect(container.querySelector('img, a')).toBeNull();
    expect(container).toHaveTextContent(timelineCopy.imageNamed('key'));
    expect(read).not.toHaveBeenCalled();
  });

  it('links only public http and https addresses, and opens them in the system browser', () => {
    const openExternal = vi.fn(async () => {});
    Object.assign(window, { electron: { ...(window.electron ?? {}), openExternal } });
    const { container } = render(
      <MessageBody
        body={[
          '[docs](https://biorouter.ucsf.edu/docs.html)',
          '[mail](mailto:lab@example.org)',
          '[bad](javascript:alert(1))',
          '[data](data:text/html;base64,PHNjcmlwdD4=)',
          '[file](file:///etc/passwd)',
          '[local](../secrets.txt)',
        ].join(' ')}
      />
    );
    const links = screen.getAllByRole('link');
    expect(links.map((link) => link.textContent)).toEqual(['docs']);
    expect(container.innerHTML).not.toMatch(/javascript:|data:text|file:\/\//);
    expect(container).toHaveTextContent('bad');

    // A link says where it really goes, whatever its words say.
    expect(screen.getByRole('link', { name: 'docs' })).toHaveAttribute(
      'title',
      'https://biorouter.ucsf.edu/docs.html'
    );
    fireEvent.click(screen.getByRole('link', { name: 'docs' }));
    expect(openExternal).toHaveBeenCalledWith('https://biorouter.ucsf.edu/docs.html');
  });

  describe('a link the desktop app never opens (RENDERER-3)', () => {
    // The main process opens only a public http(s) address (`externalBrowserNavigation.ts`): not
    // mailto, not a literal IP, not localhost, not a name that is never public. Drawn as a link,
    // such an address was a dead click with nothing said. It is text now, its address in view.
    const openExternal = vi.fn(async () => {});
    const body = [
      '[email the core](mailto:core@lab.org?subject=hi)',
      'core2@lab.org',
      '[jupyter](http://10.20.0.5:8888/lab)',
      'http://127.0.0.1:8888',
      '[ipv6](http://[::1]:8080/)',
      '[dev](http://localhost:3000/x)',
      '[app](http://app.localhost/)',
      '[printer](http://printer/)',
      '[wiki](https://wiki.internal/page)',
      '[nas](http://nas.local/)',
      '[router](http://router.home.arpa/)',
      '[creds](https://user:pw@biorouter.ucsf.edu/)',
    ].join('\n\n');

    it('shows each one as text with its address visible, never as a link', () => {
      Object.assign(window, { electron: { ...(window.electron ?? {}), openExternal } });
      const { container } = render(<MessageBody body={body} />);
      expect(screen.queryAllByRole('link')).toEqual([]);
      expect(container.querySelector('a')).toBeNull();
      const text = container.textContent ?? '';
      expect(text).toContain('email the core (core@lab.org)');
      expect(text).toContain('core2@lab.org');
      expect(text).not.toContain('core2@lab.org (core2@lab.org)');
      expect(text).toContain('jupyter (http://10.20.0.5:8888/lab)');
      expect(text).toContain('http://127.0.0.1:8888');
      expect(text).not.toContain('http://127.0.0.1:8888 (http://127.0.0.1:8888/)');
      expect(text).toContain('wiki (https://wiki.internal/page)');
      expect(text).toContain('dev (http://localhost:3000/x)');
      // Why it is not a link, where there is room to say so.
      const unlinked = [...container.querySelectorAll('.crew-md-unlinked')];
      expect(unlinked).toHaveLength(12);
      expect(unlinked[0]).toHaveAttribute('title', timelineCopy.linkNotOpenedEmail);
      expect(unlinked[2]).toHaveAttribute('title', timelineCopy.linkNotOpenedPrivate);
      for (const node of unlinked) fireEvent.click(node);
      expect(openExternal).not.toHaveBeenCalled();
    });

    it('keeps a public address a link, and an image of a private one text', () => {
      const { container } = render(
        <MessageBody
          body={
            '[lab site](https://lab.example.org/) ![gel](http://192.168.1.4/gel.png) ' +
            '![plot](https://lab.example.org/plot.png)'
          }
        />
      );
      expect(screen.getAllByRole('link').map((link) => link.textContent)).toEqual([
        'lab site',
        timelineCopy.imageNamed('plot'),
      ]);
      expect(container.textContent).toContain(
        `${timelineCopy.imageNamed('gel')} (http://192.168.1.4/gel.png)`
      );
    });
  });

  /**
   * QA M4: `[https://www.ucsf.edu](https://evil.example.net/login)` read as ucsf.edu, with the
   * real target only in a hover title that a keyboard or a screen reader never gets.
   */
  describe('a link whose words name another host', () => {
    it('shows the host it really opens, as part of the link’s name', () => {
      render(
        <MessageBody body={'Sign in at [https://www.ucsf.edu](https://evil.example.net/login)'} />
      );
      const link = screen.getByRole('link', {
        name: 'https://www.ucsf.edu (evil.example.net)',
      });
      expect(link).toHaveAttribute('href', 'https://evil.example.net/login');
      expect(within(link).getByText('(evil.example.net)')).toHaveClass('crew-md-link-host');
    });

    it.each([
      ['a bare domain', '[ucsf.edu](https://evil.example.net/)', 'evil.example.net'],
      ['a path on a domain', '[ucsf.edu/login](https://evil.example.net/)', 'evil.example.net'],
      ['a look-alike name', '[https://www.uсsf.edu](https://www.ucsf.edu/)', 'www.ucsf.edu'],
      [
        'an image named by an address',
        '![https://www.ucsf.edu](https://evil.example.net/x.png)',
        'evil.example.net',
      ],
      [
        'words that hide the host behind a user name',
        '[https://www.ucsf.edu@evil.example.net/login](https://evil.example.net/login)',
        'evil.example.net',
      ],
      // The words need not be one bare address to name one (a review found each of these drew no
      // host while the words plainly said ucsf.edu).
      [
        'a sentence’s final period',
        '[ucsf.edu.](https://evil.example.net/login)',
        'evil.example.net',
      ],
      ['a comma', '[ucsf.edu,](https://evil.example.net/login)', 'evil.example.net'],
      ['an exclamation mark', '[ucsf.edu!](https://evil.example.net/login)', 'evil.example.net'],
      ['brackets', '[(https://www.ucsf.edu)](https://evil.example.net/login)', 'evil.example.net'],
      ['quotes', '[“www.ucsf.edu”](https://evil.example.net/login)', 'evil.example.net'],
      [
        'a word after the address',
        '[https://www.ucsf.edu login](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'words after the address and its path',
        '[https://www.ucsf.edu/login (official)](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'words before the address',
        '[Go to ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'backslashes for the scheme’s slashes',
        // Markdown reads `\\` as one backslash: the words are `https:\\www.ucsf.edu`.
        '[https:\\\\\\\\www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'one backslash for the scheme’s slashes',
        '[https:\\\\www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'look-alike slashes and full stops',
        '[https:／／www.ucsf。edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a hidden character inside the name',
        '[ucsf\u{200B}.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'tags around the name',
        '[<b>ucsf.edu</b>](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a name the link does not open among one it does',
        '[evil.example.net, not ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a name glued to a word',
        '[visit:ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      ['an address', '[10.0.0.1](https://evil.example.net/login)', 'evil.example.net'],
      [
        'full-width letters',
        '[ｕｃｓｆ.ｅｄｕ](https://evil.example.net/login)',
        'evil.example.net',
      ],
      ['a dot leader for a dot', '[ucsf․edu](https://evil.example.net/login)', 'evil.example.net'],
      // An address after a scheme names its host with or without a dot the code can see: a
      // character drawn as a dot, or none at all (round 2 read these as naming nothing).
      [
        'a scheme and a host dotted with a Lisu tone letter',
        '[https://www\u{A4F8}ucsf\u{A4F8}edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a host dotted with a Lisu tone letter',
        '[www\u{A4F8}ucsf\u{A4F8}edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a host dotted with an Arabic-Indic zero',
        '[https://ucsf\u{0660}edu/](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a scheme and a one-word host',
        '[https://ucsf](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a scheme and a one-word host with a path',
        '[https://intranet/x](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a scheme with no slash',
        '[https:ucsf](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a scheme glued to a word',
        '[visit:https://ucsf](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a look-alike colon',
        '[https\u{02F8}//www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a one-word host between brackets',
        '[(https://intranet)](https://evil.example.net/login)',
        'evil.example.net',
      ],
      // A host before a later `:/` is the host: that `:/` is in a path, a query or a fragment.
      [
        'an address in the query',
        '[www.ucsf.edu/login?next=http://intranet](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'an address in the query after a path',
        '[ucsf.edu/sso?return=https://portal](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a colon and slash in the fragment',
        '[ucsf.edu/login#:/](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a colon and slash after the host',
        '[ucsf.edu:/login](https://evil.example.net/login)',
        'evil.example.net',
      ],
      ['leading slashes', '[//ucsf.edu](https://evil.example.net/login)', 'evil.example.net'],
      // A word before the first slash, question mark or hash that names no host has no path, so an
      // address after it is still read (round 3 read these as naming nothing).
      [
        'a word and a slash before the address',
        '[Login/https://www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a word and a slash before an address with a path',
        '[Ref/https://www.ucsf.edu/login](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a word and a question mark before the address',
        '[Login?https://www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a word and a hash before the address',
        '[Login#https://www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a word and a backslash before the address',
        '[x\\\\https://ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a fraction before the address',
        '[½https://www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'a word and a slash before a domain',
        '[Portal/www.ucsf.edu](https://evil.example.net/login)',
        'evil.example.net',
      ],
      // An image in a link draws its alt text as the link's words (`Image: https://www.ucsf.edu`).
      [
        'an image in the link, named by an address',
        '[![https://www.ucsf.edu](data:x)](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'an image in the link, named by a domain',
        '[![ucsf.edu](x.png)](https://evil.example.net/login)',
        'evil.example.net',
      ],
      [
        'an image in the link, named by a word and a slash before an address',
        '[![Ref/https://www.ucsf.edu](data:x)](https://evil.example.net/login)',
        'evil.example.net',
      ],
    ])('names the real host after %s', (_label, body, host) => {
      const { container } = render(<MessageBody body={body} />);
      expect(container.querySelector('.crew-md-link-host')).toHaveTextContent(`(${host})`);
    });

    it('names the real host in the name of a link around an image', () => {
      render(
        <MessageBody body={'[![https://www.ucsf.edu](data:x)](https://evil.example.net/login)'} />
      );
      const link = screen.getByRole('link', {
        name: `${timelineCopy.imageNamed('https://www.ucsf.edu')} (evil.example.net)`,
      });
      expect(link).toHaveAttribute('href', 'https://evil.example.net/login');
    });

    /**
     * Each of Unicode's confusables of FULL STOP, and the hosts a word names past what the eye
     * reads as punctuation, straight through the reader: the words as a link's text node holds
     * them, with no markdown in the way.
     */
    it.each([
      ['https://www\u{A4F8}ucsf\u{A4F8}edu'],
      ['www\u{A4F8}ucsf\u{A4F8}edu'],
      ['https://ucsf\u{0660}edu/'],
      ['ucsf\u{06F0}edu'],
      ['ucsf\u{0701}edu'],
      ['ucsf\u{0702}edu'],
      ['ucsf\u{A60E}edu'],
      ['ucsf\u{10A50}edu'],
      ['ucsf\u{1D16D}edu'],
      ['https://ucsf'],
      ['HTTPS://INTRANET/x'],
      ['http:intranet'],
      ['https://ucsf:8443/x'],
      ['https:\\intranet'],
      ['https://[::1]/'],
      ['https://ucsf.edu,evil.example.org'],
      ['www.ucsf.edu/login?next=http://intranet'],
      ['ucsf.edu/sso?return=https://portal'],
      ['ucsf.edu/login#:/'],
      ['ucsf.edu:/login'],
      ['//ucsf.edu'],
      ['(//ucsf.edu)'],
      // A colon drawn by a look-alike still reads as a scheme's, and a `//` as an address.
      ['https\u{02F8}//www.ucsf.edu'],
      ['https\u{A4FD}//intranet'],
      ['https\u{2236}/www.ucsf.edu'],
      ['https;//www.ucsf.edu'],
      ['https//intranet'],
      // A part before the first `/`, `?` or `#` that names no host has no path: what follows it is
      // read in turn, as an address or as a domain.
      ['Login/https://www.ucsf.edu'],
      ['Login?https://www.ucsf.edu'],
      ['Login#https://www.ucsf.edu'],
      ['x\\https://ucsf.edu'],
      ['go/https://www.ucsf.edu/sso'],
      ['Ref/https://www.ucsf.edu/login'],
      ['a/https://www.ucsf.edu'],
      ['1?https://www.ucsf.edu/login'],
      ['x#https://www.ucsf.edu'],
      ['a:b/https://www.ucsf.edu'],
      ['½https://www.ucsf.edu'],
      ['℅https://www.ucsf.edu'],
      ['2https:/www.ucsf.edu'],
      ['a/b/c/https://www.ucsf.edu'],
      ['Login/(https://www.ucsf.edu)'],
      ['Login/2https:ucsf'],
      ['Note:https:ucsf'],
      ['https:?www.ucsf.edu'],
      ['https://?next=https://www.ucsf.edu'],
      ['Portal/www.ucsf.edu'],
      ['Sign-in?www.ucsf.edu'],
      ['Login#ucsf.edu'],
      ['Login/ucsf.edu:/x'],
      // A part with a scheme that needs no slash and one before its slash: both addresses are read.
      ['http::x:/www.ucsf.edu'],
      ['https:%:/www.ucsf.edu'],
    ])('reads %s as naming a host other than evil.example.net', (words) => {
      expect(mismatchedLinkHost(words, 'https://evil.example.net/login')).toBe('evil.example.net');
    });

    it('reads both addresses of a part with a scheme that needs no slash and one before its slash', () => {
      expect(mismatchedLinkHost('https:ucsf.edu:https://intranet', 'https://www.ucsf.edu/')).toBe(
        'www.ucsf.edu'
      );
    });

    it.each([
      [
        'an address with a redirect in its query',
        'https://www.ucsf.edu/login?next=https://portal.ucsf.edu/x',
      ],
      ['an address in brackets', '(https://www.ucsf.edu)'],
      ['an address with a port', 'https://www.ucsf.edu:443/x'],
      ['an address and a full stop', 'https://www.ucsf.edu.'],
      ['a scheme in capitals', 'HTTPS://WWW.UCSF.EDU/'],
      ['a word before a colon', 'Note:see the portal'],
      ['a mail address', 'mailto:bob@ucsf.edu'],
      ['Arabic-Indic digits', 'السعر ٣٠٥'],
      ['a length mark in a word', 'kaːt'],
      ['a path with a double slash', 'https://www.ucsf.edu/a//b'],
      ['a host, then a double slash', 'www.ucsf.edu//x'],
      ['a word and a slash before the address', 'Login/https://www.ucsf.edu'],
      ['a word and a question mark before the domain', 'Sign-in?www.ucsf.edu'],
      ['a host, then an address in its path', 'www.ucsf.edu/go/https://portal.example.org'],
      ['words joined by slashes', 'and/or TCP/IP input/output'],
      ['times joined by a slash', '12:30/13:00'],
      ['a fraction', '½ cup'],
    ])('reads %s as naming only the host it opens, or none', (_label, words) => {
      expect(mismatchedLinkHost(words, 'https://www.ucsf.edu/login')).toBeNull();
    });

    // Link words are up to 64 KB somebody else chose, read as each row mounts: every test is
    // anchored or a plain split, so none rescans the words from each position.
    it.each([
      ['labels', 'a.'.repeat(32_000)],
      ['dots and hyphens', `${'-.'.repeat(32_000)}x`],
      ['one long label', `${'a'.repeat(64_000)}-`],
      ['a scheme-like run', `${'h'.repeat(64_000)}:`],
      ['addresses', 'ucsf.edu '.repeat(7_000)],
      ['schemes', 'https://'.repeat(8_000)],
      ['colons', `${'a:'.repeat(32_000)}/`],
      ['leading punctuation', `${'('.repeat(64_000)}x`],
      [
        'a host with punctuation at its ends',
        `https://${'('.repeat(32_000)}x${')'.repeat(32_000)}`,
      ],
      // A word is read part by part until a part names a host, so parts that name none must not
      // each read the rest of the word.
      ['parts that name no host', 'a/'.repeat(32_000)],
      ['parts that end in a scheme', 'a:/?'.repeat(16_000)],
      ['empty addresses', 'https:#'.repeat(9_000)],
      ['double slashes after words', 'a//#'.repeat(16_000)],
      ['slashes after a word', `x${'/'.repeat(64_000)}`],
      ['digits before colons', `${'1:'.repeat(32_000)}/`],
      ['a scheme-like run in a later part', `a/${'h'.repeat(64_000)}:`],
    ])('reads 64 KB of %s in bounded time', (_label, words) => {
      const started = performance.now();
      mismatchedLinkHost(words, 'https://www.ucsf.edu/');
      expect(performance.now() - started).toBeLessThan(750);
    });

    it.each([
      ['the same host', '[https://www.ucsf.edu/news](https://www.ucsf.edu/about)'],
      ['the same host with or without www', '[ucsf.edu](https://www.ucsf.edu/)'],
      ['words that are not an address', '[the lab docs](https://evil.example.net/)'],
      ['an autolinked address', 'https://www.ucsf.edu/news'],
      ['an autolinked address to a file', 'https://www.ucsf.edu/docs/index.html'],
      [
        'the same host in a sentence',
        '[Read the news at www.ucsf.edu.](https://www.ucsf.edu/news)',
      ],
      ['abbreviations and numbers', '[e.g. Fig.2, v1.2, U.S.A.](https://www.ucsf.edu/)'],
      ['the same host, then a line of words', '[www.ucsf.edu\nnews](https://www.ucsf.edu/)'],
      [
        'an autolinked address with an address in its query',
        'https://www.ucsf.edu/login?next=https://portal.ucsf.edu/x',
      ],
      ['the same host in brackets', '[(https://www.ucsf.edu)](https://www.ucsf.edu/)'],
      ['the same host with a port', '[https://www.ucsf.edu:443/x](https://www.ucsf.edu/x)'],
      ['an image in the link named by its host', '[![www.ucsf.edu](x.png)](https://www.ucsf.edu/)'],
      ['an image in the link named in words', '[![the lab logo](x.png)](https://www.ucsf.edu/)'],
    ])('adds nothing for %s', (_label, body) => {
      const { container } = render(<MessageBody body={body} />);
      expect(screen.getByRole('link')).toBeInTheDocument();
      expect(container.querySelector('.crew-md-link-host')).toBeNull();
    });
  });

  it('turns headings into bold lines, so a message never adds a page heading', () => {
    render(<MessageBody body={'# Results\n\nbody'} />);
    expect(screen.queryByRole('heading')).toBeNull();
    expect(screen.getByText('Results')).toHaveClass('crew-md-heading');
  });

  it('shows a fenced block with its language and a Copy that copies the code', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');
    const { container } = render(<MessageBody body={'```r\nsum(x)\nmean(x)\n```'} />);
    expect(container.querySelector('pre')).toHaveTextContent('sum(x)');
    expect(screen.getByText('r')).toHaveClass('crew-md-code-lang');
    await user.click(screen.getByRole('button', { name: timelineCopy.copyCode }));
    expect(writeText).toHaveBeenCalledWith('sum(x)\nmean(x)');
  });

  describe('tables and code blocks: a Tab stop only when they scroll (Q2-12, Q2-57)', () => {
    /** Makes every element report `scrollWidth` wider than `clientWidth` while `wide` holds. */
    function layoutWidths(wide: boolean) {
      vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(wide ? 900 : 400);
      vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(400);
    }
    const TABLE = '| Sample | od600_t0 |\n|---|---|\n| WT-1 | 0.05 |';

    it('draws a table that fits as a plain box: no region, no Tab stop', () => {
      layoutWidths(false);
      const { container } = render(<MessageBody body={TABLE} />);
      expect(screen.queryByRole('region')).toBeNull();
      const box = container.querySelector('.crew-md-table-scroll') as HTMLElement;
      expect(box).not.toHaveAttribute('tabindex');
      expect(box).not.toHaveAttribute('aria-label');
      expect(screen.getAllByRole('cell')).toHaveLength(2);
    });

    it('makes a table that scrolls a focusable region named for its header cells', () => {
      layoutWidths(true);
      render(<MessageBody body={TABLE} />);
      const region = screen.getByRole('region', { name: 'Table: Sample, od600_t0' });
      expect(region).toHaveAttribute('tabindex', '0');
      expect(region).toHaveClass('crew-md-table-scroll');
      expect(timelineCopy.tableNamed('Sample, od600_t0')).toBe('Table: Sample, od600_t0');
    });

    it('re-measures when the column changes width', () => {
      let observed: (() => void) | null = null;
      class Observer {
        constructor(callback: () => void) {
          observed = callback;
        }
        observe() {}
        disconnect() {}
      }
      vi.stubGlobal('ResizeObserver', Observer);
      try {
        const wide = vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(400);
        vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(400);
        render(<MessageBody body={TABLE} />);
        expect(screen.queryByRole('region')).toBeNull();
        // The details pane opens and the column narrows: now it scrolls.
        wide.mockReturnValue(900);
        act(() => observed?.());
        expect(screen.getByRole('region', { name: 'Table: Sample, od600_t0' })).toHaveAttribute(
          'tabindex',
          '0'
        );
      } finally {
        vi.unstubAllGlobals();
      }
    });

    it('marks a box with more to its right for the fade, and drops it at the scroll end (Q3-18)', () => {
      layoutWidths(false);
      const { container, unmount } = render(<MessageBody body={TABLE} />);
      // A table that fits never fades.
      expect(container.querySelector('.crew-md-table-scroll')).not.toHaveAttribute('data-overflow');
      unmount();

      layoutWidths(true);
      render(<MessageBody body={TABLE} />);
      const region = screen.getByRole('region', { name: 'Table: Sample, od600_t0' });
      expect(region).toHaveAttribute('data-overflow', 'true');
      // Scrolled to the end: 500 + 400 = 900, the last column in full, so no fade over it.
      region.scrollLeft = 500;
      fireEvent.scroll(region);
      expect(region).not.toHaveAttribute('data-overflow');
      // Still a region a keyboard can scroll back in.
      expect(region).toHaveAttribute('tabindex', '0');
      region.scrollLeft = 120;
      fireEvent.scroll(region);
      expect(region).toHaveAttribute('data-overflow', 'true');
    });

    it('does the same for a code block, named for its language', () => {
      layoutWidths(false);
      const { container, unmount } = render(<MessageBody body={'```r\nsum(x)\n```'} />);
      expect(container.querySelector('pre')).not.toHaveAttribute('tabindex');
      expect(screen.queryByRole('region')).toBeNull();
      unmount();
      layoutWidths(true);
      render(<MessageBody body={'```r\nsum(x)\n```'} />);
      expect(screen.getByRole('region', { name: 'Code: r' })).toHaveAttribute('tabindex', '0');
    });
  });

  it('accepts only absolute http(s) and mailto hrefs', () => {
    expect(safeExternalHref('https://a.example/x')).toBe('https://a.example/x');
    expect(safeExternalHref(' HTTP://A.example ')).toBe('http://a.example/');
    expect(safeExternalHref('mailto:x@y.z')).toBe('mailto:x@y.z');
    expect(safeExternalHref('mailto:x@y.z', false)).toBeNull();
    for (const url of ['javascript:alert(1)', '//evil.example', '/etc/hosts', 'data:,x', 7, null]) {
      expect(safeExternalHref(url)).toBeNull();
    }
  });
});

/**
 * QA M3, M13 and SEC-9: every other Crew surface already neutralises a bidi override and a
 * zero-width character, and message bodies were the one raw surface left. Escapes are braced so
 * this file never holds the characters it tests.
 */
describe('hidden characters and direction', () => {
  const hiddenMarks = (container: HTMLElement) =>
    Array.from(container.querySelectorAll('.crew-md-hidden-char')).map((node) => node.textContent);

  it('shows a right-to-left override as its escape instead of reversing the words', () => {
    const { container } = render(
      <MessageBody body={'Please open invoice_\u{202E}gnp.exe and dangling \u{202E}override'} />
    );
    expect(hiddenMarks(container)).toEqual(['\\u{202e}', '\\u{202e}']);
    // No live override is left anywhere in what is drawn.
    expect(container.textContent).not.toMatch(/[\u{202A}-\u{202E}\u{2066}-\u{2069}]/u);
    expect(container).toHaveTextContent('invoice_\\u{202e}gnp.exe');
    const mark = container.querySelector('.crew-md-hidden-char');
    expect(mark).toHaveAttribute('title', timelineCopy.hiddenCharacter('U+202E'));
    expect(mark).toHaveAttribute('dir', 'ltr');
  });

  it('shows a zero-width character that would make one handle look like another', () => {
    const { container } = render(
      <MessageBody body={'thanks @cre\u{200B}w_bob and cre\u{200B}w_alice'} />
    );
    expect(hiddenMarks(container)).toEqual(['\\u{200b}', '\\u{200b}']);
  });

  /**
   * A zero-width character in an element of its own (emphasis, strike-through, a link's words)
   * sits between the letters around the element, which draws no box: it is shown where the eye
   * would otherwise read `@crew_bob`, `bob@lab.org` or `ucsf.edu`.
   */
  it.each([
    ['emphasis', 'hi @crew_b*\u{200B}*ob', 'em'],
    ['strike-through', 'hi @crew_b~~\u{200B}~~ob', 'del'],
    ['a link', 'hi @crew_b[\u{2060}](https://a.bc)ob', 'a'],
    ['emphasis in an email address', 'bob@lab*\u{200B}*.org', 'em'],
    ['emphasis in a domain', 'visit ucsf*\u{2060}*.edu', 'em'],
  ])('shows a hidden character wrapped in %s', (_label, body, wrapper) => {
    const { container } = render(
      <MessageBody body={body} mention="crew_bob" mentionLabelId="mention-label" />
    );
    expect(container.querySelector(`${wrapper} .crew-md-hidden-char`)).not.toBeNull();
    expect(container.textContent).not.toMatch(/[\u{200B}\u{2060}]/u);
    expect(container.querySelector('.crew-md-mention')).toBeNull();
  });

  it('keeps an emoji joiner sequence whole and a Hebrew paragraph as written', () => {
    const scientist = '\u{1F469}\u{1F3FD}\u{200D}\u{1F52C}';
    const hebrew = 'שלום לכולם, הפגישה בשעה 3.';
    const { container } = render(<MessageBody body={`${scientist} done\n\n${hebrew}`} />);
    expect(container.querySelector('.crew-md-hidden-char')).toBeNull();
    expect(container.textContent).toContain(scientist);
    expect(container.textContent).toContain(hebrew);
  });

  it('lays out each block in its own direction, not one direction for the whole message', () => {
    const { container } = render(
      <MessageBody
        body={[
          'English first.',
          'שלום לכולם.',
          '# כותרת',
          '- פריט',
          '> ציטוט',
          '| עמודה |\n| --- |\n| תא |',
        ].join('\n\n')}
      />
    );
    for (const selector of [
      '.crew-md-p',
      '.crew-md-heading',
      '.crew-md-item',
      '.crew-md-quote',
      'th',
      'td',
    ]) {
      expect(container.querySelector(selector), selector).toHaveAttribute('dir', 'auto');
    }
    // Only the blocks carry a direction: the wrapper would take the first paragraph's for all.
    expect(container.querySelector('.crew-md')).not.toHaveAttribute('dir');
    expect(container.querySelector('.crew-message-body')).not.toHaveAttribute('dir');
  });

  it('draws a code block’s hidden characters, and Copy code still copies the bytes sent', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');
    const code = 'if user == "admin\u{202E} \u{2066}// check\u{2069}\u{2066}":';
    const { container } = render(<MessageBody body={`\`\`\`py\n${code}\n\`\`\``} />);
    const pre = container.querySelector('pre') as HTMLElement;
    expect(pre.textContent).not.toMatch(/[\u{202A}-\u{202E}\u{2066}-\u{2069}]/u);
    expect(pre.querySelectorAll('.crew-md-hidden-char')).toHaveLength(4);
    await user.click(screen.getByRole('button', { name: timelineCopy.copyCode }));
    expect(writeText).toHaveBeenCalledWith(code);
  });

  /**
   * react-markdown turns raw HTML into text only as it builds the elements, after the body step,
   * so an HTML block, a tag's attribute or a comment kept its hidden characters live.
   */
  it.each([
    ['an HTML block', '<div>\nOpen invoice_\u{202E}gnp.exe now\n</div>', '<div>'],
    ['an inline tag’s attribute', 'Open invoice_<x a="\u{202E}gnp.exe"> now', '<x a="'],
    ['an HTML comment', 'Look <!-- \u{202E} --> here', '<!-- '],
  ])(
    'shows the hidden characters of %s, which is drawn as the characters typed',
    (_label, body, typed) => {
      const { container } = render(<MessageBody body={body} />);
      expect(container.textContent).not.toMatch(/[\u{202A}-\u{202E}\u{2066}-\u{2069}]/u);
      expect(hiddenMarks(container)).toEqual(['\\u{202e}']);
      expect(container).toHaveTextContent(typed);
    }
  );

  it('names a table region with the escapes, never the raw controls', () => {
    const { container } = render(<MessageBody body={'| a\u{202E}b |\n| --- |\n| 1 |'} />);
    expect(container.querySelector('th')?.textContent).toBe('a\\u{202e}b');
  });
});

/** QA M2: a message that mentions you looked like any other unread message. */
describe('a mention of the viewer', () => {
  const chips = (container: HTMLElement) =>
    Array.from(container.querySelectorAll('.crew-md-mention')).map((node) => node.textContent);
  const label = (container: HTMLElement) => container.querySelector('#mention-label');

  it('marks @username in prose, in any case, and adds the row’s hidden label', () => {
    const { container } = render(
      <MessageBody
        body={'@crew_bob can you check? cc @CREW_BOB.'}
        mention="crew_bob"
        mentionLabelId="mention-label"
      />
    );
    expect(chips(container)).toEqual(['@crew_bob', '@CREW_BOB']);
    expect(container.querySelector('.crew-md-mention')).toHaveAttribute('data-mention', 'you');
    expect(label(container)).toHaveTextContent(timelineCopy.mentionsYou);
    expect(label(container)).not.toBeVisible();
  });

  it('does not mark a mention inside code, a code block or a link’s words', () => {
    const { container } = render(
      <MessageBody
        body={
          'Run `notify @crew_bob` then\n\n```\n@crew_bob\n```\n\n[@crew_bob](https://www.ucsf.edu)'
        }
        mention="crew_bob"
        mentionLabelId="mention-label"
      />
    );
    expect(chips(container)).toEqual([]);
    expect(label(container)).toBeNull();
  });

  it('does not mark someone else, a longer name, an address or a spoofed name', () => {
    const { container } = render(
      <MessageBody
        body={
          '@crew_alice and @crew_bobby and @crew_bob.lee wrote to crew@crew_bob.org about @cre\u{200B}w_bob and @crew_bob\u{200B}x'
        }
        mention="crew_bob"
        mentionLabelId="mention-label"
      />
    );
    expect(chips(container)).toEqual([]);
    expect(label(container)).toBeNull();
  });

  it('marks nothing without a username', () => {
    const { container } = render(<MessageBody body={'@crew_bob'} />);
    expect(chips(container)).toEqual([]);
  });
});

describe('the long-message fold', () => {
  it('folds a long body behind Show more with its size, and unfolds it', async () => {
    const body = 'word '.repeat(Math.ceil(CLAMP_CHAR_THRESHOLD / 5) + 20).trim();
    const { container } = render(<MessageBody body={body} />);
    const clamp = container.querySelector('.crew-message-body');
    expect(clamp).toHaveAttribute('data-clamped', 'true');
    const more = screen.getByRole('button', { name: timelineCopy.showMore });
    expect(more).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByText(/\d+ words/)).toBeInTheDocument();
    await userEvent.click(more);
    expect(clamp).not.toHaveAttribute('data-clamped');
    expect(screen.getByRole('button', { name: timelineCopy.showLess })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
  });

  it('leaves a short body alone', () => {
    const { container } = render(<MessageBody body="Counts are in." />);
    expect(container.querySelector('[data-clamped]')).toBeNull();
    expect(screen.queryByRole('button', { name: timelineCopy.showMore })).toBeNull();
  });
});
