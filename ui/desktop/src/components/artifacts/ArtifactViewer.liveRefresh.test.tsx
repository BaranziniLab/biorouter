import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ComponentProps, ComponentType } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import ArtifactViewer from './ArtifactViewer';
import WebPagePreview from './WebPagePreview';
import type { ArtifactFilePreview, ArtifactSource } from './artifactTypes';

const LiveViewer = ArtifactViewer as ComponentType<
  ComponentProps<typeof ArtifactViewer> & { refreshRevision: number }
>;
const LiveWebPage = WebPagePreview as ComponentType<
  ComponentProps<typeof WebPagePreview> & { refreshRevision: number }
>;

function install() {
  const readArtifactFile = vi.fn(
    async (): Promise<ArtifactFilePreview> => ({
      kind: 'text',
      title: 'result.txt',
      path: '/tmp/result.txt',
      mimeType: 'text/plain',
      text: 'left',
      size: 4,
      revision: 'first-bytes',
      found: true,
    })
  );
  const browser = {
    create: vi.fn(async (_id: string, url: string) => ({
      url,
      title: 'Synthetic app',
      managedApp: true,
      sourceRevision: '1:1',
      canGoBack: false,
      canGoForward: false,
      isLoading: false,
      error: null,
    })),
    onState: vi.fn(() => () => {}),
    setBounds: vi.fn(async () => {}),
    setVisible: vi.fn(async () => {}),
    control: vi.fn(async () => true),
    destroy: vi.fn(async () => {}),
  };
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      readArtifactFile,
      prepareArtifactHtml: vi.fn(async ({ html }: { html: string }) => ({ html })),
      embeddedBrowser: browser,
      broadcastThemeChange: vi.fn(),
      on: vi.fn(() => () => {}),
    },
  });
  return { readArtifactFile, browser };
}

describe('completed coding changes refresh the existing artifact', () => {
  it('re-reads a same-path, same-size file on a completed-write revision', async () => {
    const mocks = install();
    const artifact: ArtifactSource = { kind: 'file', title: 'result.txt', path: '/tmp/result.txt' };
    const close = vi.fn();
    const open = vi.fn();
    const view = (revision: number) => (
      <ThemeProvider>
        <LiveViewer
          artifact={artifact}
          onClose={close}
          onOpenArtifact={open}
          refreshRevision={revision}
        />
      </ThemeProvider>
    );
    const { rerender } = render(view(0));
    expect(await screen.findByText('left')).toBeInTheDocument();
    mocks.readArtifactFile.mockResolvedValueOnce({
      kind: 'text',
      title: 'result.txt',
      path: '/tmp/result.txt',
      mimeType: 'text/plain',
      text: 'rite',
      size: 4,
      revision: 'second-bytes',
      found: true,
    });
    rerender(view(1));
    await waitFor(() => expect(mocks.readArtifactFile).toHaveBeenCalledTimes(2), { timeout: 1000 });
    expect(await screen.findByText('rite')).toBeInTheDocument();
    expect(screen.queryByText('left')).not.toBeInTheDocument();
  });

  it('reloads an already-open managed app after a completed build, without recreating its view', async () => {
    const { browser } = install();
    const openExternal = vi.fn();
    const view = (revision: number) => (
      <LiveWebPage
        url="http://127.0.0.1:64005/apps/qa/"
        onOpenExternal={openExternal}
        refreshRevision={revision}
      />
    );
    const { rerender } = render(view(0));
    await waitFor(() => expect(browser.create).toHaveBeenCalledOnce());
    rerender(view(1));
    await waitFor(
      () => expect(browser.control).toHaveBeenCalledWith(expect.any(String), 'reload-if-idle'),
      { timeout: 1000 }
    );
    expect(browser.create).toHaveBeenCalledOnce();
  });

  it('keeps the latest good file during refresh and ignores an older read finishing last', async () => {
    const { readArtifactFile } = install();
    const artifact: ArtifactSource = {
      kind: 'file',
      title: 'result.txt',
      path: '/tmp/result.txt',
      line: 2,
    };
    const open = vi.fn();
    const close = vi.fn();
    const view = (revision: number, sessionId = 'a') => (
      <ThemeProvider>
        <LiveViewer
          artifact={artifact}
          onClose={close}
          onOpenArtifact={open}
          refreshRevision={revision}
          sessionId={sessionId}
        />
      </ThemeProvider>
    );
    const { rerender } = render(view(0));
    expect(await screen.findByText('left')).toBeInTheDocument();
    let finish!: (value: ArtifactFilePreview) => void;
    readArtifactFile.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    rerender(view(1));
    await waitFor(() => expect(readArtifactFile).toHaveBeenCalledTimes(2));
    expect(screen.getByText('left')).toBeInTheDocument();
    expect(screen.queryByText('Loading')).not.toBeInTheDocument();
    readArtifactFile.mockResolvedValueOnce({
      kind: 'text',
      path: artifact.path,
      title: artifact.title,
      text: 'newest',
      revision: 'newest',
      size: 6,
      mimeType: 'text/plain',
      found: true,
    });
    rerender(view(2));
    expect(await screen.findByText('newest')).toBeInTheDocument();
    await act(async () =>
      finish({
        kind: 'text',
        path: artifact.path,
        title: artifact.title,
        text: 'older',
        revision: 'older',
        size: 5,
        mimeType: 'text/plain',
        found: true,
      })
    );
    expect(screen.queryByText('older')).not.toBeInTheDocument();
    for (const call of readArtifactFile.mock.calls) expect(call).toEqual(['/tmp/result.txt']);
  });

  it('retains a good preview after a failed read and offers an explicit retry', async () => {
    const { readArtifactFile } = install();
    const artifact: ArtifactSource = { kind: 'file', title: 'result.txt', path: '/tmp/result.txt' };
    const open = vi.fn();
    const close = vi.fn();
    const view = (revision: number) => (
      <ThemeProvider>
        <LiveViewer
          artifact={artifact}
          onClose={close}
          onOpenArtifact={open}
          refreshRevision={revision}
        />
      </ThemeProvider>
    );
    const { rerender } = render(view(0));
    await screen.findByText('left');
    readArtifactFile.mockResolvedValueOnce({
      kind: 'error',
      path: artifact.path,
      title: artifact.title,
      error: 'Not available',
      found: false,
    });
    rerender(view(1));
    const retry = await screen.findByRole('button', { name: 'Retry update' });
    expect(screen.getByText('left')).toBeInTheDocument();
    fireEvent.click(retry);
    await waitFor(() => expect(readArtifactFile).toHaveBeenCalledTimes(3));
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Retry update' })).not.toBeInTheDocument()
    );
  });

  it.each(['session', 'file'])(
    'does not publish a late background read after a %s switch',
    async (change) => {
      const { readArtifactFile } = install();
      const first: ArtifactSource = { kind: 'file', title: 'result.txt', path: '/tmp/result.txt' };
      const second: ArtifactSource = { kind: 'file', title: 'other.txt', path: '/tmp/other.txt' };
      const open = vi.fn();
      const close = vi.fn();
      const view = (revision: number, switched = false) => (
        <ThemeProvider>
          <LiveViewer
            artifact={switched && change === 'file' ? second : first}
            sessionId={switched && change === 'session' ? 'b' : 'a'}
            onClose={close}
            onOpenArtifact={open}
            refreshRevision={revision}
          />
        </ThemeProvider>
      );
      const { rerender } = render(view(0));
      await screen.findByText('left');
      let finish!: (value: ArtifactFilePreview) => void;
      readArtifactFile.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          })
      );
      rerender(view(1));
      await waitFor(() => expect(readArtifactFile).toHaveBeenCalledTimes(2));
      readArtifactFile.mockResolvedValue({
        kind: 'text',
        title: 'current',
        path: change === 'file' ? second.path : first.path,
        text: 'current-session-file',
        revision: 'current',
        size: 20,
        mimeType: 'text/plain',
        found: true,
      });
      rerender(view(1, true));
      await screen.findByText('current-session-file');
      await act(async () =>
        finish({
          kind: 'text',
          title: first.title,
          path: first.path,
          text: 'stale-old-session',
          revision: 'stale',
          size: 17,
          mimeType: 'text/plain',
          found: true,
        })
      );
      expect(screen.queryByText('stale-old-session')).not.toBeInTheDocument();
      expect(screen.getByText('current-session-file')).toBeInTheDocument();
    }
  );

  it('defers an in-flight read if resizing begins before it returns', async () => {
    const { readArtifactFile } = install();
    const artifact: ArtifactSource = { kind: 'file', title: 'result.txt', path: '/tmp/result.txt' };
    const open = vi.fn();
    const close = vi.fn();
    const view = (revision: number, resizing = false) => (
      <ThemeProvider>
        <LiveViewer
          artifact={artifact}
          onClose={close}
          onOpenArtifact={open}
          refreshRevision={revision}
          isResizing={resizing}
        />
      </ThemeProvider>
    );
    const { rerender } = render(view(0));
    await screen.findByText('left');
    let finish!: (value: ArtifactFilePreview) => void;
    readArtifactFile.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    rerender(view(1));
    await waitFor(() => expect(readArtifactFile).toHaveBeenCalledTimes(2));
    rerender(view(1, true));
    await act(async () =>
      finish({
        kind: 'text',
        title: artifact.title,
        path: artifact.path,
        text: 'interrupted',
        revision: 'interrupted',
        size: 11,
        mimeType: 'text/plain',
        found: true,
      })
    );
    expect(screen.getByText('left')).toBeInTheDocument();
    expect(screen.queryByText('interrupted')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Update ready' })).toBeDisabled();
    rerender(view(1));
    await waitFor(() => expect(readArtifactFile).toHaveBeenCalledTimes(3));
  });

  it('defers a file update during resize and applies it when the interaction ends', async () => {
    const { readArtifactFile } = install();
    const artifact: ArtifactSource = { kind: 'file', title: 'result.txt', path: '/tmp/result.txt' };
    const open = vi.fn();
    const close = vi.fn();
    const view = (revision: number, resizing: boolean) => (
      <ThemeProvider>
        <LiveViewer
          artifact={artifact}
          onClose={close}
          onOpenArtifact={open}
          refreshRevision={revision}
          isResizing={resizing}
        />
      </ThemeProvider>
    );
    const { rerender } = render(view(0, false));
    await screen.findByText('left');
    rerender(view(1, true));
    expect(await screen.findByRole('button', { name: 'Update ready' })).toBeDisabled();
    expect(readArtifactFile).toHaveBeenCalledTimes(1);
    rerender(view(1, false));
    await waitFor(() => expect(readArtifactFile).toHaveBeenCalledTimes(2));
  });

  it('never reloads a remote page or trusts a localhost URL alone', async () => {
    const { browser } = install();
    browser.create.mockImplementation(async (_id, url) => ({
      url,
      title: 'Unmanaged',
      managedApp: false,
      sourceRevision: '1:1',
      canGoBack: false,
      canGoForward: false,
      isLoading: false,
      error: null,
    }));
    const openExternal = vi.fn();
    const view = (revision: number) => (
      <LiveWebPage
        url="http://127.0.0.1:64005/apps/qa/"
        onOpenExternal={openExternal}
        refreshRevision={revision}
      />
    );
    const { rerender } = render(view(0));
    await waitFor(() => expect(browser.create).toHaveBeenCalledOnce());
    rerender(view(1));
    await act(async () => {});
    expect(browser.control).not.toHaveBeenCalled();
  });

  it('preserves a dirty HTML form after blur; only its actual frame can defer an update', async () => {
    const { readArtifactFile } = install();
    const artifact: ArtifactSource = { kind: 'file', title: 'form.html', path: '/tmp/form.html' };
    const first: ArtifactFilePreview = {
      kind: 'html',
      title: artifact.title,
      path: artifact.path,
      text: '<!doctype html><html><head></head><body><input></body></html>',
      revision: 'first-form',
      size: 70,
      mimeType: 'text/html',
      found: true,
    };
    readArtifactFile.mockResolvedValue(first);
    const open = vi.fn();
    const close = vi.fn();
    const view = (revision: number) => (
      <ThemeProvider>
        <LiveViewer
          artifact={artifact}
          onClose={close}
          onOpenArtifact={open}
          refreshRevision={revision}
        />
      </ThemeProvider>
    );
    const { rerender } = render(view(0));
    const frame = (await screen.findByLabelText('form.html')) as HTMLIFrameElement;
    expect(frame.srcdoc).toContain('biorouter.preview.activity.v1');
    act(() =>
      window.dispatchEvent(
        new MessageEvent('message', { data: { type: 'biorouter-artifact-dirty' }, source: window })
      )
    );
    rerender(view(1));
    await waitFor(() => expect(readArtifactFile).toHaveBeenCalledTimes(2));
    act(() =>
      window.dispatchEvent(
        new MessageEvent('message', {
          data: { type: 'biorouter-artifact-dirty' },
          source: frame.contentWindow,
        })
      )
    );
    rerender(view(2));
    const update = await screen.findByRole('button', { name: 'Update ready' });
    expect(readArtifactFile).toHaveBeenCalledTimes(2);
    readArtifactFile.mockResolvedValueOnce({
      ...first,
      text: '<html><body>Updated form</body></html>',
      revision: 'next-form',
    });
    expect(frame.isConnected).toBe(true);
    fireEvent.click(update);
    await waitFor(() =>
      expect((screen.getByLabelText('form.html') as HTMLIFrameElement).srcdoc).toContain(
        'Updated form'
      )
    );
    expect(frame.isConnected).toBe(false);
    expect(readArtifactFile).toHaveBeenCalledTimes(3);
  });

  it('defers while the address is focused, then offers an update when the native page is editing', async () => {
    const { browser } = install();
    browser.control.mockResolvedValueOnce(false);
    const openExternal = vi.fn();
    const view = (revision: number) => (
      <LiveWebPage
        url="http://127.0.0.1:64005/apps/qa/"
        onOpenExternal={openExternal}
        refreshRevision={revision}
      />
    );
    const { rerender } = render(view(0));
    await waitFor(() => expect(browser.create).toHaveBeenCalledOnce());
    const address = screen.getByRole('textbox', { name: 'Address' });
    fireEvent.focus(address);
    rerender(view(1));
    expect(browser.control).not.toHaveBeenCalled();
    fireEvent.blur(address);
    fireEvent.click(await screen.findByRole('button', { name: 'Update ready' }));
    expect(browser.control).toHaveBeenLastCalledWith(expect.any(String), 'reload');
    expect(browser.create).toHaveBeenCalledOnce();
  });

  it('does not reload a suspended native view; refresh resumes without destroying storage', async () => {
    const { browser } = install();
    const openExternal = vi.fn();
    const view = (revision: number, suspended: boolean) => (
      <LiveWebPage
        url="http://127.0.0.1:64005/apps/qa/"
        onOpenExternal={openExternal}
        refreshRevision={revision}
        isSuspended={suspended}
      />
    );
    const { rerender } = render(view(0, false));
    await waitFor(() => expect(browser.create).toHaveBeenCalledOnce());
    rerender(view(1, true));
    expect(browser.control).not.toHaveBeenCalled();
    rerender(view(1, false));
    await waitFor(() =>
      expect(browser.control).toHaveBeenCalledWith(expect.any(String), 'reload-if-idle')
    );
    expect(browser.destroy).not.toHaveBeenCalled();
  });
});
