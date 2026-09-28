import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CLAMP_CHAR_THRESHOLD } from '../../../utils/messageClamp';
import { timelineCopy } from './copy';
import { MessageBody, safeExternalHref } from './MessageBody';

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
    ])('names the real host after %s', (_label, body, host) => {
      const { container } = render(<MessageBody body={body} />);
      expect(container.querySelector('.crew-md-link-host')).toHaveTextContent(`(${host})`);
    });

    it.each([
      ['the same host', '[https://www.ucsf.edu/news](https://www.ucsf.edu/about)'],
      ['the same host with or without www', '[ucsf.edu](https://www.ucsf.edu/)'],
      ['words that are not an address', '[the lab docs](https://evil.example.net/)'],
      ['an autolinked address', 'https://www.ucsf.edu/news'],
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
