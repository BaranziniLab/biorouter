import type { ArtifactSource } from './artifactTypes';

/**
 * How a link in a message opens (BB-02).
 *
 * A plain click is the person's gesture, and it is the whole consent a page load needs: the
 * link opens in the Biorouter Browser in this chat's panel. Cmd or Ctrl click and a middle click
 * open it in the default browser instead, after the usual confirmation. An agent supplied link
 * (an MCP resource, a `ui://` card) never goes through here, so it still shows the Open here card
 * and never loads by itself.
 */

/** Mark an external page artifact as opened by the person's click. Other kinds pass through. */
export function withLinkClickOrigin(artifact: ArtifactSource, now = Date.now()): ArtifactSource {
  if (artifact.kind !== 'externalUrl') return artifact;
  return {
    kind: 'externalUrl',
    title: artifact.title,
    url: artifact.url,
    openedBy: 'click',
    openedAt: now,
  };
}

type LinkPointer = {
  button: number;
  metaKey: boolean;
  ctrlKey: boolean;
};

/** Cmd or Ctrl click, or a middle click: the default browser, as in any app's link. */
export function opensInDefaultBrowser(event: LinkPointer): boolean {
  return event.button === 1 || event.metaKey || event.ctrlKey;
}
