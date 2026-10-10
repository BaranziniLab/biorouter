import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import MCPUIResourceRenderer from './MCPUIResourceRenderer';
import { ThemeProvider } from '../contexts/ThemeContext';
import type { EmbeddedResource } from '../api';
import type { ArtifactSource } from './artifacts/artifactTypes';

// MCPUIResourceRenderer no longer imports '@mcp-ui/client' at all — the panel is
// the only surface that renders a ui:// document. The mock stays anyway: it is
// what makes `queryByTestId('mcp-ui-frame')` a real assertion. Without it, "no
// iframe rendered" would pass even if the component started rendering one again,
// since a bare UIResourceRenderer emits no identifiable test id.
vi.mock('@mcp-ui/client', () => ({
  UIResourceRenderer: vi.fn(() =>
    React.createElement('iframe', {
      'data-testid': 'mcp-ui-frame',
      title: 'mock mcp ui frame',
    })
  ),
}));

function installElectron() {
  const openExternal = vi.fn().mockResolvedValue(undefined);
  const getSecretKey = vi.fn().mockResolvedValue('secret');

  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      getBiorouterdHostPort: vi.fn().mockResolvedValue('http://localhost:8765'),
      getSecretKey,
      openExternal,
      on: vi.fn().mockReturnValue(() => undefined),
    },
  });

  if (!window.matchMedia) {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn().mockReturnValue({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    });
  }

  return { openExternal, getSecretKey };
}

function renderResource(
  resource: Record<string, unknown>,
  onOpenArtifact: (artifact: ArtifactSource) => void = vi.fn()
) {
  const electron = installElectron();
  const content = { type: 'resource', resource } as EmbeddedResource & { type: 'resource' };

  const result = render(
    <ThemeProvider>
      <MCPUIResourceRenderer content={content} onOpenArtifact={onOpenArtifact} />
    </ThemeProvider>
  );

  return { ...result, ...electron, onOpenArtifact };
}

function renderSubject(uri = 'ui://chart/visualization', onOpenArtifact = vi.fn()) {
  const html = '<!doctype html><html><body><h1>Chart</h1></body></html>';
  const blob = window.btoa(html);
  return {
    ...renderResource({ uri, mimeType: 'text/html', blob }, onOpenArtifact),
    html,
  };
}

describe('MCPUIResourceRenderer', () => {
  it('renders an HTML ui:// resource as a click-to-open card, never as an inline frame', () => {
    // The whole point of this component: a figure has one display surface, the
    // artifact side panel. An inline iframe here would be a second renderer of
    // the same document with its own CSP and its own action channel.
    const { html, onOpenArtifact } = renderSubject();

    expect(screen.queryByTestId('mcp-ui-frame')).not.toBeInTheDocument();

    const card = screen.getByRole('button', {
      name: 'Open Chart Visualization in the artifact viewer',
    });
    fireEvent.click(card);

    expect(onOpenArtifact).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'html', title: 'Chart Visualization', html })
    );
  });

  it('shows a no-preview note, and no card, when the resource yields no artifact source', () => {
    // `artifactSourceFromResource` returns null for an over-long uri, an HTML
    // payload that will not decode, and a uri-list with no usable URL. This is
    // the undecodable-HTML case; all three land in the same branch.
    const { onOpenArtifact } = renderResource({
      uri: 'ui://chart/untrusted',
      mimeType: 'text/html',
      blob: '!!!!not-base64!!!!',
    });

    expect(screen.getByRole('status')).toHaveTextContent('No browser-safe preview');
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(screen.queryByTestId('mcp-ui-frame')).not.toBeInTheDocument();
    expect(onOpenArtifact).not.toHaveBeenCalled();
  });

  it('hands a non-HTML resource to the panel like every other artifact', async () => {
    // A text/plain blob is still an artifact source (kind 'mcpResource'), so it
    // gets a card. There is nowhere else for it to go: the standalone artifact
    // window that used to be the inline card's expand target is gone, and
    // `window.electron` no longer exposes a method to open one.
    const onOpenArtifact = vi.fn();
    renderResource(
      {
        uri: 'ui://chart/untrusted',
        mimeType: 'text/plain',
        blob: btoa('<script>window.bad = true</script>'),
      },
      onOpenArtifact
    );

    expect(screen.queryByTestId('mcp-ui-frame')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /open untrusted chart/i }));

    await waitFor(() =>
      expect(onOpenArtifact).toHaveBeenCalledWith(expect.objectContaining({ kind: 'mcpResource' }))
    );
  });

  it('shows URI-list resources as click-only links instead of loading them in a frame', async () => {
    const onOpenArtifact = vi.fn();
    const { openExternal } = renderResource(
      {
        uri: 'ui://report/published',
        mimeType: 'text/uri-list',
        text: 'https://example.test/report.html',
      },
      onOpenArtifact
    );

    expect(screen.queryByTestId('mcp-ui-frame')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /open report published/i }));
    await waitFor(() =>
      expect(openExternal).toHaveBeenCalledWith('https://example.test/report.html')
    );
    // An external page is not a document we hold, so it goes to the browser and
    // never to the panel.
    expect(onOpenArtifact).not.toHaveBeenCalled();
  });

  it('does not offer a delete control for agent-drafter apps in chat', () => {
    // Deleting an app is destructive and belongs in the Applications tab, not on
    // an in-chat artifact card where a stray click removes files from disk.
    renderSubject('ui://agent-drafter/researcher-impact-dashboard');

    expect(
      screen.queryByRole('button', { name: /delete researcher-impact-dashboard/i })
    ).not.toBeInTheDocument();
    expect(screen.queryByTitle('Delete application')).not.toBeInTheDocument();
    // The open affordance is still there — and it is the only control on the card.
    expect(
      screen.getByRole('button', { name: /^Open .+ Researcher Impact Dashboard/i })
    ).toBeInTheDocument();
  });

  it('opens MCP HTML artifacts in the side viewer, carrying the preferred frame size', () => {
    const html = '<!doctype html><html><body><h1>Chart</h1></body></html>';
    const onOpenArtifact = vi.fn();
    renderResource(
      {
        uri: 'ui://chart/visualization',
        mimeType: 'text/html',
        blob: btoa(html),
        _meta: { 'mcpui.dev/ui-preferred-frame-size': ['900px', '640px'] },
      },
      onOpenArtifact
    );

    fireEvent.click(screen.getByRole('button', { name: /open chart visualization/i }));

    // The card path takes its sizing from the artifact source, which is where
    // the panel reads it too — there is no second copy of that arithmetic here.
    expect(onOpenArtifact).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'html',
        title: 'Chart Visualization',
        html,
        preferredWidth: 900,
        preferredHeight: 640,
      })
    );
  });

  it.each([
    ['ui://chart/visualization', 'figure', 'Figure'],
    ['ui://dashboard/report', 'report', 'Report'],
    ['ui://agent-drafter/researcher-impact-dashboard', 'app', 'App'],
  ])('names %s by its kind (%s), never by a MIME type', (uri, kind, label) => {
    renderSubject(uri);
    const card = screen.getByRole('button', { name: /^Open / });
    expect(card).toHaveAttribute('data-artifact-kind', kind);
    expect(card).toHaveTextContent(label);
    expect(card).not.toHaveTextContent('text/html');
  });

  it('draws the card on the page: a hairline, no shadow, the full column', () => {
    renderSubject();
    const card = screen.getByRole('button', { name: /^Open / });
    expect(card).toHaveClass('br-artifact-card');
    expect(card.className).not.toMatch(/shadow|max-w-xl/);
  });
});
