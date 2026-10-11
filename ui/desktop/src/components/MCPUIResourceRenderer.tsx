import { EmbeddedResource } from '../api';
import { Maximize2 } from './icons/app-icons';
import type { ArtifactSource } from './artifacts/artifactTypes';
import { artifactSourceFromResource, titleFromResourceUri } from './artifacts/artifactUtils';
import { openInBrowser } from '../utils/openInBrowser';

interface MCPUIResourceRendererProps {
  content: EmbeddedResource & { type: 'resource' };
  onOpenArtifact: (artifact: ArtifactSource) => void;
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
 * The card's chrome carries no colour literals — it is styled entirely in
 * semantic Tailwind classes (`bg-background-default`, `text-text-muted`,
 * `border-border-subtle`), which are CSS custom properties that main.css
 * re-points per `[data-theme]`. So it is theme-family-aware for free.
 */
export default function MCPUIResourceRenderer({
  content,
  onOpenArtifact,
}: MCPUIResourceRendererProps) {
  const resource = content.resource as { uri?: string; mimeType?: string };

  const fallbackArtifactTitle =
    titleFromResourceUri(resource.uri) || resource.uri?.split('/').pop() || 'Artifact';
  const artifactSource = artifactSourceFromResource(content, fallbackArtifactTitle);
  const artifactTitle = artifactSource?.title ?? fallbackArtifactTitle;

  const handleOpenArtifact = async () => {
    if (!artifactSource) return;
    // An external URL is a live web page we do not hold a copy of, so it belongs
    // in the user's own browser rather than in the panel.
    if (artifactSource.kind === 'externalUrl') {
      await openInBrowser(artifactSource.url);
      return;
    }
    onOpenArtifact(artifactSource);
  };

  if (artifactSource) {
    const destination =
      artifactSource.kind === 'externalUrl' ? 'in the default browser' : 'in the artifact viewer';
    return (
      <div className="group mt-3 flex w-full max-w-xl items-center gap-2">
        <button
          type="button"
          onClick={handleOpenArtifact}
          aria-label={`Open ${artifactTitle} ${destination}`}
          className="flex min-w-0 flex-1 items-center gap-3 rounded-lg border border-border-subtle bg-background-default/75 px-3 py-2.5 text-left shadow-popover transition-colors hover:bg-background-medium"
        >
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-background-medium text-text-muted">
            <Maximize2 className="h-4 w-4" aria-hidden="true" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium text-text-default">
              {artifactTitle}
            </span>
            <span className="block truncate text-xs text-text-muted">
              {resource.mimeType || 'text/html'}
            </span>
          </span>
        </button>
      </div>
    );
  }

  // No source at all: the uri was too long, an HTML payload would not decode, or
  // a uri-list held no usable URL. There is nothing the panel could show.
  return (
    <div className="mt-3 text-xs text-text-muted" role="status">
      No browser-safe preview is available for {artifactTitle}.
    </div>
  );
}
