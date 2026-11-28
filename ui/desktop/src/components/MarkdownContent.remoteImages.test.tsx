import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import MarkdownContent from './MarkdownContent';
import MarkdownDocument from './artifacts/MarkdownDocument';

/**
 * Remote images in markdown load on a click, in every chat (OP-P0-09, the
 * owner's answer to D15; triage defect B3-6).
 *
 * An `<img>` with a web address sends a request the moment it is drawn: a read
 * receipt, the reader's address, and whatever the address itself carries, which
 * a model steered by a web page or a connector can fill with the chat's text.
 * So a remote image is a button naming its host until the person clicks it, and
 * the click loads that one image and nothing else.
 */

const PLOT = 'https://cdn.example.org/fig/plot.png';

type LoadResult = { ok: true; mimeType: string; bytes: Uint8Array } | { ok: false; reason: string };

function installElectron(
  options: { loadRemoteImage?: ((url: string) => Promise<LoadResult>) | null } = {}
) {
  const loadRemoteImage =
    options.loadRemoteImage === null
      ? undefined
      : vi.fn(
          options.loadRemoteImage ??
            (async () => ({
              ok: true as const,
              mimeType: 'image/png',
              bytes: new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
            }))
        );
  const electron = {
    readArtifactFile: vi.fn(async (path: string) => ({
      kind: 'image',
      title: path.split('/').pop() ?? 'image',
      path,
      mimeType: 'image/png',
      dataUrl: 'data:image/png;base64,AAAA',
      size: 3,
      found: true,
    })),
    openExternal: vi.fn(async () => undefined),
    ...(loadRemoteImage ? { loadRemoteImage } : {}),
  };
  Object.defineProperty(window, 'electron', { configurable: true, value: electron });
  return { electron, loadRemoteImage };
}

let blobCount = 0;
const createdBlobs: Blob[] = [];
const revoked: string[] = [];

beforeEach(() => {
  blobCount = 0;
  createdBlobs.length = 0;
  revoked.length = 0;
  Object.defineProperty(URL, 'createObjectURL', {
    configurable: true,
    value: vi.fn((blob: Blob) => {
      createdBlobs.push(blob);
      blobCount += 1;
      return `blob:remote-image-${blobCount}`;
    }),
  });
  Object.defineProperty(URL, 'revokeObjectURL', {
    configurable: true,
    value: vi.fn((url: string) => revoked.push(url)),
  });
});

afterEach(() => {
  // @ts-expect-error: remove the per-test electron stub.
  delete window.electron;
  vi.restoreAllMocks();
});

/** Every `<img>` source in the document, so a test can say none of them is remote. */
function imageSources(): string[] {
  return [...document.querySelectorAll('img')].map((img) => img.getAttribute('src') ?? '');
}

describe('remote images in markdown', () => {
  it('draws a remote image as a button naming its host, and requests nothing', async () => {
    const { electron, loadRemoteImage } = installElectron();
    render(<MarkdownContent content={`![Plot](${PLOT})`} workingDir="/work" />);

    const button = await screen.findByRole('button', {
      name: /load image from cdn\.example\.org/i,
    });
    expect(button).toHaveTextContent('cdn.example.org');
    expect(imageSources()).toEqual([]);
    expect(loadRemoteImage).not.toHaveBeenCalled();
    expect(electron.readArtifactFile).not.toHaveBeenCalled();
  });

  it('names the image in the button when the markdown gives it alt text', async () => {
    installElectron();
    render(<MarkdownContent content={`![Survival curve](${PLOT})`} />);
    expect(
      await screen.findByRole('button', {
        name: /load image from cdn\.example\.org.*survival curve/i,
      })
    ).toBeInTheDocument();
  });

  it('shows the full address in the label tooltip', async () => {
    installElectron();
    const user = userEvent.setup();
    render(<MarkdownContent content={`![Plot](${PLOT})`} />);

    await screen.findByRole('button', { name: /load image from/i });
    await user.tab();
    expect(await screen.findByRole('tooltip')).toHaveTextContent(PLOT);
  });

  it('loads that one image on a click, through the main process, and shows it from memory', async () => {
    const { loadRemoteImage } = installElectron();
    render(<MarkdownContent content={`![Plot](${PLOT})`} />);

    fireEvent.click(await screen.findByRole('button', { name: /load image from/i }));

    const img = await screen.findByRole('img', { name: 'Plot' });
    expect(img.tagName).toBe('IMG');
    expect(img).toHaveAttribute('src', 'blob:remote-image-1');
    expect(loadRemoteImage).toHaveBeenCalledTimes(1);
    expect(loadRemoteImage).toHaveBeenCalledWith(PLOT);
    expect(createdBlobs[0]?.type).toBe('image/png');
    expect(imageSources().some((src) => src.startsWith('http'))).toBe(false);
  });

  it('loads only the image that was clicked', async () => {
    const { loadRemoteImage } = installElectron();
    const second = 'https://img.example.net/second.png';
    render(<MarkdownContent content={`![One](${PLOT})\n\n![Two](${second})`} />);

    const buttons = await screen.findAllByRole('button', { name: /load image from/i });
    expect(buttons).toHaveLength(2);
    fireEvent.click(buttons[0]);

    await screen.findByRole('img', { name: 'One' });
    expect(loadRemoteImage).toHaveBeenCalledTimes(1);
    expect(loadRemoteImage).toHaveBeenCalledWith(PLOT);
    expect(
      screen.getByRole('button', { name: /load image from img\.example\.net/i })
    ).toBeVisible();
    expect(imageSources()).toEqual(['blob:remote-image-1']);
  });

  it('loads only the clicked copy when the same address appears twice', async () => {
    const { loadRemoteImage } = installElectron();
    render(<MarkdownContent content={`![A](${PLOT})\n\n![B](${PLOT})`} />);

    const buttons = await screen.findAllByRole('button', { name: /load image from/i });
    fireEvent.click(buttons[1]);

    await screen.findByRole('img', { name: 'B' });
    expect(loadRemoteImage).toHaveBeenCalledTimes(1);
    expect(screen.getAllByRole('button', { name: /load image from/i })).toHaveLength(1);
  });

  it('remembers nothing: drawn again, the image is a button again', async () => {
    const { loadRemoteImage } = installElectron();
    const first = render(<MarkdownContent content={`![Plot](${PLOT})`} />);
    fireEvent.click(await screen.findByRole('button', { name: /load image from/i }));
    await screen.findByRole('img', { name: 'Plot' });
    first.unmount();
    expect(revoked).toContain('blob:remote-image-1');

    render(<MarkdownContent content={`![Plot](${PLOT})`} />);
    expect(await screen.findByRole('button', { name: /load image from/i })).toBeVisible();
    expect(imageSources()).toEqual([]);
    expect(loadRemoteImage).toHaveBeenCalledTimes(1);
  });

  it('keeps a loaded image while the message keeps streaming', async () => {
    const { loadRemoteImage } = installElectron();
    const view = render(<MarkdownContent content={`![Plot](${PLOT})`} />);
    fireEvent.click(await screen.findByRole('button', { name: /load image from/i }));
    await screen.findByRole('img', { name: 'Plot' });

    view.rerender(<MarkdownContent content={`![Plot](${PLOT})\n\nMore text arrives.`} />);
    await screen.findByText('More text arrives.');
    expect(screen.getByRole('img', { name: 'Plot' })).toHaveAttribute('src', 'blob:remote-image-1');
    expect(loadRemoteImage).toHaveBeenCalledTimes(1);
  });

  it('asks for an http image over https', async () => {
    const { loadRemoteImage } = installElectron();
    render(<MarkdownContent content="![Gel](http://lab.example.org/gel.png)" />);
    fireEvent.click(
      await screen.findByRole('button', { name: /load image from lab\.example\.org/i })
    );
    await waitFor(() =>
      expect(loadRemoteImage).toHaveBeenCalledWith('https://lab.example.org/gel.png')
    );
  });

  it('falls back to the unavailable chip when the image cannot be loaded', async () => {
    installElectron({ loadRemoteImage: async () => ({ ok: false, reason: 'not-public' }) });
    render(<MarkdownContent content={`![Plot](${PLOT})`} />);
    fireEvent.click(await screen.findByRole('button', { name: /load image from/i }));

    const chip = await screen.findByRole('img', { name: 'Plot' });
    expect(chip.tagName).toBe('SPAN');
    expect(imageSources()).toEqual([]);
  });

  it('never offers an address that carries a user name or password', async () => {
    const { loadRemoteImage } = installElectron();
    render(<MarkdownContent content="![Secret](https://user:pw@cdn.example.org/a.png)" />);

    const chip = await screen.findByRole('img', { name: 'Secret' });
    expect(chip.tagName).toBe('SPAN');
    expect(screen.queryByRole('button')).toBeNull();
    expect(loadRemoteImage).not.toHaveBeenCalled();
    expect(imageSources()).toEqual([]);
  });

  // react-markdown's URL filter drops a `data:` address in markdown before any
  // renderer sees it, so a model cannot draw one; that is unchanged here. What
  // matters is that it is never mistaken for a web image either.
  it('never turns a data address in markdown into a load button or a request', async () => {
    const { loadRemoteImage } = installElectron();
    render(<MarkdownContent content="![Inline](data:image/png;base64,AAAA) and text" />);

    await screen.findByText(/and text/);
    expect(screen.queryByRole('button')).toBeNull();
    expect(loadRemoteImage).not.toHaveBeenCalled();
    expect(imageSources().some((src) => src.startsWith('http'))).toBe(false);
  });

  it('still reads a local image through the allowlisted file read at once', async () => {
    const { electron, loadRemoteImage } = installElectron();
    render(<MarkdownContent content="![Figure](./fig.png)" workingDir="/work/docs" />);

    const img = await screen.findByRole('img', { name: 'Figure' });
    expect(img).toHaveAttribute('src', 'data:image/png;base64,AAAA');
    expect(electron.readArtifactFile).toHaveBeenCalledWith('/work/docs/fig.png');
    expect(loadRemoteImage).not.toHaveBeenCalled();
  });

  it('in a browser, where no main process can fetch it, the click loads the address itself with no referrer', async () => {
    installElectron({ loadRemoteImage: null });
    render(<MarkdownContent content={`![Plot](${PLOT})`} />);
    expect(imageSources()).toEqual([]);

    fireEvent.click(await screen.findByRole('button', { name: /load image from/i }));

    const img = await screen.findByRole('img', { name: 'Plot' });
    expect(img).toHaveAttribute('src', PLOT);
    expect(img).toHaveAttribute('referrerpolicy', 'no-referrer');
  });

  it('inside a link, names the host without a second button, and the link still works', async () => {
    const { loadRemoteImage } = installElectron();
    const onOpenArtifact = vi.fn();
    render(
      <MarkdownContent
        content="[![Build](https://img.shields.io/badge/build-passing-green.svg)](https://github.com/lab/tool)"
        onOpenArtifact={onOpenArtifact}
      />
    );

    const link = await screen.findByRole('button');
    expect(screen.getAllByRole('button')).toHaveLength(1);
    expect(link).toHaveTextContent('img.shields.io');
    expect(link.querySelector('button')).toBeNull();
    expect(imageSources()).toEqual([]);

    await act(async () => {
      fireEvent.click(link);
    });
    expect(onOpenArtifact).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'externalUrl', url: 'https://github.com/lab/tool' })
    );
    expect(loadRemoteImage).not.toHaveBeenCalled();
  });
});

// A path that starts with two separators is a network path on Windows, and the
// file read's own `realpath` and `lstat` would open an SMB connection to that
// host before anyone clicked (review finding, round 1). Markdown escapes a
// backslash with a backslash, so `\\\\host` below is the path `\\host`.
describe('markdown images that name a network host as a file', () => {
  it.each([
    ['scheme-relative', '//tracker.example.net/p.png'],
    ['a UNC path', String.raw`\\\\tracker.example.net\\share\\p.png`],
    ['a file URL with a UNC path', 'file:////tracker.example.net/share/p.png'],
    ['a file URL with encoded backslashes', 'file:///%5C%5Ctracker.example.net%5Cshare%5Cp.png'],
    ['a file URL naming the host', 'file://tracker.example.net/share/p.png'],
  ])('never reads %s and draws no image', async (_form, src) => {
    const { electron, loadRemoteImage } = installElectron();
    render(
      <MarkdownContent content={`![Tracker](${src}) and text`} workingDir="C:\\Users\\ada\\proj" />
    );

    const chip = await screen.findByRole('img', { name: 'Tracker' });
    expect(chip.tagName).toBe('SPAN');
    await act(async () => {
      await Promise.resolve();
    });
    expect(electron.readArtifactFile).not.toHaveBeenCalled();
    expect(loadRemoteImage).not.toHaveBeenCalled();
    expect(screen.queryByRole('button')).toBeNull();
    expect(imageSources()).toEqual([]);
  });
});

describe('a remote image being fetched', () => {
  it('says so to a screen reader while it loads', async () => {
    installElectron({ loadRemoteImage: () => new Promise<LoadResult>(() => undefined) });
    render(<MarkdownContent content={`![Plot](${PLOT})`} />);
    fireEvent.click(await screen.findByRole('button', { name: /load image from/i }));

    expect(await screen.findByRole('status')).toHaveTextContent('Loading image: Plot');
    expect(imageSources()).toEqual([]);
  });

  it('says so without alt text as well', async () => {
    installElectron({ loadRemoteImage: () => new Promise<LoadResult>(() => undefined) });
    render(<MarkdownContent content={`![](${PLOT})`} />);
    fireEvent.click(await screen.findByRole('button', { name: /load image from/i }));

    expect(await screen.findByRole('status')).toHaveTextContent(/^Loading image$/);
  });
});

describe('remote images in a markdown file in the side panel', () => {
  it('waits for a click there too', async () => {
    const { loadRemoteImage } = installElectron();
    render(
      <MarkdownDocument
        text={`# Report\n\n![Figure 2](${PLOT})\n\n![Local](./local.png)`}
        path="/work/report.md"
      />
    );

    expect(
      await screen.findByRole('button', { name: /load image from cdn\.example\.org/i })
    ).toBeVisible();
    await screen.findByRole('img', { name: 'Local' });
    expect(imageSources()).toEqual(['data:image/png;base64,AAAA']);
    expect(loadRemoteImage).not.toHaveBeenCalled();
  });
});
