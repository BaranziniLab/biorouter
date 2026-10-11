import { describe, it, expect } from 'vitest';
import { containsHTML, wrapHTMLInCodeBlock } from '../utils/htmlSecurity';

describe('HTML Security Detection', () => {
  describe('containsHTML', () => {
    describe('should detect dangerous HTML tags', () => {
      it('detects script tags', () => {
        expect(containsHTML('<script>alert("xss")</script>')).toBe(true);
        expect(containsHTML('<script src="evil.js"></script>')).toBe(true);
        expect(containsHTML('<script>')).toBe(true);
      });

      it('detects style tags', () => {
        expect(containsHTML('<style>body { display: none; }</style>')).toBe(true);
        expect(containsHTML('<style>')).toBe(true);
      });

      it('detects iframe tags', () => {
        expect(containsHTML('<iframe src="evil.com"></iframe>')).toBe(true);
        expect(containsHTML('<iframe>')).toBe(true);
      });

      it('detects form elements', () => {
        // <form> is in the hardened allowlist (it can submit data) → still flagged.
        expect(containsHTML('<form action="/submit"></form>')).toBe(true);
        // <input>/<button> were dropped by the security-hardening pass: they cannot
        // execute on their own and react-markdown escapes them when rendered, so the
        // narrowed regex intentionally no longer flags them.
        expect(containsHTML('<input type="text" name="password">')).toBe(false);
        expect(containsHTML('<button onclick="evil()">Click</button>')).toBe(false);
      });

      it('ignores benign layout-only tags after hardening', () => {
        // The dangerous-HTML regex was narrowed to the 8 tags that execute or
        // restructure the document (script|style|iframe|object|embed|form|link|meta|base).
        // Plain layout tags are harmless (react-markdown escapes raw HTML by default),
        // so they are intentionally no longer flagged.
        expect(containsHTML('<div class="container">content</div>')).toBe(false);
        expect(containsHTML('<span style="color:red">text</span>')).toBe(false);
        expect(containsHTML('<br/>')).toBe(false);
        expect(containsHTML('<hr>')).toBe(false);
        expect(containsHTML('<img src="image.jpg" alt="test">')).toBe(false);
      });

      it('detects HTML comments', () => {
        expect(containsHTML('<!-- this is a comment -->')).toBe(true);
        expect(containsHTML('<!-- multi\nline\ncomment -->')).toBe(true);
      });
    });

    describe('should NOT detect safe content', () => {
      it('ignores auto-links', () => {
        expect(containsHTML('<https://example.com>')).toBe(false);
        expect(containsHTML('<http://test.org>')).toBe(false);
        expect(containsHTML('<https://docs.example.org/guide>')).toBe(false);
      });

      it('ignores email addresses', () => {
        expect(containsHTML('<user@example.com>')).toBe(false);
        expect(containsHTML('<admin@example.org>')).toBe(false);
        expect(containsHTML('<test.email+tag@domain.co.uk>')).toBe(false);
      });

      it('ignores TypeScript generics and placeholders', () => {
        expect(containsHTML('Array<T>')).toBe(false);
        expect(containsHTML('Promise<string>')).toBe(false);
        expect(containsHTML('<project-root>')).toBe(false);
        expect(containsHTML('<filename>')).toBe(false);
        expect(containsHTML('<<not a tag>>')).toBe(false);
      });

      it('ignores content already in code blocks', () => {
        expect(containsHTML('```html\n<div>safe</div>\n```')).toBe(false);
        expect(containsHTML('`<script>safe</script>`')).toBe(false);
        expect(containsHTML('Here is `<br/>` in inline code')).toBe(false);
      });

      it('ignores plain text', () => {
        expect(containsHTML('This is just plain text')).toBe(false);
        expect(containsHTML('No HTML here!')).toBe(false);
        expect(containsHTML('')).toBe(false);
      });

      it('ignores mathematical expressions', () => {
        expect(containsHTML('x < y && y > z')).toBe(false);
        expect(containsHTML('if (a < b && c > d)')).toBe(false);
      });
    });

    describe('edge cases', () => {
      it('handles mixed content correctly', () => {
        // Dangerous HTML (script) mixed with an auto-link → still flagged.
        expect(containsHTML('Visit <https://example.com> and <script>alert(1)</script>')).toBe(
          true
        );

        // Auto-link mixed with a now-benign layout tag (<div>) → not flagged after hardening.
        expect(containsHTML('Visit <https://example.com> and <div>click here</div>')).toBe(false);

        // Only safe content
        expect(containsHTML('Email <user@test.com> about <project-root> setup')).toBe(false);
      });

      it('handles malformed HTML', () => {
        expect(containsHTML('<div unclosed')).toBe(false); // This doesn't match our regex pattern
        expect(containsHTML('<>')).toBe(false);
        expect(containsHTML('< div >')).toBe(false);
      });
    });
  });

  describe('wrapHTMLInCodeBlock', () => {
    describe('should wrap dangerous HTML', () => {
      it('wraps single line HTML', () => {
        const input = '<script>alert("xss")</script>';
        const expected = '```html\n<script>alert("xss")</script>\n```';
        expect(wrapHTMLInCodeBlock(input)).toBe(expected);
      });

      it('wraps HTML comments', () => {
        const input = '<!-- malicious comment -->';
        const expected = '```html\n<!-- malicious comment -->\n```';
        expect(wrapHTMLInCodeBlock(input)).toBe(expected);
      });

      it('wraps mixed content selectively', () => {
        // After hardening only dangerous tags are wrapped; use <iframe> so the line
        // is actually fenced. A benign <div> line would be left untouched.
        const input = `Normal text
<iframe src="evil.com"></iframe>
More normal text`;

        const expected = `Normal text
\`\`\`html
<iframe src="evil.com"></iframe>
\`\`\`
More normal text`;

        expect(wrapHTMLInCodeBlock(input)).toBe(expected);
      });
    });

    describe('should preserve safe content', () => {
      it('preserves auto-links', () => {
        const input = 'Visit <https://example.com> for more info';
        expect(wrapHTMLInCodeBlock(input)).toBe(input);
      });

      it('preserves email addresses', () => {
        const input = 'Contact <admin@example.com> for help';
        expect(wrapHTMLInCodeBlock(input)).toBe(input);
      });

      it('preserves TypeScript generics', () => {
        const input = 'const arr: Array<string> = []';
        expect(wrapHTMLInCodeBlock(input)).toBe(input);
      });

      it('preserves existing code blocks', () => {
        const input = `# Title

\`\`\`javascript
const x = "<div>this is safe</div>";
\`\`\`

Normal text`;

        expect(wrapHTMLInCodeBlock(input)).toBe(input);
      });

      it('preserves inline code', () => {
        const input = 'Use `<br/>` for line breaks';
        expect(wrapHTMLInCodeBlock(input)).toBe(input);
      });
    });

    describe('complex scenarios', () => {
      it('handles multiple HTML lines correctly', () => {
        // Use dangerous tags (script/iframe) so both lines are wrapped after hardening;
        // benign layout tags like <div>/<span> are intentionally left untouched now.
        const input = `# Test Message

Normal paragraph

<script>First HTML line</script>
<iframe>Second HTML line</iframe>

More normal text`;

        const expected = `# Test Message

Normal paragraph

\`\`\`html
<script>First HTML line</script>
\`\`\`
\`\`\`html
<iframe>Second HTML line</iframe>
\`\`\`

More normal text`;

        expect(wrapHTMLInCodeBlock(input)).toBe(expected);
      });

      it('respects existing code block boundaries', () => {
        // The outside-the-fence tag must be dangerous to get wrapped after hardening;
        // a benign <div> there would (correctly) be left as-is. Content inside the
        // existing fence is always left untouched regardless of the tag.
        const input = `Before code block

\`\`\`html
<div>This is already safe</div>
<script>This is also safe in here</script>
\`\`\`

<script>This should be wrapped</script>`;

        const expected = `Before code block

\`\`\`html
<div>This is already safe</div>
<script>This is also safe in here</script>
\`\`\`

\`\`\`html
<script>This should be wrapped</script>
\`\`\``;

        expect(wrapHTMLInCodeBlock(input)).toBe(expected);
      });

      it('handles the test suite scenarios', () => {
        // Test Message 1: One-liners
        const test1 = `<https://example.com>
<user@example.com>
\`<T>\``;
        expect(wrapHTMLInCodeBlock(test1)).toBe(test1);

        // Test Message 2: Mixed content with a now-benign <div> tag. After the
        // security-hardening narrowed the regex, a line whose only "HTML" is a
        // layout tag like <div> is no longer detected, so it is left unchanged.
        const test2 = `Here's a link <https://example.com> and HTML <div>content</div>`;
        expect(wrapHTMLInCodeBlock(test2)).toBe(test2);

        // Test Message 2b: same shape but with a dangerous tag → the whole line is
        // wrapped (we fence the entire line, not just the HTML part).
        const test2b = `Here's a link <https://example.com> and HTML <iframe>content</iframe>`;
        const expected2b = `\`\`\`html
Here's a link <https://example.com> and HTML <iframe>content</iframe>
\`\`\``;
        expect(wrapHTMLInCodeBlock(test2b)).toBe(expected2b);

        // Test Message 7: Comment-only
        const test7 = `<!-- top-level html comment -->`;
        const expected7 = `\`\`\`html
<!-- top-level html comment -->
\`\`\``;
        expect(wrapHTMLInCodeBlock(test7)).toBe(expected7);
      });
    });

    describe('edge cases', () => {
      it('handles empty input', () => {
        expect(wrapHTMLInCodeBlock('')).toBe('');
      });

      it('handles only whitespace', () => {
        const input = '   \n  \n  ';
        expect(wrapHTMLInCodeBlock(input)).toBe(input);
      });

      it('handles nested code block scenarios', () => {
        // The line between the two fences must be a dangerous tag to get wrapped
        // after hardening (a benign <div> there would be left untouched). Tags
        // inside the fences are always preserved verbatim.
        const input = `\`\`\`
<div>safe in code block</div>
\`\`\`
<script>unsafe outside</script>
\`\`\`
<span>also safe in code block</span>
\`\`\``;

        const expected = `\`\`\`
<div>safe in code block</div>
\`\`\`
\`\`\`html
<script>unsafe outside</script>
\`\`\`
\`\`\`
<span>also safe in code block</span>
\`\`\``;

        expect(wrapHTMLInCodeBlock(input)).toBe(expected);
      });
    });
  });
});
