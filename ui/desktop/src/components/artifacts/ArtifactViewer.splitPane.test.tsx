import { render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import ArtifactViewer from './ArtifactViewer';

/**
 * Two preview panels on one page must each name their OWN preview.
 *
 * A panel's tabs name the preview they control with `aria-controls`, which the
 * document resolves by id — and the preview's id was the literal
 * `artifact-preview-content`, so a second panel's tabs pointed at the FIRST
 * panel's preview. It is the same collision that made the composer's Send submit
 * the wrong pane in a split (`ChatInput.splitPaneSend.test.tsx`).
 *
 * ⚠ Not reachable in today's app, and this test does not claim it is: every chat
 * pane mounts a BaseChat, but ChatGroupsShell renders only the ACTIVE group's
 * panel (`artifactPanelEnabled={isActiveGroup}`). Measured in a real split:
 * opening the right pane's preview closed the left one, and the window held one
 * `aria-controls` tab whose id had one holder. What this pins is that the
 * component no longer depends on that gate.
 */

function installElectronMock() {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      prepareArtifactHtml: vi.fn(async ({ html }: { html: string }) => ({ html })),
      readArtifactFile: vi.fn(async ({ path }: { path: string }) => ({
        kind: 'text',
        title: path.split('/').pop(),
        path,
        mimeType: 'text/plain',
        text: `contents of ${path}`,
        size: 20,
        found: true,
      })),
      broadcastThemeChange: vi.fn(),
      on: vi.fn().mockReturnValue(() => undefined),
    },
  });
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: vi.fn().mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  });
}

describe('two preview panels on one page', () => {
  it("points each panel's tabs at that panel's own preview", async () => {
    installElectronMock();
    render(
      <ThemeProvider>
        <section data-testid="pane-left">
          <ArtifactViewer
            artifact={{ kind: 'file', title: 'left.txt', path: '/tmp/left.txt' }}
            onClose={vi.fn()}
            onOpenArtifact={vi.fn()}
            sessionId="chat-left"
          />
        </section>
        <section data-testid="pane-right">
          <ArtifactViewer
            artifact={{ kind: 'file', title: 'right.txt', path: '/tmp/right.txt' }}
            onClose={vi.fn()}
            onOpenArtifact={vi.fn()}
            sessionId="chat-right"
          />
        </section>
      </ThemeProvider>
    );

    for (const side of ['left', 'right']) {
      const pane = screen.getByTestId(`pane-${side}`);
      await waitFor(() => expect(within(pane).getAllByRole('tab').length).toBeGreaterThan(0));
      for (const tab of within(pane).getAllByRole('tab')) {
        const controlled = tab.getAttribute('aria-controls');
        expect(controlled).toBeTruthy();
        const target = document.getElementById(controlled!);
        expect(target, `${side} tab's aria-controls target`).not.toBeNull();
        expect(pane.contains(target), `${side} tab controls a preview in its own pane`).toBe(true);
      }
    }
  });
});
