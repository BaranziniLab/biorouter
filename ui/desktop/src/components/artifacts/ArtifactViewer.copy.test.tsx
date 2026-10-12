import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import ArtifactViewer from './ArtifactViewer';
import type { ArtifactFilePreview } from './artifactTypes';
import { PREVIEW_COPY } from './copy';

const SCRIPT = 'library(ggplot2)\nggsave("volcano.png")\n';

function installElectronMock() {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      readArtifactFile: vi.fn(
        async (): Promise<ArtifactFilePreview> => ({
          kind: 'text',
          title: 'analysis.R',
          path: '/work/analysis.R',
          mimeType: 'text/x-r',
          text: SCRIPT,
          size: SCRIPT.length,
          found: true,
        })
      ),
      prepareArtifactHtml: vi.fn(async ({ html }: { html: string }) => ({ html })),
      openExternal: vi.fn(),
    },
  });
}

const installExecCommand = (impl: (command: string) => boolean) => {
  const execCommand = vi.fn(impl);
  Object.defineProperty(document, 'execCommand', { value: execCommand, configurable: true });
  return execCommand;
};

async function renderScript() {
  render(
    <ThemeProvider>
      <ArtifactViewer
        artifact={{ kind: 'file', title: 'analysis.R', path: '/work/analysis.R' }}
        onClose={vi.fn()}
        onOpenArtifact={vi.fn()}
      />
    </ThemeProvider>
  );
  await waitFor(() => expect(screen.getByText('2 lines')).toBeInTheDocument());
  return screen.getByRole('button', { name: PREVIEW_COPY.copy });
}

/** The outcome is announced beside the icon-only button (spec 3.8), never only drawn. */
const status = () => screen.getByTestId('artifact-copy-status');

/**
 * The preview bar's Copy. A refused write was caught by an empty `catch` that "left the
 * label alone", so in 1.90.4–1.91.2 (whose permission handler refused every write) the button
 * went on saying "Copy" and copied nothing. It now goes through the shared path in
 * `utils/clipboard.ts` and says when that path failed: the glyph turns to a check or a warning,
 * the failure sentence is the tooltip, and the outcome is announced in a status region.
 */
describe('ArtifactViewer — the status strip Copy', () => {
  let writeText: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    installElectronMock();
    writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      writable: true,
      value: { writeText },
    });
  });

  afterEach(() => {
    Reflect.deleteProperty(document, 'execCommand');
  });

  it('copies the whole file and says so', async () => {
    const button = await renderScript();
    await act(async () => {
      fireEvent.click(button);
    });
    expect(writeText).toHaveBeenCalledExactlyOnceWith(SCRIPT);
    await waitFor(() => expect(status()).toHaveTextContent(PREVIEW_COPY.copied));
    expect(button).toHaveAttribute('data-outcome', 'copied');
  });

  it('copies through the document when the clipboard refuses, from inside the bar', async () => {
    writeText.mockRejectedValue(new Error('Write permission denied'));
    let inStrip = false;
    const execCommand = installExecCommand((command) => {
      const area = document.activeElement as HTMLTextAreaElement;
      inStrip =
        area.tagName === 'TEXTAREA' &&
        area.value === SCRIPT &&
        screen.getByTestId('artifact-status-strip').contains(area);
      return command === 'copy';
    });
    const button = await renderScript();

    await act(async () => {
      fireEvent.click(button);
    });

    await waitFor(() => expect(status()).toHaveTextContent(PREVIEW_COPY.copied));
    expect(writeText).toHaveBeenCalledTimes(2);
    expect(execCommand).toHaveBeenCalledWith('copy');
    expect(inStrip).toBe(true);
  });

  it('says "Copy failed" when every path refuses, instead of leaving the label alone', async () => {
    writeText.mockRejectedValue(new Error('Write permission denied'));
    installExecCommand(() => false);
    const button = await renderScript();

    await act(async () => {
      fireEvent.click(button);
    });

    await waitFor(() => expect(status()).toHaveTextContent(PREVIEW_COPY.copyFailedShort));
    expect(button).toHaveAttribute('data-outcome', 'failed');
  });
});
