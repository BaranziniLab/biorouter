import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from '../../contexts/ThemeContext';
import ArtifactViewer from './ArtifactViewer';
import type { ArtifactSource } from './artifactTypes';
import { PREVIEW_COPY } from './copy';
import { TOOLTIP_DELAY_MS } from '../ui/Tooltip';

/**
 * The header's tooltips follow the panel's open state (V2-R8-4). The behaviour itself, against
 * real pointer input, is `previewTooltip.browser.test.tsx`; this file pins the wiring that
 * test cannot see: that `ArtifactViewer` hands its own `isOpen` to every control it draws, the
 * shared `PreviewIconButton`s and the More actions menu trigger alike.
 */

// A file, because only a file keeps the More actions menu (a figure stands its one action in
// the header as an icon): the menu trigger has a tooltip of its own, not a PreviewIconButton's.
const ARTIFACT: ArtifactSource = { kind: 'file', title: 'notes.txt', path: '/tmp/notes.txt' };

function installElectronMock() {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      readArtifactFile: vi.fn(async () => ({
        kind: 'text',
        title: 'notes.txt',
        path: '/tmp/notes.txt',
        mimeType: 'text/plain',
        text: 'a line',
        size: 6,
        found: true,
      })),
      openArtifactInBrowser: vi.fn(),
      openDirectoryInExplorer: vi.fn(),
      openExternal: vi.fn(),
    },
  });
}

const viewer = (isOpen: boolean) => (
  <ThemeProvider>
    <ArtifactViewer
      artifact={ARTIFACT}
      isOpen={isOpen}
      onClose={vi.fn()}
      onOpenArtifact={vi.fn()}
      sessionId="chat-1"
    />
  </ThemeProvider>
);

const HEADER_CONTROLS = [
  PREVIEW_COPY.openExternally,
  PREVIEW_COPY.moreActions,
  PREVIEW_COPY.closePanel,
];

const pastTheDelay = () => new Promise((done) => setTimeout(done, TOOLTIP_DELAY_MS + 150));

describe('ArtifactViewer tooltips follow the panel’s open state', () => {
  beforeEach(installElectronMock);
  afterEach(() => {
    Reflect.deleteProperty(window, 'electron');
  });

  it.each(HEADER_CONTROLS)('names %s after the delay while the panel is open', async (name) => {
    const user = userEvent.setup();
    render(viewer(true));
    await user.hover(await screen.findByRole('button', { name }));
    expect(await screen.findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent(name);
  });

  it.each(HEADER_CONTROLS)('opens no tooltip on %s while the panel is leaving', async (name) => {
    const user = userEvent.setup();
    render(viewer(false));
    await user.hover(await screen.findByRole('button', { name }));
    await pastTheDelay();
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
  });

  it.each(HEADER_CONTROLS)(
    'closes the tooltip on %s when the panel starts to leave',
    async (name) => {
      const user = userEvent.setup();
      const { rerender } = render(viewer(true));
      await user.hover(await screen.findByRole('button', { name }));
      await screen.findByRole('tooltip', {}, { timeout: 2000 });
      rerender(viewer(false));
      expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
    }
  );
});
