import { useMemo } from 'react';
import MarkdownContent from '../MarkdownContent';
import type { ArtifactSource } from './artifactTypes';
import { dirnameFromPath, frontMatterFields, splitFrontMatter } from './artifactUtils';

/**
 * A markdown file (`.md`, R Markdown, Quarto) as a document on the panel's
 * paper: its front matter lifted into a title block, the body set as prose in
 * the 760px column.
 *
 * The paper look is authored CSS in main.css ("Preview paper"), keyed on the
 * `br-paper-*` classes below — never on Tailwind strings, which the class
 * scanner cannot see under BIOROUTER_NO_HMR.
 */
export default function MarkdownDocument({
  text,
  path,
  onOpenArtifact,
}: {
  text: string;
  path: string;
  onOpenArtifact?: (artifact: ArtifactSource) => void;
}) {
  const { frontMatter, body } = useMemo(() => splitFrontMatter(text), [text]);
  const header = useMemo(
    () => (frontMatter === null ? null : frontMatterFields(frontMatter)),
    [frontMatter]
  );
  return (
    <article className="br-preview-measure br-paper-doc" data-preview-intrinsic="">
      {header && (header.title || header.subtitle || header.byline.length > 0 || header.rest) && (
        <header className="br-paper-frontmatter">
          {header.title && <h1 className="br-paper-title">{header.title}</h1>}
          {header.subtitle && <p className="br-paper-subtitle">{header.subtitle}</p>}
          {header.byline.length > 0 && (
            <p className="br-paper-byline">
              {header.byline.map((part, index) => (
                <span key={index}>{part}</span>
              ))}
            </p>
          )}
          {header.rest && (
            // The rest of the front matter is configuration, not prose: one
            // quiet, CLOSED disclosure away, as the YAML it is — never dropped,
            // and not a second box stacked under the title before the report
            // begins.
            <details className="br-paper-frontmatter-more">
              <summary>Front matter</summary>
              <MarkdownContent
                content={`\`\`\`yaml\n${header.rest}\n\`\`\``}
                className="br-paper-prose br-paper-frontmatter-yaml"
                variant="document"
              />
            </details>
          )}
        </header>
      )}
      {/* Anchor relative image/link paths against the FILE's own directory
          (not the app cwd), and let sibling-file links open in this panel. */}
      <MarkdownContent
        content={body}
        className="br-paper-prose"
        workingDir={dirnameFromPath(path)}
        onOpenArtifact={onOpenArtifact}
        variant="document"
      />
    </article>
  );
}
