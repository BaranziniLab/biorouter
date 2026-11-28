# Remote images in markdown

> **What this is.** The rule that a web image in markdown (an assistant reply, a tool result, a saved or shared chat, a markdown file in the side panel) loads only when a person clicks it, and how the desktop fetches that one image.
> **Status:** Current. Built for OP-P0-09, the owner's answer to D15 (remote images click-to-load in every chat).
> **Audience:** contributors working on the desktop renderer, the main process or the page policies.

## The rule

An `<img>` with a web address sends a request the moment it is drawn. The request is a read receipt, it carries the reader's network address, and the address itself can carry anything. A model steered by a web page or a connector can write a chat's private text into that address, so drawing the image would send it out with no privacy gate in the way.

So a remote image in markdown is a button until a person clicks it:

- The button says **Load image from** and the host. The host is the URL parser's own, so an internationalised name shows in its ASCII form and a look-alike cannot pass for the real one.
- Hovering or focusing it shows the full address in the app's dark label tooltip (cut at 300 characters).
- A click loads that one image. A second copy of the same address waits for its own click, and so does another image from the same host. So does the same message drawn again, in this chat or another: nothing is stored.
- An `http:` address is asked for over `https:`, the upgrade the page policy always made.
- An address with a user name or password is never offered. It shows the unavailable chip.
- Inside a link (a README badge, for example) the image is a chip naming its host, not a second button: the link is what a click there opens.

Every markdown surface renders through `MarkdownContent`, so the rule holds in a live chat, History, a shared chat, tool results and the side panel's markdown preview alike. Crew's message bodies never draw an image at all (`crew/timeline/MessageBody.tsx`).

What still draws at once, because it requests nothing from the web: a local image the file read allows, and the images the app makes itself as `data:` or `blob:` URLs (tool result images, notebook outputs, file previews).

## Network paths are not local images

On Windows a path that starts with two separators is a network path: `\\host\share\x.png`, and also `//host/x.png`, `file:////host/x.png` and their encoded forms. Reading one, even the `realpath` and `lstat` a file read starts with, opens an SMB connection to that host and may try to sign in to it. So a markdown image written that way would reach a host of the model's choosing with no click.

Two checks close it:

- `resolveMarkdownImageSource` (`components/artifacts/artifactUtils.ts`) treats such an image as unavailable, in every spelling, unless it lies on the share the markdown itself sits on. A `file:` URL that names a host is unavailable too.
- The main process refuses such a path in every preview read (`read-artifact-file`, `read-file`, `check-file-paths`, `open-directory-in-explorer`) before touching the disk, in every permission mode, unless it lies on a share that is one of the allowed roots, such as the folder the person pointed the chat at (`reachesUnchosenNetworkHost` in `utils/pathContainment.ts`). That covers file paths a reply names as well as images. The check is Windows only: elsewhere two leading slashes name the root folder.

A file the agent wrote to some other network share in Fully Automatic mode is therefore not previewable on Windows. Map the share to a drive letter, or point the chat at it, to preview it.

## How the desktop fetches a clicked image

The renderer asks the main process over `load-remote-image` (`REMOTE_IMAGE_CHANNEL`). `utils/remoteImageFetch.ts` fetches the address with Node's `https` module, which keeps no cookie jar and no cache, and sends no referrer. It:

- resolves every hop, redirects included, and refuses a loopback, private or link-local address, the rule the Biorouter Browser applies;
- connects to the address it checked and to no other (`pinnedRequestOptions`), so a host whose DNS answer changes between the check and the connection cannot reach a private address; the certificate is still checked against the name;
- follows at most five redirects, reads at most 20 MB, and gives up after 20 seconds;
- refuses an answer that does not say it is an image.

The renderer draws the bytes from a `blob:` URL and revokes it when the image goes away. Only an app window's own page may ask.

A clicked image drawn by the renderer itself would have stored the host's cookie in the persistent renderer partition and sent it back on every later click, in any chat. Fetching in the main process means the tracker sees each click alone.

The request does not go through the proxy the app gives its windows (`configureProxy` in `main.ts`). On a network that reaches the web only through a proxy, a click shows the unavailable chip rather than the image. That fails closed; nothing is sent around the proxy that the network would have refused.

## The page policies

- The desktop session's header in `main.ts` is `img-src 'self' data: blob:`. No `<img>` in the desktop renderer can reach the web on its own, so a component that forgot the click would still request nothing.
- The `<meta>` in `index.html` keeps `https:` for one reason: it is also the only image policy of the page `biorouter serve` sends to a browser, where there is no main process and the click sets the address itself. In the desktop both policies apply and the stricter wins.

In a browser on `biorouter serve` the click still waits for the person, and the request carries no referrer, but it is the browser's own request: it sends whatever cookies that browser keeps for the image's host, as its third-party cookie settings allow. The no-cookie property above holds for the desktop only.

`src/imgSrcCsp.test.ts` pins both, and fails when the desktop header admits web images again.

## Tests

- `components/MarkdownContent.remoteImages.test.tsx`: the button, the tooltip, one image per click, nothing remembered, the browser path, links, and the side panel's markdown preview.
- `components/BioRouterMessage.remoteImages.test.tsx`: an injected assistant message with a tracking image, in a live chat and in History. It records every image source, `srcset` and `fetch` the renderer starts and requires none of them to name the tracker before the click.
- `utils/remoteImageFetch.test.ts`: the main-process fetch, including the pinned connection.
- `components/artifacts/artifactUtils.test.ts` and `utils/pathContainment.test.ts`: network paths, refused in the renderer and in the main process before the disk is touched.

jsdom has no CSP engine and loads no images, so these tests watch what the renderer asks for, not what the network sees.

## Related documentation

- [Which session the app's hooks are installed on](renderer-session-hooks.md): why the header policy reaches the renderer's partition.
- [Where a generated artifact is displayed](artifact-display-surfaces.md): the side panel that previews markdown files.
- [Privacy tiers](../security/privacy-tiers.md): the gates that decide which models and sources a chat may use, which do not inspect an image address.
