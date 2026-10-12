import { describe, expect, it } from 'vitest';
import { isCodePath } from './artifactCodePath';
import { languageFromPath } from './artifactUtils';

describe("what counts as code, by the code view's own table (V2-R3-3)", () => {
  // The ones the round-3 screenshots showed drawing the document glyph beside
  // `.py`, `.ts`, `.rs` and `.sql`, which drew the code glyph.
  it.each([
    'a.R',
    'a.r',
    'run.sh',
    'a.zsh',
    'site.css',
    'Cargo.toml',
    'feed.xml',
    'pipeline.nf',
    'rules.smk',
    'a.go',
    'a.java',
    'a.c',
    'a.h',
    'a.cpp',
    'a.jl',
    'a.rb',
    'a.pl',
    'a.py',
    'a.ts',
    'a.tsx',
    'a.js',
    'a.rs',
    'a.sql',
    'a.json',
    'a.yaml',
    'a.yml',
    'config.ini',
    'Makefile',
    'Dockerfile',
    'Snakefile',
  ])('%s is code', (path) => {
    expect(isCodePath(path)).toBe(true);
  });

  it.each([
    'notes.md',
    'report.Rmd',
    'report.qmd',
    'readme.txt',
    'run.log',
    'results.csv',
    'results.tsv',
    'paper.pdf',
    'deck.pptx',
    'sheet.xlsx',
    'letter.docx',
    'analysis.ipynb',
  ])('%s is not code', (path) => {
    expect(isCodePath(path)).toBe(false);
  });

  it('asks the code view itself, so a file the highlighter paints as prose is not code', () => {
    // The two answers come from one table: whatever `languageFromPath` calls
    // markdown, text, a log or a table is a document here too.
    for (const path of ['a.md', 'a.txt', 'a.log', 'a.csv', 'a.tsv']) {
      expect(['markdown', 'text', 'log', 'csv-table', 'tsv-table']).toContain(
        languageFromPath(path)
      );
      expect(isCodePath(path)).toBe(false);
    }
  });
});
