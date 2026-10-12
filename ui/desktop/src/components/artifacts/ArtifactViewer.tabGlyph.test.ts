import { describe, expect, it } from 'vitest';
import {
  ChartColumn,
  Code,
  File,
  FileText,
  Folder,
  GitBranch,
  Globe,
  Image,
} from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import type { ArtifactSource } from './artifactTypes';
import { iconForArtifact } from './ArtifactViewer';

const file = (path: string): ArtifactSource => ({ kind: 'file', title: 'x', path });

// The glyph is a pure function of the source and of what the panel loaded for it.
// The tab strip's own rendering of it is asserted in ArtifactViewer.test.tsx.
describe('the glyph a preview tab draws (V2-R3-3)', () => {
  it('lets what the panel loaded outrank the name', () => {
    // `repo` is only a path. Loaded as a plain folder or as a repository it says so.
    expect(iconForArtifact(file('/w/repo'), 'directory')).toBe(Folder);
    expect(iconForArtifact(file('/w/repo'), 'gitDirectory')).toBe(GitBranch);
    // A dotted folder name is still a folder.
    expect(iconForArtifact(file('/w/run.v2'), 'directory')).toBe(Folder);
    expect(iconForArtifact(file('/w/shot'), 'image')).toBe(Image);
    expect(iconForArtifact(file('/w/report'), 'document')).toBe(FileText);
    expect(iconForArtifact(file('/w/thing.py'), 'binary')).toBe(File);
    expect(iconForArtifact(file('/w/page.txt'), 'html')).toBe(Globe);
  });

  it('draws code, prose, images and pages by name until a preview has loaded', () => {
    for (const path of ['/w/lib.rs', '/w/a.R', '/w/run.sh', '/w/Cargo.toml', '/w/feed.xml']) {
      expect(iconForArtifact(file(path))).toBe(Code);
    }
    for (const path of ['/w/notes.md', '/w/run.log', '/w/t.csv', '/w/p.pdf', '/w/a.ipynb']) {
      expect(iconForArtifact(file(path))).toBe(FileText);
    }
    expect(iconForArtifact(file('/w/figure.png'))).toBe(Image);
    expect(iconForArtifact(file('/w/FIGURE.PNG'))).toBe(Image);
    expect(iconForArtifact(file('/w/page.html'))).toBe(Globe);
    expect(iconForArtifact(file('/w/notes.weird'))).toBe(FileText);
  });

  it('draws a name with no extension as a folder until the preview says otherwise', () => {
    expect(iconForArtifact(file('/Users/me/repo'))).toBe(Folder);
    expect(iconForArtifact(file('/Users/me/mouse_ecig'))).toBe(Folder);
    expect(iconForArtifact(file('/Users/me/results/'))).toBe(Folder);
    // A conventional file name is a file, and code.
    expect(iconForArtifact(file('/w/Makefile'))).toBe(Code);
  });

  it('falls back to the name for text, for a failed read and for nothing loaded', () => {
    expect(iconForArtifact(file('/w/lib.rs'), 'text')).toBe(Code);
    expect(iconForArtifact(file('/w/lib.rs'), 'error')).toBe(Code);
    expect(iconForArtifact(file('/w/notes.md'), 'text')).toBe(FileText);
  });

  it('draws a figure, a page address and nothing', () => {
    expect(iconForArtifact({ kind: 'html', title: 'f', html: '<p>x</p>' })).toBe(ChartColumn);
    expect(iconForArtifact({ kind: 'externalUrl', title: 'f', url: 'https://example.test' })).toBe(
      Globe
    );
    expect(iconForArtifact(null)).toBe(File);
  });

  it('draws an Agent Drafter app with the application glyph its transcript card draws', () => {
    // One entity, one glyph (design.md §3.9): the app's card in the build chat
    // draws the registry's application glyph, so its preview tab draws the same
    // one rather than the figure glyph every other `ui://` resource takes.
    const html = (sourceUri: string): ArtifactSource => ({
      kind: 'html',
      title: 'a',
      html: '<p>x</p>',
      sourceUri,
    });
    expect(iconForArtifact(html('ui://agent-drafter/lab-notebook'))).toBe(ENTITY_ICONS.application);
    expect(iconForArtifact(html('UI://Agent_Drafter/lab-notebook'))).toBe(ENTITY_ICONS.application);
    expect(
      iconForArtifact({
        kind: 'mcpResource',
        title: 'a',
        resource: { uri: 'ui://agent-drafter/lab-notebook', text: '' },
      })
    ).toBe(ENTITY_ICONS.application);
    // Auto Visualiser figures and reports keep the figure glyph.
    expect(iconForArtifact(html('ui://autovisualiser/chart'))).toBe(ChartColumn);
    expect(iconForArtifact(html('ui://dashboard/report'))).toBe(ChartColumn);
  });
});
