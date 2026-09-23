import { afterEach, describe, it, expect, vi } from 'vitest';
import { fireEvent, render } from '@testing-library/react';
import { screen, waitFor } from '@testing-library/dom';
import MarkdownContent from './MarkdownContent';
import {
  resetFileLinkStatusForTests,
  type FilePathCheckRequest,
  type FilePathCheckResult,
} from './artifacts/fileLinkStatus';

// Mock the icons to avoid import issues
vi.mock('./icons', () => ({
  Check: () => <div data-testid="check-icon">✓</div>,
  Copy: () => <div data-testid="copy-icon">📋</div>,
}));

function installElectronMock(
  overrides: Partial<{
    readArtifactFile: ReturnType<typeof vi.fn>;
    openExternal: ReturnType<typeof vi.fn>;
  }> = {}
) {
  const electron = {
    readArtifactFile:
      overrides.readArtifactFile ??
      vi.fn(async (path: string) => ({
        kind: 'image',
        title: path.split('/').pop() ?? 'image',
        path,
        mimeType: 'image/png',
        dataUrl: 'data:image/png;base64,AAAA',
        size: 3,
        found: true,
      })),
    openExternal: overrides.openExternal ?? vi.fn(async () => undefined),
  };
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: electron,
  });
  return electron;
}

describe('MarkdownContent', () => {
  describe('fenced blocks', () => {
    // The wrapper's inline-code recipe (`prose-code:bg-background-medium
    // px-1 py-0.5`) matched the highlighter's own <code>, painting a chip on
    // every line of every block and nudging the first line 4px right. The
    // typography plugin's element variants skip `.not-prose` subtrees; jsdom
    // runs no Tailwind, so the guard is that the opt-out reaches the DOM.
    it('opts the fenced block out of the inline-code recipe', async () => {
      const { container } = render(
        <MarkdownContent content={'```python\nimport numpy as np\n```'} />
      );
      await waitFor(() => expect(container.querySelector('.biorouter-md-code')).not.toBeNull());
      expect(container.querySelector('.biorouter-md-code')).toHaveClass('not-prose');
      expect(container.querySelector('.biorouter-md-code code .token')).not.toBeNull();
    });

    // ```{r setup} reaches the renderer as `language-{r`, which `\w+` alone
    // refused, so every R Markdown / Quarto chunk rendered as plain text.
    it.each(['chat', 'document'] as const)(
      'highlights an R Markdown chunk header (%s)',
      async (variant) => {
        const { container } = render(
          <MarkdownContent
            content={'```{r setup, include=FALSE}\nx <- TRUE\n```'}
            variant={variant}
          />
        );
        await waitFor(() => expect(container.querySelector('.biorouter-md-code')).not.toBeNull());
        expect(container.querySelector('.biorouter-md-code-lang')?.textContent).toBe('r');
        // Plain text renders no token spans; the R grammar does (`TRUE`, `<-`).
        expect(container.querySelector('.biorouter-md-code code .token')).not.toBeNull();
      }
    );

    // The header names the fence AS WRITTEN — chat upper-cases it in CSS — while
    // the grammar gets the normalised name Prism's case-sensitive registry needs.
    it("labels a block with the raw fence id and highlights with Prism's name", async () => {
      const { container } = render(
        <MarkdownContent content={'```Python3\ndef f(x):\n    return x\n```'} />
      );
      await waitFor(() => expect(container.querySelector('.biorouter-md-code')).not.toBeNull());
      expect(container.querySelector('.biorouter-md-code-lang')?.textContent).toBe('Python3');
      // Highlighted as Python (the theme strips the `keyword` class, so find the span).
      const def = [...container.querySelectorAll('.biorouter-md-code code .token')].find(
        (span) => span.textContent === 'def'
      );
      expect(def).toBeDefined();
    });

    // A document keeps a long line and lets the well scroll; chat wraps to the bubble.
    it('wraps long lines in chat and keeps them in a document', async () => {
      const content = '```sh\necho ' + 'x'.repeat(300) + '\n```';
      const { container, rerender } = render(<MarkdownContent content={content} />);
      await waitFor(() =>
        expect(container.querySelector('.biorouter-md-code code')).not.toBeNull()
      );
      expect(
        container.querySelector<HTMLElement>('.biorouter-md-code code')!.style.whiteSpace
      ).toBe('pre-wrap');
      rerender(<MarkdownContent content={content} variant="document" />);
      await waitFor(() =>
        expect(
          container.querySelector<HTMLElement>('.biorouter-md-code code')!.style.whiteSpace
        ).toBe('pre')
      );
    });
  });

  describe('line breaks', () => {
    it('keeps chat hard breaks, and soft-wraps a document', async () => {
      const { container, rerender } = render(<MarkdownContent content={'one\ntwo'} />);
      await waitFor(() => expect(container.querySelector('p')).not.toBeNull());
      expect(container.querySelector('p br')).not.toBeNull();
      expect(container.firstElementChild).toHaveAttribute('data-variant', 'chat');
      rerender(<MarkdownContent content={'one\ntwo'} variant="document" />);
      await waitFor(() => expect(container.querySelector('p br')).toBeNull());
      expect(container.firstElementChild).toHaveAttribute('data-variant', 'document');
    });
  });

  describe('HTML Security Integration', () => {
    it('renders safe markdown content normally', async () => {
      const content = `# Test Title

Visit <https://example.com> for more info.

Contact <admin@example.com> for support.

Use \`Array<T>\` for generics.`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Test Title')).toBeInTheDocument();
        expect(screen.getByText(/Visit/)).toBeInTheDocument();
        expect(screen.getByText(/for more info/)).toBeInTheDocument();
        expect(screen.getByText(/Contact/)).toBeInTheDocument();
        expect(screen.getByText(/for support/)).toBeInTheDocument();
      });

      // Should not create extra code blocks for safe content
      const codeBlocks = screen.queryAllByText(/```html/);
      expect(codeBlocks).toHaveLength(0);
    });

    it('wraps dangerous HTML in code blocks', async () => {
      const content = `# Security Test

This is safe text.

<script>alert('xss')</script>

More safe text.`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Security Test')).toBeInTheDocument();
        expect(screen.getByText('This is safe text.')).toBeInTheDocument();
        expect(screen.getByText('More safe text.')).toBeInTheDocument();
      });

      // The script tag should be in a code block, not executed
      const scriptElements = document.querySelectorAll('script');
      expect(scriptElements).toHaveLength(0); // No actual script tags should be created

      // Should find the script content in a code block (text may be split across spans)
      await waitFor(() => {
        expect(screen.getByText(/alert/)).toBeInTheDocument();
        expect(screen.getByText(/xss/)).toBeInTheDocument();
      });
    });

    it('handles HTML comments securely', async () => {
      const content = `# Comment Test

<!-- This is a malicious comment -->

Normal text continues.`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Comment Test')).toBeInTheDocument();
        expect(screen.getByText('Normal text continues.')).toBeInTheDocument();
      });

      // Comment should be in a code block
      await waitFor(() => {
        expect(screen.getByText(/This is a malicious comment/)).toBeInTheDocument();
      });
    });

    it('preserves existing code blocks', async () => {
      const content = `# Code Block Test

\`\`\`javascript
const html = "<div>This is safe in a code block</div>";
console.log(html);
\`\`\`

<div>This should be wrapped</div>`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Code Block Test')).toBeInTheDocument();
      });

      // Should preserve the original JavaScript code block (text may be split)
      await waitFor(() => {
        expect(screen.getByText(/const/)).toBeInTheDocument();
        expect(screen.getAllByText(/html/)).toHaveLength(2); // Variable name and function parameter
      });

      // The div outside the code block should be wrapped
      await waitFor(() => {
        expect(screen.getByText(/This should be wrapped/)).toBeInTheDocument();
      });
    });

    it('handles mixed safe and unsafe content', async () => {
      const content = `# Mixed Content Test

1. Auto-link: <https://block.dev>
2. Inline code: \`const x = Array<T>();\`
3. Real markup: <input type="text" disabled>
4. Placeholder path: <project-root>/src`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Mixed Content Test')).toBeInTheDocument();
        expect(screen.getByText(/Auto-link/)).toBeInTheDocument();
        expect(screen.getByText(/Inline code/)).toBeInTheDocument();
        expect(screen.getByText(/Real markup/)).toBeInTheDocument();
        expect(screen.getByText(/Placeholder path/)).toBeInTheDocument();
      });

      // Only the input tag should be wrapped
      await waitFor(() => {
        expect(screen.getByText(/input/)).toBeInTheDocument();
        expect(screen.getByText(/type/)).toBeInTheDocument();
        expect(screen.getByText(/disabled/)).toBeInTheDocument();
      });

      // Should not have actual input elements in the DOM
      const inputElements = document.querySelectorAll('input');
      expect(inputElements).toHaveLength(0);
    });
  });

  describe('Code Block Functionality', () => {
    it('renders code blocks with syntax highlighting', async () => {
      const content = `\`\`\`javascript
console.log('Hello, World!');
\`\`\``;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText(/console/)).toBeInTheDocument();
        expect(screen.getByText(/log/)).toBeInTheDocument();
        expect(screen.getByText(/Hello, World!/)).toBeInTheDocument();
      });
    });

    it('renders inline code', async () => {
      const content = 'Use `console.log()` to debug.';

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText(/Use/)).toBeInTheDocument();
        expect(screen.getByText(/to debug/)).toBeInTheDocument();
        expect(screen.getByText('console.log()')).toBeInTheDocument();
      });
    });
  });

  // "Run" hands a shell code block to this chat's in-app terminal. The whole
  // affordance is OPT-IN: eleven surfaces mount MarkdownContent and only one is
  // a live chat, so the default must be a transcript with no shell in it.
  describe('Run in the terminal', () => {
    const fence = (language: string, body: string) => ['```' + language, body, '```'].join('\n');

    async function findRunButton() {
      return screen.findByRole('button', { name: /^run$/i });
    }

    it('offers Run on a shell block when the surface can run one', async () => {
      render(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={vi.fn(() => true)} />
      );

      expect(await findRunButton()).toBeInTheDocument();
    });

    it('hands the command over on click', async () => {
      const onRunInTerminal = vi.fn(() => true);
      render(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={onRunInTerminal} />
      );

      fireEvent.click(await findRunButton());

      expect(onRunInTerminal).toHaveBeenCalledExactlyOnceWith('ls -la');
    });

    it('keeps Copy beside it — the old path is not replaced', async () => {
      render(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={vi.fn(() => true)} />
      );

      await findRunButton();
      expect(screen.getByRole('button', { name: /^copy$/i })).toBeInTheDocument();
    });

    it('confirms the hand-off, since the terminal may be below the fold', async () => {
      render(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={vi.fn(() => true)} />
      );

      fireEvent.click(await findRunButton());

      expect(await screen.findByRole('button', { name: /^sent$/i })).toBeInTheDocument();
    });

    /**
     * "Sent" is a claim about what happened, not about what was clicked.
     *
     * The pane refuses when its shell has exited — the bytes would go into a
     * closed pty and vanish — and the button used to answer that with a tick
     * and the word "Sent" regardless, because the handler returned `void` and
     * there was nothing to read.
     */
    it('does NOT claim "Sent" when the terminal refuses the command', async () => {
      render(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={vi.fn(() => false)} />
      );

      fireEvent.click(await findRunButton());

      expect(await screen.findByRole('button', { name: /terminal closed/i })).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /^sent$/i })).not.toBeInTheDocument();
    });

    it('is ABSENT by default — the eleven other mount sites have no terminal', async () => {
      render(<MarkdownContent content={fence('bash', 'ls -la')} />);

      expect(await screen.findByRole('button', { name: /^copy$/i })).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /^run$/i })).not.toBeInTheDocument();
    });

    it('is absent on a non-shell block', async () => {
      render(
        <MarkdownContent
          content={fence('rust', 'fn main() {}')}
          onRunInTerminal={vi.fn(() => true)}
        />
      );

      expect(await screen.findByRole('button', { name: /^copy$/i })).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /^run$/i })).not.toBeInTheDocument();
    });

    it('is absent on a shell TRANSCRIPT, whose body is a prompt plus output', async () => {
      // The trap the full-identifier read exists to close: `/language-(\w+)/`
      // stops at the hyphen, so this block's display language is `shell` — which
      // IS runnable. Running it would execute the prompt character and then two
      // lines of `ls` output.
      render(
        <MarkdownContent
          content={fence('shell-session', '$ ls\nREADME.md\nsrc')}
          onRunInTerminal={vi.fn(() => true)}
        />
      );

      expect(await screen.findByRole('button', { name: /^copy$/i })).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /^run$/i })).not.toBeInTheDocument();
    });

    it('is present on a plain ```shell block, which is a command', async () => {
      render(
        <MarkdownContent content={fence('shell', 'ls -la')} onRunInTerminal={vi.fn(() => true)} />
      );

      expect(await findRunButton()).toBeInTheDocument();
    });

    it('is absent on an empty shell block', async () => {
      render(
        <MarkdownContent content={fence('bash', '   ')} onRunInTerminal={vi.fn(() => true)} />
      );

      expect(await screen.findByRole('button', { name: /^copy$/i })).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /^run$/i })).not.toBeInTheDocument();
    });

    it('hands over a multi-line block whole, interior indentation intact', async () => {
      const onRunInTerminal = vi.fn(() => true);
      const script = 'for f in *.txt; do\n  echo "$f"\ndone';
      render(<MarkdownContent content={fence('bash', script)} onRunInTerminal={onRunInTerminal} />);

      fireEvent.click(await findRunButton());

      expect(onRunInTerminal).toHaveBeenCalledExactlyOnceWith(script);
    });

    it('keeps its confirmation across a parent re-render with a STABLE callback', async () => {
      // Measured in a real browser: an unstable `onRunInTerminal` gives
      // ReactMarkdown a new `components.code` identity every render, and React
      // treats a new component type as a different component — so the whole
      // code-block subtree unmounts and remounts, taking this state (and
      // Copy's "Copied", which has always worked the same way) with it.
      //
      // BaseChat's handler is useCallback-stable for exactly this reason. The
      // assertion is here rather than there because this is where an unstable
      // callback would show itself: a transcript whose blocks are torn down and
      // rebuilt on every streaming frame.
      const onRunInTerminal = vi.fn(() => true);
      const { rerender } = render(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={onRunInTerminal} />
      );

      fireEvent.click(await findRunButton());
      await screen.findByRole('button', { name: /^sent$/i });

      rerender(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={onRunInTerminal} />
      );

      expect(screen.getByRole('button', { name: /^sent$/i })).toBeInTheDocument();
    });

    it('still shows the language label and the code itself', async () => {
      render(
        <MarkdownContent content={fence('bash', 'ls -la')} onRunInTerminal={vi.fn(() => true)} />
      );

      await findRunButton();
      expect(screen.getByText('bash')).toBeInTheDocument();
    });
  });

  describe('Markdown Features', () => {
    it('renders headers correctly', async () => {
      const content = `# H1 Header
## H2 Header
### H3 Header`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByRole('heading', { level: 1, name: 'H1 Header' })).toBeInTheDocument();
        expect(screen.getByRole('heading', { level: 2, name: 'H2 Header' })).toBeInTheDocument();
        expect(screen.getByRole('heading', { level: 3, name: 'H3 Header' })).toBeInTheDocument();
      });
    });

    it('renders lists correctly', async () => {
      const content = `- Item 1
- Item 2
- Item 3

1. Numbered 1
2. Numbered 2`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Item 1')).toBeInTheDocument();
        expect(screen.getByText('Item 2')).toBeInTheDocument();
        expect(screen.getByText('Item 3')).toBeInTheDocument();
        expect(screen.getByText('Numbered 1')).toBeInTheDocument();
        expect(screen.getByText('Numbered 2')).toBeInTheDocument();
      });
    });

    it('renders links with correct attributes', async () => {
      const content = '[Visit Block](https://block.dev)';

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        const link = screen.getByRole('link', { name: 'Visit Block' });
        expect(link).toBeInTheDocument();
        expect(link).toHaveAttribute('href', 'https://block.dev');
        expect(link).toHaveAttribute('target', '_blank');
        expect(link).toHaveAttribute('rel', 'noopener noreferrer');
      });
    });

    it('opens previewable file links as read-only artifacts', async () => {
      const onOpenArtifact = vi.fn();
      const content = '[analysis.sql](/Users/wgu/project/analysis.sql)';

      render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);

      await waitFor(() => {
        fireEvent.click(screen.getByRole('button', { name: 'analysis.sql' }));
      });

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'analysis.sql',
        path: '/Users/wgu/project/analysis.sql',
      });
      expect(screen.queryByRole('link', { name: 'analysis.sql' })).not.toBeInTheDocument();
    });

    it.each([
      '[Report](reports/summary.md)',
      'Open `reports/summary.md`.',
      'Open reports/summary.md.',
    ])('file-link reliability: refuses an unanchored relative file: %s', async (content) => {
      const onOpenArtifact = vi.fn();
      render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);

      await waitFor(() => expect(screen.getByText(/Report|reports\/summary\.md/)).toBeVisible());
      expect(screen.queryByRole('button')).not.toBeInTheDocument();
      expect(screen.queryByRole('link')).not.toBeInTheDocument();
      expect(onOpenArtifact).not.toHaveBeenCalled();
    });

    it.each([
      '../private/report.md',
      'reports/../../private/report.md',
      '%2e%2e/private/report.md',
    ])(
      'file-link reliability: preserves relative-path refusal through Markdown: %s',
      async (target) => {
        const onOpenArtifact = vi.fn();
        render(
          <MarkdownContent
            content={`[Report](${target})`}
            workingDir="/work/session"
            onOpenArtifact={onOpenArtifact}
          />
        );

        expect(await screen.findByText('Report')).toBeVisible();
        expect(screen.queryByRole('button', { name: 'Report' })).not.toBeInTheDocument();
        expect(screen.queryByRole('link', { name: 'Report' })).not.toBeInTheDocument();
        expect(onOpenArtifact).not.toHaveBeenCalled();
      }
    );

    it.each(['/work/source.rs:42', '/work/source.rs#L42'])(
      'file-link reliability: separates the source location from file I/O: %s',
      async (target) => {
        const onOpenArtifact = vi.fn();
        render(<MarkdownContent content={`[Source](${target})`} onOpenArtifact={onOpenArtifact} />);

        fireEvent.click(await screen.findByRole('button', { name: 'Source' }));
        expect(onOpenArtifact).toHaveBeenCalledWith(
          expect.objectContaining({
            kind: 'file',
            title: 'source.rs',
            path: '/work/source.rs',
            line: 42,
          })
        );
      }
    );

    it.each([
      [[], '/work/source.rs'],
      [['/earlier/output/source.rs'], '/earlier/output/source.rs'],
    ] as const)(
      'file-link reliability: preserves a bare filename source line through URL filtering: %s',
      async (knownFilePaths, path) => {
        const onOpenArtifact = vi.fn();
        render(
          <MarkdownContent
            content="[Source](source.rs:42)"
            workingDir="/work"
            knownFilePaths={knownFilePaths}
            onOpenArtifact={onOpenArtifact}
          />
        );
        fireEvent.click(await screen.findByRole('button', { name: 'Source' }));
        expect(onOpenArtifact).toHaveBeenCalledWith({
          kind: 'file',
          title: 'source.rs',
          path,
          line: 42,
        });
      }
    );

    it.each(['See /work/Δ/source.rs:42.', 'See /work/Δ/source.rs#L42.'])(
      'file-link reliability: preserves the full Unicode path and location in prose: %s',
      async (content) => {
        const onOpenArtifact = vi.fn();
        render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);
        fireEvent.click(await screen.findByRole('button'));
        expect(onOpenArtifact).toHaveBeenCalledWith({
          kind: 'file',
          title: 'source.rs',
          path: '/work/Δ/source.rs',
          line: 42,
        });
      }
    );

    it.each([
      [String.raw`C:\Users\Ada\Project\report.csv`, 'report.csv'],
      [String.raw`\\server\share\reports\summary.md`, 'summary.md'],
    ])(
      'file-link reliability: opens a plain-prose Windows path exactly: %s',
      async (path, title) => {
        const onOpenArtifact = vi.fn();
        // Markdown uses `\\` to render one literal backslash, so preserve both UNC
        // introducer slashes in the prose that reaches the component override.
        const markdownPath = path.startsWith('\\\\') ? `\\\\${path}` : path;
        render(
          <MarkdownContent content={`Open ${markdownPath}.`} onOpenArtifact={onOpenArtifact} />
        );

        fireEvent.click(await screen.findByRole('button', { name: path }));
        expect(onOpenArtifact).toHaveBeenCalledWith({ kind: 'file', title, path });
      }
    );

    it('file-link reliability: never linkifies a root suffix after unsupported path characters', async () => {
      const onOpenArtifact = vi.fn();
      render(
        <MarkdownContent
          content={'See /work/odd"directory/report.md'}
          workingDir="/work"
          onOpenArtifact={onOpenArtifact}
        />
      );
      expect(await screen.findByText(/directory\/report.md/)).toBeVisible();
      expect(screen.queryByRole('button', { name: '/report.md' })).not.toBeInTheDocument();
      expect(onOpenArtifact).not.toHaveBeenCalled();
    });

    it.each([
      ['See /work/source.rs%23L42.', '/work/source.rs#L42'],
      ['[Literal](results/source.rs%23L42)', '/work/results/source.rs#L42'],
      ['[Literal](source.rs%3A42)', '/elsewhere/source.rs:42'],
      ['[Literal](results/source.rs%2523L42)', '/work/results/source.rs%23L42'],
    ])(
      'file-link reliability: resolves encoded literal names without source reinterpretation: %s',
      async (content, path) => {
        const onOpenArtifact = vi.fn();
        render(
          <MarkdownContent
            content={content}
            workingDir="/work"
            knownFilePaths={['/elsewhere/source.rs:42']}
            onOpenArtifact={onOpenArtifact}
          />
        );
        fireEvent.click(await screen.findByRole('button'));
        expect(onOpenArtifact).toHaveBeenCalledWith({
          kind: 'file',
          title: path.split('/').pop(),
          path,
        });
      }
    );

    it.each([
      '/tmp/source.rs:0',
      '/tmp/source.rs:9007199254740992',
      '/tmp/source.rs:42:7',
      '/tmp/source.rs#L42:7',
      '/tmp/source.rs%00',
      'source.rs:0',
    ])(
      'file-link reliability: malformed local targets stay inert instead of opening externally: %s',
      async (target) => {
        const onOpenArtifact = vi.fn();
        const electron = installElectronMock();
        render(<MarkdownContent content={`[Source](${target})`} onOpenArtifact={onOpenArtifact} />);
        expect(await screen.findByText('Source')).toBeVisible();
        expect(screen.queryByRole('button', { name: 'Source' })).not.toBeInTheDocument();
        expect(screen.queryByRole('link', { name: 'Source' })).not.toBeInTheDocument();
        expect(electron.openExternal).not.toHaveBeenCalled();
        expect(onOpenArtifact).not.toHaveBeenCalled();
      }
    );

    it.each([
      '/work/Study%20%231/%CE%94%20results.csv',
      'file:///work/Study%20%231/%CE%94%20results.csv',
      '</work/Study %231/Δ results.csv>',
    ])(
      'file-link reliability: preserves encoded hashes, Unicode and spaces: %s',
      async (target) => {
        const onOpenArtifact = vi.fn();
        render(
          <MarkdownContent content={`[Results](${target})`} onOpenArtifact={onOpenArtifact} />
        );

        fireEvent.click(await screen.findByRole('button', { name: 'Results' }));
        expect(onOpenArtifact).toHaveBeenCalledWith({
          kind: 'file',
          title: 'Δ results.csv',
          path: '/work/Study #1/Δ results.csv',
        });
      }
    );

    it.each([
      ['/work/source.rs%3A42', 'source.rs:42'],
      ['/work/source.rs%23L42', 'source.rs#L42'],
      ['file:///work/source.rs%23L42', 'source.rs#L42'],
    ])(
      'file-link reliability: preserves percent-encoded literal source-location characters: %s',
      async (target, name) => {
        const onOpenArtifact = vi.fn();
        render(
          <MarkdownContent content={`[Literal name](${target})`} onOpenArtifact={onOpenArtifact} />
        );

        fireEvent.click(await screen.findByRole('button', { name: 'Literal name' }));
        expect(onOpenArtifact).toHaveBeenCalledWith({
          kind: 'file',
          title: name,
          path: `/work/${name}`,
        });
      }
    );

    it('decodes markdown URL escapes before opening a local file with spaces', async () => {
      const onOpenArtifact = vi.fn();
      const content = '[Preview PowerPoint](/private/tmp/BioOKF%20Presentation.pptx)';

      render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);

      fireEvent.click(await screen.findByRole('button', { name: 'Preview PowerPoint' }));

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'BioOKF Presentation.pptx',
        path: '/private/tmp/BioOKF Presentation.pptx',
      });
    });

    it('decodes a Claude-style percent-escaped path from inline code', async () => {
      const onOpenArtifact = vi.fn();
      const content = 'Open `/private/tmp/BioOKF%20Presentation.pptx` in the preview.';

      render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);

      fireEvent.click(
        await screen.findByRole('button', {
          name: '/private/tmp/BioOKF%20Presentation.pptx',
        })
      );

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'BioOKF Presentation.pptx',
        path: '/private/tmp/BioOKF Presentation.pptx',
      });
    });

    it('decodes a percent-escaped path discovered in plain Markdown text', async () => {
      const onOpenArtifact = vi.fn();
      const content = 'Saved to /private/tmp/BioOKF%20Presentation.pptx';

      render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);

      fireEvent.click(
        await screen.findByRole('button', {
          name: '/private/tmp/BioOKF%20Presentation.pptx',
        })
      );

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'BioOKF Presentation.pptx',
        path: '/private/tmp/BioOKF Presentation.pptx',
      });
    });

    it('decodes a file URL once without turning double-encoded traversal into path segments', async () => {
      const onOpenArtifact = vi.fn();
      const content = '[Preview](file:///private/tmp/%252e%252e/BioOKF%2520Presentation.pptx)';

      render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);

      fireEvent.click(await screen.findByRole('button', { name: 'Preview' }));

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'BioOKF%20Presentation.pptx',
        path: '/private/tmp/%2e%2e/BioOKF%20Presentation.pptx',
      });
    });

    it('keeps file URI links inside the artifact preview instead of opening localhost', async () => {
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="[Preview report](file:///Users/wgu/project/report.pdf)"
          onOpenArtifact={onOpenArtifact}
        />
      );

      fireEvent.click(await screen.findByRole('button', { name: 'Preview report' }));

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'report.pdf',
        path: '/Users/wgu/project/report.pdf',
      });
      expect(screen.queryByRole('link', { name: 'Preview report' })).not.toBeInTheDocument();
    });

    it('renders incomplete file schemes and metadata directories as inert text', async () => {
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="Use `file://` links; the hidden `.git` directory is metadata. [Broken](file://)"
          onOpenArtifact={onOpenArtifact}
        />
      );

      expect(await screen.findByText('file://')).toHaveProperty('tagName', 'CODE');
      expect(screen.getByText('.git')).toHaveProperty('tagName', 'CODE');
      expect(screen.queryByRole('button', { name: 'file://' })).not.toBeInTheDocument();
      expect(screen.queryByRole('button', { name: '.git' })).not.toBeInTheDocument();
      expect(screen.queryByRole('link', { name: 'Broken' })).not.toBeInTheDocument();
      expect(onOpenArtifact).not.toHaveBeenCalled();
    });

    it('opens an absolute generated-file path formatted as inline code', async () => {
      const onOpenArtifact = vi.fn();
      const content =
        'Created a self-contained weather website at: `/Users/wgu/Desktop/weather-website/index.html`';

      render(<MarkdownContent content={content} onOpenArtifact={onOpenArtifact} />);

      fireEvent.click(
        await screen.findByRole('button', {
          name: '/Users/wgu/Desktop/weather-website/index.html',
        })
      );

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'index.html',
        path: '/Users/wgu/Desktop/weather-website/index.html',
      });
    });

    it('resolves a relative inline-code file path from the session working directory', async () => {
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="Open `dist/index.html` to view the site."
          workingDir="/Users/wgu/Desktop/weather-website"
          onOpenArtifact={onOpenArtifact}
        />
      );

      fireEvent.click(await screen.findByRole('button', { name: 'dist/index.html' }));

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'index.html',
        path: '/Users/wgu/Desktop/weather-website/dist/index.html',
      });
    });

    it('links a bare relative generated-file path in table content', async () => {
      const onOpenArtifact = vi.fn();
      const content = `| Output | Location |
| --- | --- |
| Website | dist/index.html |`;

      render(
        <MarkdownContent
          content={content}
          workingDir="/Users/wgu/Desktop/weather-website"
          onOpenArtifact={onOpenArtifact}
        />
      );

      fireEvent.click(await screen.findByRole('button', { name: 'dist/index.html' }));

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'index.html',
        path: '/Users/wgu/Desktop/weather-website/dist/index.html',
      });
    });

    it('opens inline-code loopback sites in the side panel', async () => {
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="Preview the site at `http://localhost:4173/dashboard`."
          onOpenArtifact={onOpenArtifact}
        />
      );

      fireEvent.click(
        await screen.findByRole('button', { name: 'http://localhost:4173/dashboard' })
      );

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'externalUrl',
        title: 'http://localhost:4173/dashboard',
        url: 'http://localhost:4173/dashboard',
      });
    });

    it('renders tables correctly', async () => {
      const content = `| Name | Value |
|------|-------|
| Test | 123   |
| Demo | 456   |`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Name')).toBeInTheDocument();
        expect(screen.getByText('Value')).toBeInTheDocument();
        expect(screen.getByText('Test')).toBeInTheDocument();
        expect(screen.getByText('123')).toBeInTheDocument();
        expect(screen.getByText('Demo')).toBeInTheDocument();
        expect(screen.getByText('456')).toBeInTheDocument();
      });
    });
  });

  describe('Error Handling', () => {
    it('handles empty content gracefully', async () => {
      render(<MarkdownContent content="" />);

      // Should not throw and should render the component
      const container = document.querySelector('.w-full.overflow-x-hidden');
      expect(container).toBeInTheDocument();
    });

    it('handles malformed markdown gracefully', async () => {
      const content = `# Unclosed header
[Unclosed link(https://example.com
\`\`\`
Unclosed code block`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        // Should still render what it can
        expect(screen.getByText('Unclosed header')).toBeInTheDocument();
      });
    });
  });

  describe('Line Break Functionality', () => {
    it('preserves single line breaks with remark-breaks plugin', async () => {
      const content = `First line
Second line
Third line`;

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        // Check that all text content is present (text may be split by <br> tags)
        expect(container).toHaveTextContent('First line');
        expect(container).toHaveTextContent('Second line');
        expect(container).toHaveTextContent('Third line');
      });

      // Check that line breaks are preserved (rendered as <br> tags)
      const brElements = container.querySelectorAll('br');
      expect(brElements.length).toBeGreaterThan(0);
    });

    it('handles mixed content with line breaks', async () => {
      const content = `# Header
Paragraph with
line breaks.

- List item 1
- List item 2`;

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByRole('heading', { level: 1, name: 'Header' })).toBeInTheDocument();

        // Check that text content is present (text may be split by <br> tags)
        expect(container).toHaveTextContent('Paragraph with');
        expect(container).toHaveTextContent('line breaks.');
        expect(screen.getByText('List item 1')).toBeInTheDocument();
        expect(screen.getByText('List item 2')).toBeInTheDocument();
      });
    });

    it('maintains existing markdown features with line breaks', async () => {
      const content = `**Bold text**
with line break

\`code\` and
more text`;

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        // Bold text should still work
        const boldElement = container.querySelector('strong');
        expect(boldElement).toBeInTheDocument();
        expect(boldElement).toHaveTextContent('Bold text');

        // Code should still work
        expect(screen.getByText('code')).toBeInTheDocument();
      });
    });
  });

  describe('URL Overflow Handling', () => {
    it('handles very long URLs without overflow', async () => {
      const longUrl =
        'https://example-docs.com/document/d/1oruk3lcrnhoOXMFzBJB8X6qQ5AtQTmj4XXxXk3xK-3g/edit?usp=sharing&mode=edit&version=1';
      const content = `Check out this document: ${longUrl}

Another very long URL: https://www.example.com/very/long/path/with/many/segments/and/parameters?param1=value1&param2=value2&param3=value3&param4=value4&param5=value5`;

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText(/Check out this document/)).toBeInTheDocument();
        expect(screen.getByText(/Another very long URL/)).toBeInTheDocument();
      });

      // Check that URLs are rendered as links
      const links = container.querySelectorAll('a');
      expect(links.length).toBeGreaterThan(0);

      // Check that links have proper CSS classes for word breaking
      links.forEach((link) => {
        // The CSS should allow the text to break
        expect(link).toBeInTheDocument();
      });
    });

    it('handles markdown links with long URLs', async () => {
      const longUrl =
        'https://example-docs.com/document/d/1oruk3lcrnhoOXMFzBJB8X6qQ5AtQTmj4XXxXk3xK-3g/edit?usp=sharing&mode=edit&version=1';
      const content = `[Click here for the document](${longUrl})`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        const link = screen.getByRole('link', { name: 'Click here for the document' });
        expect(link).toBeInTheDocument();
        expect(link).toHaveAttribute('href', longUrl);
      });
    });

    it('handles multiple long URLs in the same message', async () => {
      const content = `Here are some long URLs:

1. Example Doc: https://example-docs.com/document/d/1oruk3lcrnhoOXMFzBJB8X6qQ5AtQTmj4XXxXk3xK-3g/edit?usp=sharing&mode=edit&version=1
2. Another long URL: https://www.example.com/very/long/path/with/many/segments/and/parameters?param1=value1&param2=value2&param3=value3
3. Third URL: https://api.example.com/v1/users/12345/documents/67890/attachments/abcdef123456789?format=json&include=metadata&sort=created_at`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText(/Here are some long URLs/)).toBeInTheDocument();
        expect(screen.getByText(/Example Doc/)).toBeInTheDocument();
        expect(screen.getByText(/Another long URL/)).toBeInTheDocument();
        expect(screen.getByText(/Third URL/)).toBeInTheDocument();
      });
    });

    it('applies word-break CSS classes to the container', () => {
      const content = 'Test content';
      render(<MarkdownContent content={content} />);

      const markdownContainer = document.querySelector('.prose');
      expect(markdownContainer).toBeInTheDocument();
      expect(markdownContainer).toHaveClass('prose-a:break-all');
      // `prose-a:overflow-wrap-anywhere` used to be asserted here. It is not a
      // Tailwind v4 utility and generated no CSS at all, so the assertion only
      // ever proved the string was in the class attribute.
      expect(markdownContainer?.className).not.toContain('overflow-wrap-anywhere');
    });
  });

  describe('Shared markdown treatment', () => {
    const proseContainer = () => document.querySelector('.prose');

    it.each(['chat', 'document'] as const)(
      'preserves heading hierarchy and nested lists in %s',
      (variant) => {
        render(
          <MarkdownContent
            variant={variant}
            content={
              '# Overview\n\n## Findings\n\n### Detail\n\n#### Notes\n\n- Parent\n  - Child 中文 α'
            }
          />
        );
        for (const [index, name] of ['Overview', 'Findings', 'Detail', 'Notes'].entries()) {
          expect(screen.getByRole('heading', { level: index + 1, name })).toBeInTheDocument();
        }
        expect(screen.getByText('Child 中文 α').closest('ul')?.parentElement?.tagName).toBe('LI');
        expect(proseContainer()).toHaveClass('biorouter-markdown');
      }
    );

    it.each(['chat', 'document'] as const)(
      'makes tables keyboard accessible and preserves alignment and complete values in %s',
      (variant) => {
        render(
          <MarkdownContent
            variant={variant}
            content={
              '| Measure | Value | Status |\n| :--- | ---: | :---: |\n| α coefficient | 5.809e-04 | Ready |\n| Long_identifier_with_no_breaks | 1234567890.012345 | ✓ |'
            }
          />
        );
        const region = screen.getByRole('region', { name: 'Scrollable table' });
        expect(region).toHaveAttribute('tabindex', '0');
        expect(region.querySelector('table')).toBe(screen.getByRole('table'));
        expect(screen.getByRole('cell', { name: '5.809e-04' })).toHaveStyle({ textAlign: 'right' });
        expect(screen.getByRole('cell', { name: 'Ready' })).toHaveStyle({ textAlign: 'center' });
        expect(screen.getByRole('cell', { name: '1234567890.012345' })).toBeInTheDocument();
      }
    );

    it.each(['chat', 'document'] as const)(
      'copies unlabeled fenced text and keeps it inert in %s',
      async (variant) => {
        const copy = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, 'clipboard', {
          configurable: true,
          value: { writeText: copy },
        });
        const onOpenArtifact = vi.fn();
        const onRunInTerminal = vi.fn();
        const { container } = render(
          <MarkdownContent
            variant={variant}
            content={'```\n/tmp/results.csv\n  α = 0.0005809\n```\n\nInline `value` stays inline.'}
            onOpenArtifact={onOpenArtifact}
            onRunInTerminal={onRunInTerminal}
          />
        );
        expect(container.querySelector('.biorouter-md-code-lang')).toHaveTextContent('text');
        fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
        await waitFor(() => expect(copy).toHaveBeenCalledWith('/tmp/results.csv\n  α = 0.0005809'));
        expect(screen.queryByRole('button', { name: 'Run' })).not.toBeInTheDocument();
        expect(container.querySelector('.biorouter-md-code button[title*="Preview"]')).toBeNull();
        expect(screen.getByText('value').tagName).toBe('CODE');
        expect(onRunInTerminal).not.toHaveBeenCalled();
        expect(onOpenArtifact).not.toHaveBeenCalled();
      }
    );

    it('suppresses the curly quotes the typography plugin injects into blockquotes', async () => {
      render(<MarkdownContent content="> Confidence is an edge attribute." />);

      await waitFor(() => {
        expect(screen.getByText('Confidence is an edge attribute.')).toBeInTheDocument();
      });

      const container = proseContainer();
      // The plugin emits `content: open-quote` on `blockquote p:first-of-type`
      // and `close-quote` on `p:last-of-type` (NOT first-of-type). Both need
      // suppressing or stray “ ” marks render around every quote.
      expect(container).toHaveClass('[&_blockquote_p:first-of-type]:before:content-none');
      expect(container).toHaveClass('[&_blockquote_p:last-of-type]:after:content-none');
    });

    it('leaves table presentation to the shared stylesheet', async () => {
      const content = `| Compound | Edges |
| --- | ---: |
| Fingolimod | 318 |`;

      render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(screen.getByText('Fingolimod')).toBeInTheDocument();
      });

      const className = proseContainer()?.className ?? '';
      // The shared stylesheet owns the header band and horizontal separators.
      expect(className).not.toContain('prose-td:border');
      expect(className).not.toContain('prose-th:border');
      expect(className).not.toContain('prose-thead:bg-background-medium');
      // prose-sm sets the table to 12px (the Caption role); §3.2 puts table
      // text at the 13px Secondary/metadata step.
      expect(proseContainer()).toHaveClass('prose-table:text-[13px]');
      // Digits line up whether or not the model authored a `---:` column.
      expect(proseContainer()).toHaveClass('prose-td:tabular-nums');
      expect(proseContainer()).toHaveClass('prose-th:tabular-nums');
    });

    it('gives links a single accent-token treatment', async () => {
      render(<MarkdownContent content="[edge documentation](https://example.com/docs)" />);

      await waitFor(() => {
        expect(screen.getByRole('link', { name: 'edge documentation' })).toBeInTheDocument();
      });

      const container = proseContainer();
      // Ink comes from --tw-prose-links (main.css points it at --text-accent);
      // the underline is the accent at 40%.
      expect(container).toHaveClass('prose-a:decoration-text-accent/40');
      expect(container).toHaveClass('prose-a:underline-offset-2');
      expect(container).toHaveClass('prose-a:font-medium');
      // The neutral `decoration-border-strong` treatment is retired.
      expect(container?.className).not.toContain('decoration-border-strong');
    });

    it('gives artifact links the same accent treatment as plain links', async () => {
      const onOpenArtifact = vi.fn();
      render(
        <MarkdownContent
          content="See `/Users/wgu/project/analysis.sql` for the query."
          onOpenArtifact={onOpenArtifact}
        />
      );

      const button = await screen.findByRole('button', { name: '/Users/wgu/project/analysis.sql' });
      // A <button> is not an <a>, so `prose-a:*` cannot reach it — it must
      // restate the one treatment rather than invent a third.
      expect(button).toHaveClass('text-text-accent');
      expect(button).toHaveClass('decoration-text-accent/40');
      expect(button).toHaveClass('underline-offset-2');
      expect(button).not.toHaveClass('decoration-border-strong');
    });

    it('uses one inline-code size across the code element and the artifact button', async () => {
      const onOpenArtifact = vi.fn();
      render(
        <MarkdownContent
          content="Open `dist/index.html` now."
          workingDir="/Users/wgu/Desktop/weather-website"
          onOpenArtifact={onOpenArtifact}
        />
      );

      const button = await screen.findByRole('button', { name: 'dist/index.html' });
      // The two ArtifactLinkButton variants used to disagree (0.9em vs 0.95em)
      // for the same widget; inline code was 0.9em (12.6px) against the fenced
      // block's 13px. All three are now the 13px "Code / terminal" step.
      expect(button).toHaveClass('text-[13px]');
      expect(proseContainer()).toHaveClass('prose-code:text-[13px]');
    });

    it('leaves inline code a single fill and drops the competing bg-inline-code class', async () => {
      render(<MarkdownContent content="Use `console.log()` to debug." />);

      const code = await screen.findByText('console.log()');
      expect(code.tagName).toBe('CODE');
      // Two fills (`bg-inline-code` on the element, `prose-code:bg-*` on the
      // wrapper) were reconciled only by a specificity ladder in main.css.
      expect(code).not.toHaveClass('bg-inline-code');
      expect(proseContainer()).toHaveClass('prose-code:bg-background-medium');
    });
  });

  describe('KaTeX Math Rendering - singleDollarTextMath: false', () => {
    it('treats single dollar signs as plain text', async () => {
      const content = 'The formula $x_i$ represents the i-th element.';

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        const katexElements = container.querySelectorAll('.katex');
        expect(katexElements.length).toBe(0);
        expect(container).toHaveTextContent('$x_i$');
      });
    });

    it('renders double dollar signs as display math', async () => {
      const content = `Calculate

$$
x^2 + y^2
$$

for the result.`;

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        const katexDisplay = container.querySelector('.katex-display');
        expect(katexDisplay).toBeInTheDocument();
      });
    });

    it('leaves display-math centring, margin and overflow to the .katex-display CSS', async () => {
      const content = `Calculate

$$
x^2 + y^2
$$

for the result.`;

      const { container } = render(<MarkdownContent content={content} />);

      const display = await waitFor(() => {
        const el = container.querySelector('.katex-display');
        expect(el).toBeInTheDocument();
        return el!;
      });

      // remark-math emits display math as a BLOCK sibling, so KaTeX's
      // `.katex-display` is a direct child of the prose root and never sits
      // inside a paragraph at all.
      expect(display.closest('p')).toBeNull();

      // main.css's `.katex-display` is the single source of truth for centring,
      // margin and overflow. MarkdownParagraph used to restate all three on a
      // `flex justify-center my-3 overflow-x-auto` wrapper; nothing in the
      // rendered tree may carry them any more.
      for (const dup of ['justify-center', 'my-3', 'overflow-x-auto']) {
        expect(container.querySelector(`[class*="${dup}"]`)).toBeNull();
      }
    });

    it('handles shell commands without triggering math mode', async () => {
      const content = 'Run echo "$FOO_BAR" to see the value.';

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        const katexElements = container.querySelectorAll('.katex');
        expect(katexElements.length).toBe(0);
        expect(container).toHaveTextContent('$FOO_BAR');
      });
    });

    it('preserves math in code blocks', async () => {
      const content = 'The formula `math\nx^2\n` uses inline code.';

      const { container } = render(<MarkdownContent content={content} />);

      await waitFor(() => {
        expect(container).toHaveTextContent('x^2');
      });
    });
  });

  describe('Image rendering in the preview', () => {
    afterEach(() => {
      // @ts-expect-error — remove the per-test electron stub.
      delete window.electron;
      vi.restoreAllMocks();
    });

    it('inlines a relative local image via the allowlisted read IPC as a data URI', async () => {
      const electron = installElectronMock();
      const content = '![Figure one](./fig.png)';

      render(<MarkdownContent content={content} workingDir="/home/ada/project/docs" />);

      await waitFor(() => {
        expect(electron.readArtifactFile).toHaveBeenCalledWith('/home/ada/project/docs/fig.png');
      });

      const img = await waitFor(() => {
        const el = document.querySelector('img');
        expect(el).not.toBeNull();
        return el as HTMLImageElement;
      });
      expect(img.getAttribute('src')).toBe('data:image/png;base64,AAAA');
      expect(img).toHaveAttribute('alt', 'Figure one');
    });

    it('renders a remote https image directly without touching the read IPC', async () => {
      const electron = installElectronMock();
      const content = '![Remote](https://example.com/plot.png)';

      render(<MarkdownContent content={content} workingDir="/home/ada/project/docs" />);

      const img = await waitFor(() => {
        const el = document.querySelector('img');
        expect(el).not.toBeNull();
        return el as HTMLImageElement;
      });
      expect(img.getAttribute('src')).toBe('https://example.com/plot.png');
      expect(electron.readArtifactFile).not.toHaveBeenCalled();
    });

    it('shows a broken-image placeholder for a traversal path and never reads it', async () => {
      const electron = installElectronMock();
      const content = '![Secret](../../secret.png)';

      render(<MarkdownContent content={content} workingDir="/home/ada/project/docs" />);

      const placeholder = await screen.findByRole('img', { name: 'Secret' });
      expect(placeholder.tagName).toBe('SPAN');
      expect(document.querySelector('img')).toBeNull();
      expect(electron.readArtifactFile).not.toHaveBeenCalled();
    });

    it('falls back to a placeholder when the allowlist denies the local image', async () => {
      const electron = installElectronMock({
        readArtifactFile: vi.fn(async (path: string) => ({
          kind: 'error',
          title: 'fig.png',
          path,
          error: `Access denied: path '${path}' is outside allowed directories`,
          found: false,
        })),
      });
      const content = '![Denied](/etc/shadow.png)';

      render(<MarkdownContent content={content} workingDir="/home/ada/project/docs" />);

      await waitFor(() => expect(electron.readArtifactFile).toHaveBeenCalled());
      expect(await screen.findByRole('img', { name: 'Denied' })).toHaveProperty('tagName', 'SPAN');
      expect(document.querySelector('img')).toBeNull();
    });
  });

  describe('External link handler', () => {
    afterEach(() => {
      // @ts-expect-error — remove the per-test electron stub.
      delete window.electron;
      vi.restoreAllMocks();
    });

    it('opens an external link in the system browser and does not navigate', async () => {
      const electron = installElectronMock();
      const content = '[Docs](https://example.com/docs)';

      render(<MarkdownContent content={content} />);

      const link = await screen.findByRole('link', { name: 'Docs' });
      // fireEvent.click returns false when the handler called preventDefault, i.e.
      // the renderer will NOT navigate to the href — the browser opens it instead.
      const notPrevented = fireEvent.click(link);

      expect(electron.openExternal).toHaveBeenCalledWith('https://example.com/docs');
      expect(notPrevented).toBe(false);
    });

    it('opens a public HTTP link in the artifact panel when one is available', async () => {
      const electron = installElectronMock();
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="[Docs](https://example.com/docs)"
          onOpenArtifact={onOpenArtifact}
        />
      );

      fireEvent.click(await screen.findByRole('button', { name: 'Docs' }));

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'externalUrl',
        title: 'https://example.com/docs',
        url: 'https://example.com/docs',
      });
      expect(electron.openExternal).not.toHaveBeenCalled();
    });

    it('keeps loopback HTTP links on the artifact-panel route', async () => {
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="[Local app](http://127.0.0.1:4173/dashboard)"
          onOpenArtifact={onOpenArtifact}
        />
      );

      fireEvent.click(await screen.findByRole('button', { name: 'Local app' }));

      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'externalUrl',
        title: 'http://127.0.0.1:4173/dashboard',
        url: 'http://127.0.0.1:4173/dashboard',
      });
    });

    it('does not route unsafe URL schemes into the artifact panel', async () => {
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="[Run script](javascript:alert%281%29)"
          onOpenArtifact={onOpenArtifact}
        />
      );

      expect(await screen.findByText('Run script')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Run script' })).not.toBeInTheDocument();
      expect(screen.queryByRole('link', { name: 'Run script' })).not.toBeInTheDocument();
      expect(onOpenArtifact).not.toHaveBeenCalled();
    });
  });

  // A path the assistant merely NAMED — a script it described, a `/tmp` tree
  // since cleaned up — used to render as an accent-coloured link that did
  // nothing when clicked. `check-file-paths` separates the two cases; these
  // tests pin both halves of the resulting contract, plus the surfaces that
  // have no bridge to ask.
  describe('Existence-aware file links', () => {
    function installCheckBridge(
      answer: (request: FilePathCheckRequest) => FilePathCheckResult = () => ({
        exists: true,
        isDirectory: false,
      })
    ) {
      const checkFilePaths = vi.fn(async (requests: FilePathCheckRequest[]) =>
        requests.map(answer)
      );
      Object.defineProperty(window, 'electron', {
        configurable: true,
        value: { checkFilePaths },
      });
      return checkFilePaths;
    }

    afterEach(() => {
      // @ts-expect-error — remove the per-test electron stub.
      delete window.electron;
      resetFileLinkStatusForTests();
      vi.restoreAllMocks();
    });

    it('keeps unavailable header file links as plain text and preserves inline-code chips', async () => {
      installCheckBridge(() => ({ exists: false, isDirectory: false }));
      const { container } = render(
        <MarkdownContent
          content={'| /tmp/missing.csv | `/tmp/missing-code.csv` |\n| --- | --- |\n| one | two |'}
          onOpenArtifact={vi.fn()}
        />
      );
      await waitFor(() => expect(screen.getByText('/tmp/missing.csv').tagName).toBe('SPAN'));
      const text = screen.getByText('/tmp/missing.csv');
      expect(text.tagName).toBe('SPAN');
      expect(text.closest('th')).not.toBeNull();
      expect(text).not.toHaveClass('biorouter-inline-code');
      expect(screen.getByText('/tmp/missing-code.csv')).toHaveClass('biorouter-inline-code');
      expect(container.querySelector('thead button')).toBeNull();
    });

    it('keeps the accent link treatment for a file confirmed to exist', async () => {
      installCheckBridge();
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="See `/Users/wgu/project/analysis.sql` for the query."
          onOpenArtifact={onOpenArtifact}
        />
      );

      const button = await screen.findByRole('button', { name: '/Users/wgu/project/analysis.sql' });
      expect(button).toHaveClass('text-text-accent');
      expect(button).toHaveClass('underline');
      expect(button).toHaveClass('cursor-pointer');

      fireEvent.click(button);
      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'analysis.sql',
        path: '/Users/wgu/project/analysis.sql',
      });
    });

    it('decolors a file the main process cannot find and makes it inert', async () => {
      installCheckBridge(() => ({ exists: false, isDirectory: false }));
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="I would put it in /Users/wgu/project/imagined.py next."
          onOpenArtifact={onOpenArtifact}
        />
      );

      const mention = await screen.findByText('/Users/wgu/project/imagined.py');
      await waitFor(() => expect(mention.tagName).toBe('SPAN'));
      // Decolored, not muted: the ask was that it read as ordinary prose.
      expect(mention).toHaveClass('text-text-default');
      expect(mention).not.toHaveClass('text-text-accent');
      expect(mention).not.toHaveClass('underline');
      expect(mention).not.toHaveClass('cursor-pointer');
      // Not a button, not focusable, and nothing happens when it is clicked.
      expect(
        screen.queryByRole('button', { name: '/Users/wgu/project/imagined.py' })
      ).not.toBeInTheDocument();
      expect(mention).not.toHaveAttribute('tabindex');
      fireEvent.click(mention);
      expect(onOpenArtifact).not.toHaveBeenCalled();
    });

    it('keeps a missing inline-code path looking like inline code, minus the link', async () => {
      installCheckBridge(() => ({ exists: false, isDirectory: false }));
      const onOpenArtifact = vi.fn();

      render(
        <MarkdownContent
          content="Write it to `/Users/wgu/project/imagined.py`."
          onOpenArtifact={onOpenArtifact}
        />
      );

      const mention = await screen.findByText('/Users/wgu/project/imagined.py');
      await waitFor(() => expect(mention.tagName).toBe('SPAN'));
      // The TEXT is unchanged — same family, size and fill as inline code.
      expect(mention).toHaveClass('font-mono');
      expect(mention).toHaveClass('text-[13px]');
      expect(mention).toHaveClass('bg-background-medium');
      expect(mention).not.toHaveClass('text-text-accent');
    });

    it('never renders a link before the answer arrives, not even for one frame', async () => {
      let release: ((results: FilePathCheckResult[]) => void) | undefined;
      const checkFilePaths = vi.fn(
        () =>
          new Promise<FilePathCheckResult[]>((resolve) => {
            release = resolve;
          })
      );
      Object.defineProperty(window, 'electron', {
        configurable: true,
        value: { checkFilePaths },
      });

      render(
        <MarkdownContent
          content="See `/Users/wgu/project/analysis.sql` for the query."
          onOpenArtifact={vi.fn()}
        />
      );

      const mention = await screen.findByText('/Users/wgu/project/analysis.sql');
      expect(mention.tagName).toBe('SPAN');
      expect(screen.queryByRole('button')).not.toBeInTheDocument();

      release?.([{ exists: true, isDirectory: false }]);
      await waitFor(() =>
        expect(
          screen.getByRole('button', { name: '/Users/wgu/project/analysis.sql' })
        ).toBeInTheDocument()
      );
    });

    it('keeps the legacy behaviour on a surface with no bridge to ask', async () => {
      const onOpenArtifact = vi.fn();

      // No `window.electron` at all — `biorouter serve` in a browser, and every
      // suite above this one. "Start plain" would mean nothing is EVER a link
      // there, so the absence of the bridge keeps the pre-existing contract.
      render(
        <MarkdownContent
          content="See `/Users/wgu/project/imagined.py` for the query."
          onOpenArtifact={onOpenArtifact}
        />
      );

      const button = await screen.findByRole('button', { name: '/Users/wgu/project/imagined.py' });
      expect(button).toHaveClass('text-text-accent');
      fireEvent.click(button);
      expect(onOpenArtifact).toHaveBeenCalled();
    });

    it('asks once for a path a single message mentions three times', async () => {
      const checkFilePaths = installCheckBridge();

      render(
        <MarkdownContent
          content={[
            'First `/Users/wgu/project/analysis.sql`.',
            'Then /Users/wgu/project/analysis.sql again.',
            '- [Third](/Users/wgu/project/analysis.sql)',
          ].join('\n\n')}
          onOpenArtifact={vi.fn()}
        />
      );

      await waitFor(() =>
        expect(screen.getAllByRole('button', { name: /analysis\.sql|Third/ })).toHaveLength(3)
      );
      expect(checkFilePaths).toHaveBeenCalledTimes(1);
      expect(checkFilePaths.mock.calls[0][0]).toEqual([
        { path: '/Users/wgu/project/analysis.sql' },
      ]);
    });

    it.each([
      ['a Windows drive-letter path', 'C:\\Users\\x\\a.py'],
      ['a home-relative path', '~/a.py'],
    ])('hands %s to the main process verbatim, unmangled', async (_label, target) => {
      const checkFilePaths = installCheckBridge();

      render(
        <MarkdownContent
          content={`Open \`${target}\` now.`}
          workingDir="/Users/wgu/project"
          onOpenArtifact={vi.fn()}
        />
      );

      // Resolution of `~` and of a drive letter belongs to the main process,
      // which knows the host OS; the renderer must not join either onto the
      // session working directory on its way there.
      await waitFor(() => expect(checkFilePaths).toHaveBeenCalledTimes(1));
      expect(checkFilePaths.mock.calls[0][0]).toEqual([
        { path: target, workingDir: '/Users/wgu/project' },
      ]);
      expect(await screen.findByRole('button', { name: target })).toBeInTheDocument();
    });
  });
});
