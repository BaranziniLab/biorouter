import './tool-call.css';
import type { ComponentType } from 'react';
import { EmbeddedResource } from '../api';
import { ChartColumn, FileText, Globe } from './icons/app-icons';
import { ENTITY_ICONS } from './icons/entity-icons';
import type { ArtifactSource } from './artifacts/artifactTypes';
import { artifactSourceFromResource, titleFromResourceUri } from './artifacts/artifactUtils';
import { ARTIFACT_CARD_COPY } from './toolCallCopy';

interface MCPUIResourceRendererProps {
  content: EmbeddedResource & { type: 'resource' };
  onOpenArtifact: (artifact: ArtifactSource) => void;
}

export type ArtifactCardKind = keyof typeof ARTIFACT_CARD_COPY.kind;

const KIND_GLYPHS: Record<ArtifactCardKind, ComponentType<{ className?: string }>> = {
  figure: ChartColumn,
  // A report is a document of figures (`render_dashboard`), so it reads as a page.
  report: FileText,
  app: ENTITY_ICONS.application,
  page: Globe,
  resource: FileText,
};

/**
 * What the card holds, in a person's words: never a MIME type. Read from the
 * artifact source (an external page opens in the browser) and the `ui://`
 * address the tool chose (`ui://agent-drafter/<id>` is an app,
 * `ui://dashboard/report` is a report, every other Auto Visualiser address is
 * a figure).
 */
export function artifactCardKind(source: ArtifactSource, uri?: string): ArtifactCardKind {
  if (source.kind === 'externalUrl') return 'page';
  const address = (uri ?? '').toLowerCase();
  if (/^ui:\/\/agent[-_]drafter\//.test(address)) return 'app';
  if (/^ui:\/\/dashboard\//.test(address)) return 'report';
  if (source.kind === 'html') return 'figure';
  return 'resource';
}

/**
 * A `ui://` resource — an Auto Visualiser figure, an Agent Drafter app card —
 * has exactly one display surface: the artifact side panel. In the transcript it
 * is only ever a card you click to open it there. It is never rendered inline,
 * and there is no second "expand" destination.
 *
 * An inline iframe would be a second renderer of the same document, with its own
 * CSP, its own action channel and its own resize behaviour — three things that
 * diverge from the panel's silently, because nothing makes them agree. The panel
 * already does all three, so this component renders one of exactly two things:
 * the card (whenever `artifactSourceFromResource` yields a source) or a
 * no-preview note (when it yields none).
 *
 * The card spans the 760px column, carries a hairline and no shadow (it sits on
 * the page, it does not float), and holds a 32px kind tile, the title and the
 * kind. It only opens; there is no delete control (CLAUDE.md, Artifact side
 * panel). Styled in authored CSS (`.br-artifact-card`, tool-call.css).
 */
export default function MCPUIResourceRenderer({
  content,
  onOpenArtifact,
}: MCPUIResourceRendererProps) {
  const resource = content.resource as { uri?: string; mimeType?: string };

  const fallbackArtifactTitle =
    titleFromResourceUri(resource.uri) ||
    resource.uri?.split('/').pop() ||
    ARTIFACT_CARD_COPY.fallbackTitle;
  const artifactSource = artifactSourceFromResource(content, fallbackArtifactTitle);
  const artifactTitle = artifactSource?.title ?? fallbackArtifactTitle;

  const handleOpenArtifact = async () => {
    if (!artifactSource) return;
    // An external URL is a live web page we do not hold a copy of, so it belongs
    // in the user's own browser rather than in the panel.
    if (artifactSource.kind === 'externalUrl') {
      await window.electron.openExternal(artifactSource.url);
      return;
    }
    onOpenArtifact(artifactSource);
  };

  if (artifactSource) {
    const kind = artifactCardKind(artifactSource, resource.uri);
    const Glyph = KIND_GLYPHS[kind];
    return (
      <button
        type="button"
        onClick={handleOpenArtifact}
        aria-label={ARTIFACT_CARD_COPY.openLabel(
          artifactTitle,
          artifactSource.kind === 'externalUrl'
        )}
        data-artifact-kind={kind}
        className="br-artifact-card"
      >
        <span className="br-artifact-card-tile" aria-hidden="true">
          <Glyph />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-label text-text-default">{artifactTitle}</span>
          <span className="block truncate text-supporting text-text-muted">
            {ARTIFACT_CARD_COPY.kind[kind]}
          </span>
        </span>
      </button>
    );
  }

  // No source at all: the uri was too long, an HTML payload would not decode, or
  // a uri-list held no usable URL. There is nothing the panel could show.
  return (
    <div className="text-supporting text-text-muted" role="status">
      {ARTIFACT_CARD_COPY.noPreview(artifactTitle)}
    </div>
  );
}
