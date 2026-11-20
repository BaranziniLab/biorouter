import { describe, expect, it } from 'vitest';
import { READ_AS_MARKDOWN_FUNCTION, readAsMarkdown } from './readPage';

/**
 * `browser_read` (BB-14): the page as Markdown, without the values of form fields (BB-13). Run
 * against jsdom here; the Electron spec runs the same function in Chromium.
 */

function read(html: string, max = 20_000, selector?: string) {
  document.body.innerHTML = html;
  const target = selector ? document.querySelector(selector) : undefined;
  return readAsMarkdown.call(target ?? undefined, max);
}

describe('readAsMarkdown', () => {
  it('turns headings, paragraphs, lists, links and emphasis into Markdown', () => {
    const { text } = read(
      `<h1>Electron</h1><p>Electron is a <b>framework</b> by <a href="https://github.com/">GitHub</a>.</p>
       <ul><li>One</li><li>Two<ol><li>Nested</li></ol></li></ul>`
    );
    expect(text).toContain('# Electron');
    expect(text).toContain('Electron is a **framework** by [GitHub](https://github.com/).');
    expect(text).toContain('- One');
    expect(text).toContain('- Two');
    expect(text).toContain('  1. Nested');
  });

  it('reads a table as a Markdown table', () => {
    const { text } = read(
      `<table><tr><th>Gene</th><th>Score</th></tr><tr><td>HLA-DRB1</td><td>0.92</td></tr><tr><td>IL7R</td><td>0.4|1</td></tr></table>`
    );
    expect(text.split('\n')).toEqual([
      '| Gene | Score |',
      '| --- | --- |',
      '| HLA-DRB1 | 0.92 |',
      '| IL7R | 0.4\\|1 |',
    ]);
  });

  it('never reads form field values, scripts or hidden text', () => {
    const { text } = read(
      `<form><label>Password <input type="password" value="hunter2"></label>
       <textarea>draft secret</textarea><select><option>Hidden choice</option></select></form>
       <script>var token = "abc";</script><p hidden>invisible</p><p aria-hidden="true">also hidden</p><p>Visible</p>`
    );
    expect(text).not.toContain('hunter2');
    expect(text).not.toContain('draft secret');
    expect(text).not.toContain('Hidden choice');
    expect(text).not.toContain('token');
    expect(text).not.toContain('invisible');
    expect(text).not.toContain('also hidden');
    expect(text).toContain('Password');
    expect(text).toContain('Visible');
  });

  it('reads one element when called on it, and stops at the limit', () => {
    const scoped = read('<p>Outside</p><section id="s"><p>Inside</p></section>', 20_000, '#s');
    expect(scoped.text).toBe('Inside');
    const long = read(`<p>${'word '.repeat(5000)}</p>`, 500);
    expect(long.truncated).toBe(true);
    expect(long.text.length).toBeLessThanOrEqual(500);
  });

  it('is self-contained source, so it can run inside the page', () => {
    const fn = new Function(`return (${READ_AS_MARKDOWN_FUNCTION})`)() as typeof readAsMarkdown;
    document.body.innerHTML = '<h2>Hi</h2><p>there</p>';
    expect(fn.call(undefined, 1000).text).toBe('## Hi\n\nthere');
  });
});
