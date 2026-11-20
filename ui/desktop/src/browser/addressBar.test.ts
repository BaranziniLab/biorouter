import { describe, expect, it } from 'vitest';
import { addressFieldTarget, DEFAULT_SEARCH_ENGINE } from './addressBar';

const search = (text: string) =>
  `https://www.google.com/search?q=${encodeURIComponent(text.trim())}`;

describe('the Biorouter Browser address field', () => {
  it.each([
    ['https://example.com/a?b=1', 'https://example.com/a?b=1'],
    ['HTTP://Example.com', 'http://example.com/'],
    ['example.com', 'https://example.com/'],
    ['en.wikipedia.org/wiki/Main_Page', 'https://en.wikipedia.org/wiki/Main_Page'],
    ['pubmed.ncbi.nlm.nih.gov/?term=ms', 'https://pubmed.ncbi.nlm.nih.gov/?term=ms'],
    ['github.com/login', 'https://github.com/login'],
    ['  www.ucsf.edu  ', 'https://www.ucsf.edu/'],
  ])('loads %s as %s', (input, url) => {
    expect(addressFieldTarget(input)).toEqual({ kind: 'url', url });
  });

  it.each([
    ['localhost:8888', 'http://localhost:8888/'],
    ['localhost', 'http://localhost/'],
    ['app.localhost:3000/x', 'http://app.localhost:3000/x'],
    ['127.0.0.1:5173', 'http://127.0.0.1:5173/'],
    ['192.168.1.10/admin', 'http://192.168.1.10/admin'],
    ['[::1]:8000', 'http://[::1]:8000/'],
    ['jupyter:8888/lab', 'http://jupyter:8888/lab'],
  ])('loads the local address %s over http', (input, url) => {
    expect(addressFieldTarget(input)).toEqual({ kind: 'url', url });
  });

  it.each([
    'multiple sclerosis genes',
    'pubmed',
    '1.2',
    'what is example.com',
    'file:///etc/passwd',
    'javascript:alert(1)',
    'data:text/html,<b>x</b>',
    'mailto:someone@example.com',
    'about:blank',
    'https://user:pass@example.com/',
  ])('searches for %s instead of loading it', (input) => {
    expect(addressFieldTarget(input)).toEqual({ kind: 'search', url: search(input) });
  });

  it('answers nothing for empty input', () => {
    expect(addressFieldTarget('')).toBeNull();
    expect(addressFieldTarget('   ')).toBeNull();
  });

  it('uses another search engine when one is given', () => {
    expect(
      addressFieldTarget('rna seq', {
        name: 'DuckDuckGo',
        template: 'https://duckduckgo.com/?q={searchTerms}',
      })
    ).toEqual({ kind: 'search', url: 'https://duckduckgo.com/?q=rna%20seq' });
    // A template without the placeholder falls back to the default rather than dropping the query.
    expect(addressFieldTarget('x', { name: 'Broken', template: 'https://bad.example/' })?.url).toBe(
      DEFAULT_SEARCH_ENGINE.template.replace('{searchTerms}', 'x')
    );
  });

  it('caps what an oversized input can send to the search engine', () => {
    const target = addressFieldTarget('a b '.repeat(5000));
    expect(target?.kind).toBe('search');
    expect(target!.url.length).toBeLessThan(8000);
  });
});
