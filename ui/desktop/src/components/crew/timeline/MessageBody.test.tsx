import { fireEvent, render, screen } from '@testing-library/react';
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

  it('links only http, https and mailto, and opens them in the system browser', () => {
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
    expect(links.map((link) => link.textContent)).toEqual(['docs', 'mail']);
    expect(container.innerHTML).not.toMatch(/javascript:|data:text|file:\/\//);
    expect(container).toHaveTextContent('bad');

    fireEvent.click(screen.getByRole('link', { name: 'docs' }));
    expect(openExternal).toHaveBeenCalledWith('https://biorouter.ucsf.edu/docs.html');
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

  it('renders a table in a scrollable region', () => {
    render(<MessageBody body={'| a | b |\n|---|---|\n| 1 | 2 |'} />);
    expect(screen.getByRole('region', { name: 'Scrollable table' })).toBeInTheDocument();
    expect(screen.getAllByRole('cell')).toHaveLength(2);
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
