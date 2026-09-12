import { describe, expect, it } from 'vitest';
import {
  artifactSourceFromResource,
  artifactSourceFromResourceLink,
  baseToolName,
  basenameFromPath,
  decodeResourceHtml,
  dirnameFromPath,
  extensionFromPath,
  fileArtifactPathsFromToolCall,
  languageFromPath,
  languageLabel,
  looksLikePreviewableFile,
  parseDelimitedTable,
  pathFromArtifactHref,
  resolveArtifactPath,
  resolveMarkdownImageSource,
  sandboxedSurface,
  withHostTheme,
} from './artifactUtils';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from '../../styles/themes.generated';
import type { ThemeFamily } from '../../contexts/ThemeContext';

const WORKING_DIR = '/home/ada/project';

describe('basenameFromPath', () => {
  it('returns the final path segment', () => {
    expect(basenameFromPath('/work/report.md')).toBe('report.md');
    expect(basenameFromPath('C:\\data\\plot.png')).toBe('plot.png');
    expect(basenameFromPath('results.csv')).toBe('results.csv');
  });

  it('decodes valid percent-encoding in a resource URI basename', () => {
    expect(basenameFromPath('ui://reports/my%20report.html')).toBe('my report.html');
  });

  it('does NOT throw on a literal percent sign in a real filename', () => {
    // A file named with a stray `%` (common: "results 100%.csv") is not valid
    // percent-encoding, so decodeURIComponent throws URIError: "URI malformed".
    // basenameFromPath runs during chat render (collectArtifactsFromMessages),
    // so an unguarded throw crashes the whole app into the "Honk!" boundary.
    expect(() => basenameFromPath('/work/results 100%.csv')).not.toThrow();
    expect(basenameFromPath('/work/results 100%.csv')).toBe('results 100%.csv');
    expect(basenameFromPath('/tmp/50%off/report%.html')).toBe('report%.html');
  });

  it('extensionFromPath survives a literal percent sign in the name', () => {
    expect(() => extensionFromPath('/work/results 100%.csv')).not.toThrow();
    expect(extensionFromPath('/work/results 100%.csv')).toBe('csv');
  });
});

describe('MCP artifact resources', () => {
  it('accepts explicit HTML MIME types with parameters', () => {
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: 'ui://report/safe',
            mimeType: 'text/html; charset=utf-8',
            text: '<!doctype html><title>Safe</title>',
          },
        },
        'Artifact'
      )
    ).toMatchObject({
      kind: 'html',
      html: '<!doctype html><title>Safe</title>',
    });
  });

  it('does not execute non-HTML blob or text resources as HTML', () => {
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: 'ui://report/plain',
            mimeType: 'text/plain',
            blob: btoa('<script>window.bad = true</script>'),
          },
        },
        'Artifact'
      )
    ).toMatchObject({ kind: 'mcpResource' });
  });

  it('rejects malformed encoded HTML', () => {
    expect(decodeResourceHtml({ blob: 'not-base64' })).toBeNull();
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: 'ui://report/broken',
            mimeType: 'text/html',
            blob: 'not-base64',
          },
        },
        'Artifact'
      )
    ).toBeNull();
  });

  it('rejects oversized resource URIs before deriving a preview title', () => {
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: `ui://report/${'x'.repeat(8 * 1024)}`,
            mimeType: 'text/html',
            text: '<!doctype html>',
          },
        },
        'Artifact'
      )
    ).toBeNull();
  });

  it('turns web resource links into click-only previews and blocks unsafe schemes', () => {
    expect(
      artifactSourceFromResourceLink({
        uri: 'https://example.test/report.html',
        name: 'report',
        title: 'Study report',
      })
    ).toEqual({
      kind: 'externalUrl',
      title: 'Study report',
      url: 'https://example.test/report.html',
    });
    expect(
      artifactSourceFromResourceLink({ uri: 'javascript:alert(1)', name: 'unsafe' })
    ).toBeNull();
    expect(
      artifactSourceFromResourceLink({
        uri: 'https://user:secret@example.test/report',
        name: 'credentials',
      })
    ).toBeNull();
    expect(
      artifactSourceFromResourceLink({
        uri: `https://example.test/${'x'.repeat(8 * 1024)}`,
        name: 'oversized',
      })
    ).toBeNull();

    expect(
      artifactSourceFromResourceLink({
        uri: 'https://example.test/safe',
        name: 'report',
        title: `Safe\u001b]8;;https://evil.test\u0007spoof\u202e${'x'.repeat(300)}`,
      })?.title
    ).toBe(`Safe]8;;https://evil.testspoof${'x'.repeat(226)}`);
  });

  it('keeps externally hosted HTML resources click-only', () => {
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: 'https://example.test/generated-report.html',
            mimeType: 'text/html',
            text: '<script>window.location = "https://unexpected.test"</script>',
          },
        },
        'Generated report'
      )
    ).toMatchObject({
      kind: 'externalUrl',
      url: 'https://example.test/generated-report.html',
    });
  });

  it('sanitizes a control-only fallback title', () => {
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: '',
            mimeType: 'text/plain',
            text: 'not an HTML preview',
          },
        },
        '\u001b\u202e'
      )?.title
    ).toBe('Artifact');
  });

  it('turns URI-list resources into normalized click-only previews', () => {
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: 'ui://report/published',
            mimeType: 'text/uri-list; charset=utf-8',
            text: '# published report\nhttps://example.test/report with spaces.html\n',
          },
        },
        'Published report'
      )
    ).toMatchObject({
      kind: 'externalUrl',
      url: 'https://example.test/report%20with%20spaces.html',
    });
    expect(
      artifactSourceFromResource(
        {
          type: 'resource',
          resource: {
            uri: 'ui://report/unsafe',
            mimeType: 'text/uri-list',
            text: 'file:///etc/passwd',
          },
        },
        'Unsafe report'
      )
    ).toBeNull();
  });
});

describe('previewable artifact paths', () => {
  it('accepts concrete local files and folders', () => {
    expect(looksLikePreviewableFile('/Users/ada/project/report.pdf')).toBe(true);
    expect(looksLikePreviewableFile('file:///Users/ada/project/My%20Folder')).toBe(true);
    expect(looksLikePreviewableFile('./results/data.csv')).toBe(true);
    expect(looksLikePreviewableFile('report.docx')).toBe(true);
    expect(looksLikePreviewableFile('C:\\Users\\ada\\project')).toBe(true);
    expect(looksLikePreviewableFile('\\\\server\\share\\report.pdf')).toBe(true);
    expect(pathFromArtifactHref('file:///Users/ada/project/My%20Report.pdf')).toBe(
      '/Users/ada/project/My Report.pdf'
    );
  });

  it.each(['file://', 'file:///', '/', '~/', './', '../', '.git', '/work/.git'])(
    'rejects non-artifact path token %s',
    (value) => {
      expect(looksLikePreviewableFile(value)).toBe(false);
    }
  );

  it('rejects web links and unsupported bare filenames', () => {
    expect(looksLikePreviewableFile('https://localhost:5174/report.pdf')).toBe(false);
    expect(looksLikePreviewableFile('archive.bin')).toBe(false);
  });
});

describe('languageFromPath', () => {
  it('maps the scripts an agent actually writes to a Prism language', () => {
    expect(languageFromPath('/w/analysis.R')).toBe('r');
    expect(languageFromPath('/w/run.py')).toBe('python');
    expect(languageFromPath('/w/lib.rs')).toBe('rust');
    expect(languageFromPath('/w/setup.sh')).toBe('bash');
    expect(languageFromPath('/w/query.sql')).toBe('sql');
    expect(languageFromPath('/w/main.go')).toBe('go');
    expect(languageFromPath('/w/app.tsx')).toBe('tsx');
    expect(languageFromPath('/w/conf.yml')).toBe('yaml');
  });

  it('treats R Markdown and Quarto as markdown', () => {
    expect(languageFromPath('/w/report.Rmd')).toBe('markdown');
    expect(languageFromPath('/w/report.qmd')).toBe('markdown');
    expect(languageFromPath('/w/report.md')).toBe('markdown');
  });

  it('falls back to the mime type, then plain text', () => {
    expect(languageFromPath('/w/noext', 'application/json')).toBe('json');
    expect(languageFromPath('/w/noext')).toBe('text');
  });
});

describe('languageLabel', () => {
  it('names languages the way people write them', () => {
    expect(languageLabel('/w/analysis.R')).toBe('R');
    expect(languageLabel('/w/report.Rmd')).toBe('R Markdown');
    expect(languageLabel('/w/a.cpp')).toBe('C++');
    expect(languageLabel('/w/q.sql')).toBe('SQL');
    expect(languageLabel('/w/x.py')).toBe('Python');
    expect(languageLabel('/w/notes.txt')).toBe('Text');
  });

  it('keeps acronyms uppercase rather than title-casing them', () => {
    expect(languageLabel('/w/genes.csv')).toBe('CSV');
    expect(languageLabel('/w/genes.tsv')).toBe('TSV');
    expect(languageLabel('/w/a.json')).toBe('JSON');
    expect(languageLabel('/w/a.yaml')).toBe('YAML');
    expect(languageLabel('/w/a.yml')).toBe('YAML');
    expect(languageLabel('/w/a.xml')).toBe('XML');
    expect(languageLabel('/w/a.toml')).toBe('TOML');
    expect(languageLabel('/w/a.css')).toBe('CSS');
    expect(languageLabel('/w/a.html')).toBe('HTML');
  });

  it('spells camel-cased language names correctly', () => {
    expect(languageLabel('/w/app.ts')).toBe('TypeScript');
    expect(languageLabel('/w/app.tsx')).toBe('TSX');
    expect(languageLabel('/w/app.js')).toBe('JavaScript');
    expect(languageLabel('/w/app.jsx')).toBe('JSX');
    expect(languageLabel('/w/run.sh')).toBe('Shell');
    expect(languageLabel('/w/notes.md')).toBe('Markdown');
    expect(languageLabel('/w/lib.rs')).toBe('Rust');
    expect(languageLabel('/w/main.go')).toBe('Go');
  });
});

describe('parseDelimitedTable', () => {
  it('splits a simple CSV', () => {
    expect(parseDelimitedTable('a,b\n1,2\n', ',')).toEqual([
      ['a', 'b'],
      ['1', '2'],
    ]);
  });

  it('keeps a delimiter inside a quoted field', () => {
    expect(parseDelimitedTable('gene,note\n"TP53, alias",x\n', ',')).toEqual([
      ['gene', 'note'],
      ['TP53, alias', 'x'],
    ]);
  });

  it('unescapes doubled quotes', () => {
    expect(parseDelimitedTable('a\n"say ""hi"""\n', ',')).toEqual([['a'], ['say "hi"']]);
  });

  it('handles CRLF line endings and TSV', () => {
    expect(parseDelimitedTable('a\tb\r\n1\t2\r\n', '\t')).toEqual([
      ['a', 'b'],
      ['1', '2'],
    ]);
  });

  it('drops trailing blank lines', () => {
    expect(parseDelimitedTable('a,b\n1,2\n\n', ',')).toEqual([
      ['a', 'b'],
      ['1', '2'],
    ]);
  });
});

describe('baseToolName', () => {
  it('strips the extension prefix', () => {
    expect(baseToolName('developer__text_editor')).toBe('text_editor');
    expect(baseToolName('text_editor')).toBe('text_editor');
    expect(baseToolName('a__b__shell')).toBe('shell');
  });
});

describe('resolveArtifactPath', () => {
  it('keeps absolute and home-relative paths', () => {
    expect(resolveArtifactPath('/tmp/report.md')).toBe('/tmp/report.md');
    expect(resolveArtifactPath('~/notes.md')).toBe('~/notes.md');
  });

  it('anchors relative paths to the working directory', () => {
    expect(resolveArtifactPath('results/plot.png', WORKING_DIR)).toBe(
      '/home/ada/project/results/plot.png'
    );
    expect(resolveArtifactPath('./out.csv', WORKING_DIR)).toBe('/home/ada/project/out.csv');
  });

  it('refuses relative paths with nothing to anchor them to', () => {
    expect(resolveArtifactPath('results/plot.png')).toBeNull();
  });

  it('refuses paths that escape the working directory', () => {
    expect(resolveArtifactPath('../secrets.md', WORKING_DIR)).toBeNull();
    expect(resolveArtifactPath('..\\secrets.md', WORKING_DIR)).toBeNull();
    expect(resolveArtifactPath('..', WORKING_DIR)).toBeNull();
  });

  it('keeps Windows absolute paths intact instead of gluing them to the working dir', () => {
    expect(resolveArtifactPath('C:\\Users\\me\\report.md', 'C:\\Users\\me\\proj')).toBe(
      'C:\\Users\\me\\report.md'
    );
    expect(resolveArtifactPath('C:/Users/me/report.md', 'C:\\proj')).toBe('C:/Users/me/report.md');
    expect(resolveArtifactPath('\\\\server\\share\\a.md', 'C:\\proj')).toBe(
      '\\\\server\\share\\a.md'
    );
  });

  it('joins relative paths with the working directory separator', () => {
    expect(resolveArtifactPath('docs\\out.md', 'C:\\Users\\me\\proj')).toBe(
      'C:\\Users\\me\\proj\\docs\\out.md'
    );
    expect(resolveArtifactPath('.\\out.md', 'C:\\proj')).toBe('C:\\proj\\out.md');
  });

  it('unwraps file:// urls and surrounding quotes', () => {
    expect(resolveArtifactPath('file:///tmp/a%20b.md')).toBe('/tmp/a b.md');
    expect(resolveArtifactPath('"/tmp/quoted.md"')).toBe('/tmp/quoted.md');
  });
});

describe('dirnameFromPath', () => {
  it('returns the directory of a file, without a trailing separator', () => {
    expect(dirnameFromPath('/home/ada/project/report.md')).toBe('/home/ada/project');
    expect(dirnameFromPath('/home/ada/project/sub/fig.png')).toBe('/home/ada/project/sub');
  });

  it('handles Windows separators', () => {
    expect(dirnameFromPath('C:\\Users\\me\\proj\\report.md')).toBe('C:\\Users\\me\\proj');
  });

  it('is empty for a bare filename (nothing to anchor against)', () => {
    expect(dirnameFromPath('report.md')).toBe('');
  });

  it('unwraps file:// urls before splitting', () => {
    expect(dirnameFromPath('file:///tmp/docs/report.md')).toBe('/tmp/docs');
  });
});

describe('resolveMarkdownImageSource', () => {
  const FILE_DIR = '/home/ada/project/docs';

  it('resolves a relative image against the previewed file directory', () => {
    expect(resolveMarkdownImageSource('./fig.png', FILE_DIR)).toEqual({
      kind: 'local',
      path: '/home/ada/project/docs/fig.png',
    });
    expect(resolveMarkdownImageSource('img/plot.png', FILE_DIR)).toEqual({
      kind: 'local',
      path: '/home/ada/project/docs/img/plot.png',
    });
  });

  it('keeps an absolute local image path as a local read', () => {
    expect(resolveMarkdownImageSource('/tmp/out/fig.png', FILE_DIR)).toEqual({
      kind: 'local',
      path: '/tmp/out/fig.png',
    });
    expect(resolveMarkdownImageSource('file:///tmp/out/fig%20a.png')).toEqual({
      kind: 'local',
      path: '/tmp/out/fig a.png',
    });
  });

  it('blocks a relative image that traverses out of the file directory', () => {
    expect(resolveMarkdownImageSource('../../secret.png', FILE_DIR)).toEqual({ kind: 'blocked' });
    expect(resolveMarkdownImageSource('fig.png')).toEqual({ kind: 'blocked' });
    expect(resolveMarkdownImageSource('   ')).toEqual({ kind: 'blocked' });
  });

  it('passes remote http(s) and data URIs straight through', () => {
    expect(resolveMarkdownImageSource('https://example.com/a.png', FILE_DIR)).toEqual({
      kind: 'remote',
      url: 'https://example.com/a.png',
    });
    expect(resolveMarkdownImageSource('http://example.com/a.png')).toEqual({
      kind: 'remote',
      url: 'http://example.com/a.png',
    });
    expect(resolveMarkdownImageSource('data:image/png;base64,AAAA')).toEqual({
      kind: 'remote',
      url: 'data:image/png;base64,AAAA',
    });
  });
});

describe('fileArtifactPathsFromToolCall — text_editor', () => {
  const call = (args: Record<string, unknown>) =>
    fileArtifactPathsFromToolCall('developer__text_editor', args, WORKING_DIR);

  it('previews a written markdown report', () => {
    expect(call({ command: 'write', path: '/home/ada/project/report.md' })).toEqual([
      '/home/ada/project/report.md',
    ]);
  });

  it('previews a written R script', () => {
    expect(call({ command: 'write', path: '/home/ada/project/analysis.R' })).toEqual([
      '/home/ada/project/analysis.R',
    ]);
  });

  it('previews edits, inserts and diffs, not just fresh writes', () => {
    for (const command of ['str_replace', 'insert', 'create', 'diff']) {
      expect(call({ command, path: '/tmp/notes.md' })).toEqual(['/tmp/notes.md']);
    }
  });

  it('does not preview a file the agent merely viewed', () => {
    expect(call({ command: 'view', path: '/tmp/notes.md' })).toEqual([]);
  });

  it('does not preview an undo', () => {
    expect(call({ command: 'undo_edit', path: '/tmp/notes.md' })).toEqual([]);
  });

  it('accepts the file_path alias some models emit', () => {
    expect(call({ command: 'write', file_path: '/tmp/x.csv' })).toEqual(['/tmp/x.csv']);
  });

  it('anchors a relative path to the working directory', () => {
    expect(call({ command: 'write', path: 'docs/summary.md' })).toEqual([
      '/home/ada/project/docs/summary.md',
    ]);
  });

  it('ignores calls with no path at all', () => {
    expect(call({ command: 'write' })).toEqual([]);
  });
});

describe('fileArtifactPathsFromToolCall — other writing tools', () => {
  it('handles write_file and create_file', () => {
    expect(fileArtifactPathsFromToolCall('write_file', { path: '/tmp/a.md' }, WORKING_DIR)).toEqual(
      ['/tmp/a.md']
    );
    expect(
      fileArtifactPathsFromToolCall('create_file', { filename: '/tmp/b.csv' }, WORKING_DIR)
    ).toEqual(['/tmp/b.csv']);
  });

  it('ignores tools that do not write files', () => {
    expect(
      fileArtifactPathsFromToolCall('developer__list_files', { path: '/tmp/a.md' }, WORKING_DIR)
    ).toEqual([]);
    expect(
      fileArtifactPathsFromToolCall('autovisualiser__show_chart', { data: {} }, WORKING_DIR)
    ).toEqual([]);
  });

  it('ignores non-object arguments', () => {
    expect(fileArtifactPathsFromToolCall('write_file', 'oops', WORKING_DIR)).toEqual([]);
    expect(fileArtifactPathsFromToolCall('write_file', null, WORKING_DIR)).toEqual([]);
  });
});

describe('fileArtifactPathsFromToolCall — shell', () => {
  const shell = (command: string) =>
    fileArtifactPathsFromToolCall('developer__shell', { command }, WORKING_DIR);

  it('catches a redirect target', () => {
    expect(shell('python summarize.py > results/summary.csv')).toEqual([
      '/home/ada/project/results/summary.csv',
    ]);
  });

  it('catches an append redirect', () => {
    expect(shell('echo hi >> log.txt')).toEqual(['/home/ada/project/log.txt']);
  });

  it('catches conventional output flags', () => {
    expect(shell('Rscript plot.R -o figure.png')).toEqual(['/home/ada/project/figure.png']);
    expect(shell('pandoc a.md --output report.html')).toEqual(['/home/ada/project/report.html']);
  });

  it('handles quoted output paths with spaces', () => {
    expect(shell('convert in.png -o "my figures/out.png"')).toEqual([
      '/home/ada/project/my figures/out.png',
    ]);
  });

  it('ignores stderr redirection and other non-file targets', () => {
    expect(shell('make 2>&1')).toEqual([]);
    expect(shell('cat a.md > /dev/null')).toEqual([]);
  });

  it('does not turn a plain listing into artifacts', () => {
    expect(shell('ls -la')).toEqual([]);
    expect(shell('cat report.md')).toEqual([]);
  });

  it('ignores redirect targets with no previewable extension', () => {
    expect(shell('./build > out.bin')).toEqual([]);
  });

  it('finds several outputs in one command', () => {
    expect(shell('a.sh > one.csv; b.sh > two.csv')).toEqual([
      '/home/ada/project/one.csv',
      '/home/ada/project/two.csv',
    ]);
  });

  it('file-link reliability: anchors output after a success-guarded literal cd', () => {
    expect(shell('cd reports && python summarize.py > summary.csv')).toEqual([
      '/home/ada/project/reports/summary.csv',
    ]);
    expect(shell('cd "/tmp/Other Study" && echo done > summary.md')).toEqual([
      '/tmp/Other Study/summary.md',
    ]);
  });

  it.each([
    ['/tmp/run', '/tmp/run/report.md'],
    ['runs/one', '/home/ada/project/runs/one/report.md'],
  ])(
    'file-link reliability: honors the actual shell working_directory argument: %s',
    (directory, path) => {
      expect(
        fileArtifactPathsFromToolCall(
          'developer__shell',
          {
            command: 'echo x > report.md',
            working_directory: directory,
          },
          WORKING_DIR
        )
      ).toEqual([path]);
    }
  );

  it('file-link reliability: carries a literal wrapped shell working_directory into output discovery', () => {
    expect(
      fileArtifactPathsFromToolCall(
        'code_execution__execute_code',
        {
          code: 'await developer.shell({ command: "echo x > report.md", working_directory: "/tmp/run" });',
        },
        WORKING_DIR
      )
    ).toEqual(['/tmp/run/report.md']);
  });

  it.each([
    'cd "$OUTPUT_DIR" && echo done > summary.md',
    'cd reports; echo done > summary.md',
    'if check; then cd reports; fi; echo done > summary.md',
    '(cd reports && echo done > summary.md)',
  ])('file-link reliability: refuses an ambiguous shell cwd: %s', (command) => {
    expect(shell(command)).toEqual([]);
  });

  it('file-link reliability: preserves absolute output despite an unresolved shell cwd', () => {
    expect(shell('cd "$OUTPUT_DIR" && echo done > /tmp/summary.md')).toEqual(['/tmp/summary.md']);
  });

  it('file-link reliability: keeps sequential literal cd scopes distinct', () => {
    expect(shell('cd a && echo x > first.md && cd b && echo y > second.md')).toEqual([
      '/home/ada/project/a/first.md',
      '/home/ada/project/a/b/second.md',
    ]);
  });

  it('file-link reliability: does not interpret quoted command text as shell execution', () => {
    expect(shell('echo "cd a" > out.md')).toEqual(['/home/ada/project/out.md']);
    expect(shell("printf 'echo > /elsewhere/report.md'")).toEqual([]);
  });

  it.each([
    'cd - && echo x > report.md',
    'cd ~other && echo x > report.md',
    'cd reports[12] && echo x > report.md',
    "cd '~/reports' && echo x > report.md",
    'echo x > report[12].md',
    'echo x > report\\ name.md',
  ])('file-link reliability: refuses nonliteral shell pathname semantics: %s', (command) => {
    expect(shell(command)).toEqual([]);
  });
});

describe('fileArtifactPathsFromToolCall — code_execution wrapper', () => {
  // With the `code_execution` extension enabled (the default), the model wraps
  // every real tool call inside `code_execution__execute_code`. Before the fix
  // these all returned [] and no written file ever reached the artifact panel.
  const exec = (code: string, workingDir: string | undefined = WORKING_DIR) =>
    fileArtifactPathsFromToolCall(
      'code_execution__execute_code',
      { code, tool_graph: {} },
      workingDir
    );

  it('extracts an absolute text_editor create — the exact live repro', () => {
    // Regression guard for /tmp/biookf-rebuild/SCHEMA.md not previewing.
    const code = [
      'import { text_editor } from "developer";',
      'const r = text_editor({ path: "/tmp/biookf-rebuild/SCHEMA.md", command: "create", file_text: "x" });',
      'record_result(r);',
    ].join('\n');
    expect(exec(code)).toEqual(['/tmp/biookf-rebuild/SCHEMA.md']);
  });

  it('resolves a `${dir}` template path against a const-string binding', () => {
    const code = [
      'import { text_editor } from "developer";',
      'const dir = "/tmp/biookf-rebuild";',
      'text_editor({ path: `${dir}/SCHEMA.md`, command: "str_replace", old_str: "a", new_str: "b" });',
    ].join('\n');
    expect(exec(code)).toEqual(['/tmp/biookf-rebuild/SCHEMA.md']);
  });

  it('does NOT surface a text_editor view (read, not a write)', () => {
    const code = [
      'import { text_editor } from "developer";',
      'const dir = "/tmp/biookf-rebuild";',
      'const s = text_editor({ path: `${dir}/SCHEMA.md`, command: "view" });',
    ].join('\n');
    expect(exec(code)).toEqual([]);
  });

  it('extracts a shell redirect target written inside the blob', () => {
    const code = [
      'import { shell } from "developer";',
      'const out = shell({ command: "python summarize.py > results/summary.csv" });',
    ].join('\n');
    expect(exec(code)).toEqual(['/home/ada/project/results/summary.csv']);
  });

  it('extracts from a String.raw multi-line shell command', () => {
    const code = [
      'import { shell } from "developer";',
      'const out = shell({ command: String.raw`set -e',
      'cd /tmp/biookf-rebuild',
      'printf "hello" > /tmp/biookf-rebuild/notes.md` });',
    ].join('\n');
    expect(exec(code)).toEqual(['/tmp/biookf-rebuild/notes.md']);
  });

  it('handles an embedded write_file with no command gate', () => {
    const code = 'write_file({ file_path: "/tmp/out/report.html", content: "<p>hi</p>" });';
    expect(exec(code)).toEqual(['/tmp/out/report.html']);
  });

  it('dedupes a file written and then re-edited in the same blob', () => {
    const code = [
      'const dir = "/tmp/biookf-rebuild";',
      'text_editor({ path: `${dir}/SCHEMA.md`, command: "create", file_text: "a" });',
      'text_editor({ path: `${dir}/SCHEMA.md`, command: "str_replace", old_str: "a", new_str: "b" });',
    ].join('\n');
    expect(exec(code)).toEqual(['/tmp/biookf-rebuild/SCHEMA.md']);
  });

  it('does not let a file_text value spoof a path key', () => {
    const code =
      'text_editor({ command: "create", path: "/tmp/real.md", file_text: "see path: /etc/passwd here" });';
    expect(exec(code)).toEqual(['/tmp/real.md']);
  });

  it('skips a python-heredoc write it cannot statically resolve — no false artifact', () => {
    const code = [
      'import { shell } from "developer";',
      'shell({ command: String.raw`python3 - <<PY',
      'from pathlib import Path',
      "Path('SCHEMA.md').write_text('x')",
      'PY` });',
    ].join('\n');
    expect(exec(code)).toEqual([]);
  });

  it('returns [] when the code arg is missing or non-string', () => {
    expect(fileArtifactPathsFromToolCall('code_execution__execute_code', {}, WORKING_DIR)).toEqual(
      []
    );
    expect(
      fileArtifactPathsFromToolCall('code_execution__execute_code', { code: 42 }, WORKING_DIR)
    ).toEqual([]);
  });
});

describe('withHostTheme', () => {
  it('injects the host theme right after <head> so it runs before the figure runtime', () => {
    const html =
      '<!doctype html><html><head><script>/*common*/</script></head><body></body></html>';
    const out = withHostTheme(html, 'light');
    expect(out).toContain('<head><script>window.__BR_VIZ_HOST_THEME__="light";</script>');
    // ...and it precedes the figure's own runtime script.
    expect(out.indexOf('__BR_VIZ_HOST_THEME__')).toBeLessThan(out.indexOf('/*common*/'));
  });

  it('carries dark through and falls back to a prefix when there is no <head>', () => {
    expect(withHostTheme('<html><head></head></html>', 'dark')).toContain(
      '__BR_VIZ_HOST_THEME__="dark"'
    );
    expect(withHostTheme('<div>no head</div>', 'light')).toBe(
      '<script>window.__BR_VIZ_HOST_THEME__="light";</script><div>no head</div>'
    );
  });
});

describe('sandboxedSurface', () => {
  it('returns the generated tokens for every family and mode', () => {
    for (const family of THEME_FAMILY_IDS) {
      for (const mode of ['light', 'dark'] as const) {
        expect(sandboxedSurface(family, mode)).toBe(GENERATED_THEMES[family][mode].surface);
      }
    }
  });

  // The whole point: a sandboxed preview must read the ACTIVE family's tokens,
  // not a hardcoded pair. This used to be asserted on the GROUND — the three
  // families were required to disagree about `background`. They no longer do:
  // neutrals are shared infrastructure and all three dark previews paint
  // #1b1b19 by design, so that assertion would now fail for the right reason.
  //
  // The guard moves to the axis that survives. A family is its INK and its
  // accent, so `foreground` is what must still differ three ways — and it is a
  // strictly better probe than the ground was, because a re-hardcoded preview
  // would pin the ink too.
  it('gives each family a distinct ink in dark mode', () => {
    const inks = THEME_FAMILY_IDS.map((family) => sandboxedSurface(family, 'dark').foreground);
    expect(new Set(inks).size).toBe(THEME_FAMILY_IDS.length);
  });

  // The other half of the same rule, pinned so it cannot drift back: the ground
  // is SHARED. A family that reintroduced its own is the regression now.
  it('paints one shared ground across families, in both modes', () => {
    for (const mode of ['light', 'dark'] as const) {
      const grounds = THEME_FAMILY_IDS.map((family) => sandboxedSurface(family, mode).background);
      expect(new Set(grounds).size).toBe(1);
    }
  });

  // `theme_family` is free-form localStorage: a build that once shipped a family
  // we later removed leaves an id nothing maps to. Returning `undefined` there
  // would put `background:undefined` into the srcdoc and paint the document
  // unstyled, so fall back the way BioRouterMark does.
  it('falls back to parchment for an unknown family', () => {
    expect(sandboxedSurface('nonesuch' as ThemeFamily, 'dark')).toBe(
      GENERATED_THEMES.parchment.dark.surface
    );
  });
});

/**
 * Defect 3.5's other half, pinned rather than changed. The prose matcher was
 * the only place a directory was rejected — the tool-call extractor has never
 * applied an extension filter to a write tool's path argument, so a tool that
 * creates a folder already yields it. Asserting that here keeps a future
 * "tighten the collector" change from quietly re-introducing the same gap on
 * the receipt side, where it would be much harder to notice.
 */
describe('fileArtifactPathsFromToolCall — a folder is a path like any other', () => {
  it('keeps an extensionless write target', () => {
    expect(
      fileArtifactPathsFromToolCall('developer__write_file', { path: 'results' }, '/work')
    ).toEqual(['/work/results']);
  });
});
