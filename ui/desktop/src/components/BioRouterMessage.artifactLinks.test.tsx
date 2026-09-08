import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { Message } from '../api';
import type { ArtifactSource } from './artifacts/artifactTypes';
import BioRouterMessage from './BioRouterMessage';

const assistantText = (id: string, text: string): Message => ({
  id,
  role: 'assistant',
  created: 1,
  metadata: { userVisible: true, agentVisible: true },
  content: [{ type: 'text', text }],
});

function messageView(
  message: Message,
  messages: Message[],
  onOpenArtifact: (artifact: ArtifactSource) => void,
  workingDir: string | null = '/work',
  sessionId = 'link-session',
  isStreaming = false
) {
  return (
    <BioRouterMessage
      sessionId={sessionId}
      message={message}
      messages={messages}
      toolCallNotifications={new Map()}
      onRunInTerminal={null}
      workingDir={workingDir ?? undefined}
      onOpenArtifact={onOpenArtifact}
      isStreaming={isStreaming}
    />
  );
}

describe('BioRouterMessage artifact links', () => {
  it('file-link reliability: leaves prose paths inert while this assistant message streams', async () => {
    const onOpenArtifact = vi.fn();
    const message = assistantText('streaming', 'Writing `/tmp/report.js`');
    render(messageView(message, [message], onOpenArtifact, '/work', 'link-session', true));

    expect(await screen.findByText('/tmp/report.js')).toBeVisible();
    expect(screen.queryByRole('button', { name: '/tmp/report.js' })).not.toBeInTheDocument();
    expect(onOpenArtifact).not.toHaveBeenCalled();
  });

  it('opens the generated inline-code file from an assistant message', async () => {
    const onOpenArtifact = vi.fn();
    const message: Message = {
      id: 'weather-site-message',
      role: 'assistant',
      created: 1,
      metadata: { userVisible: true, agentVisible: true },
      content: [
        {
          type: 'text',
          text: `Created a self-contained weather website at:

\`/Users/wgu/Desktop/weather-website/index.html\``,
        },
      ],
    };

    render(
      <BioRouterMessage
        sessionId="weather-site-session"
        message={message}
        messages={[message]}
        toolCallNotifications={new Map()}
        onRunInTerminal={null}
        workingDir="/Users/wgu/Desktop"
        onOpenArtifact={onOpenArtifact}
      />
    );

    fireEvent.click(
      await screen.findByRole('button', {
        name: '/Users/wgu/Desktop/weather-website/index.html',
      })
    );

    expect(onOpenArtifact).toHaveBeenCalledWith({
      kind: 'file',
      title: 'index.html',
      path: '/Users/wgu/Desktop/weather-website/index.html',
    });
  });

  it.each(['report.md', 'results/report.md'])(
    'file-link reliability: distinguishes basename shorthand from qualified relative targets: %s',
    async (shorthand) => {
      const onOpenArtifact = vi.fn();
      const earlier = assistantText('created', 'Created `/work/run/results/report.md`.');
      const current = assistantText('follow-up', `[Open report](${shorthand})`);
      render(messageView(current, [earlier, current], onOpenArtifact));

      fireEvent.click(await screen.findByRole('button', { name: 'Open report' }));
      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'report.md',
        path: shorthand === 'report.md' ? '/work/run/results/report.md' : '/work/results/report.md',
      });
    }
  );

  it('file-link reliability: refuses shorthand shared by two earlier artifact paths', async () => {
    const onOpenArtifact = vi.fn();
    const earlier = assistantText(
      'two-reports',
      'Compare `/work/alpha/report.md` and `/work/beta/report.md`.'
    );
    const current = assistantText('ambiguous', '[Open report](report.md)');
    render(messageView(current, [earlier, current], onOpenArtifact));

    expect(await screen.findByText('Open report')).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Open report' })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Open report' })).not.toBeInTheDocument();
    expect(onOpenArtifact).not.toHaveBeenCalled();
  });

  it('file-link reliability: preserves explicit absolute targets despite basename collisions', async () => {
    const onOpenArtifact = vi.fn();
    const earlier = assistantText('old', 'Created `/work/alpha/report.md`.');
    const current = assistantText('explicit', '[Open report](/work/beta/report.md)');
    render(messageView(current, [earlier, current], onOpenArtifact));

    fireEvent.click(await screen.findByRole('button', { name: 'Open report' }));
    expect(onOpenArtifact).toHaveBeenCalledWith({
      kind: 'file',
      title: 'report.md',
      path: '/work/beta/report.md',
    });
  });

  it.each([false, true])(
    'file-link reliability: uses only successful visible writes as provenance (failed=%s)',
    async (failed) => {
      const onOpenArtifact = vi.fn();
      const request: Message = {
        ...assistantText('write-request', ''),
        content: [
          {
            type: 'toolRequest',
            id: 'write-tool',
            toolCall: {
              status: 'success',
              value: {
                name: 'developer__text_editor',
                arguments: { command: 'write', path: '/work/results/report.md' },
              },
            },
          },
        ],
      };
      const response: Message = {
        id: 'write-result',
        role: 'tool',
        created: 2,
        metadata: { userVisible: false, agentVisible: true },
        content: [
          {
            type: 'toolResponse',
            id: 'write-tool',
            toolResult: {
              status: 'success',
              value: {
                is_error: failed,
                content: [{ type: 'text', text: failed ? 'denied' : 'saved' }],
              },
            },
          },
        ],
      };
      const current = assistantText('after-write', '[Open report](report.md)');
      render(messageView(current, [request, response, current], onOpenArtifact));

      fireEvent.click(await screen.findByRole('button', { name: 'Open report' }));
      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'report.md',
        path: failed ? '/work/report.md' : '/work/results/report.md',
      });
    }
  );

  it.each(['hidden', 'foreign', 'future'])(
    'file-link reliability: excludes %s messages from earlier-artifact provenance',
    async (source) => {
      const onOpenArtifact = vi.fn();
      const other = assistantText('other', 'Created `/elsewhere/private/report.md`.');
      if (source === 'hidden') other.metadata.userVisible = false;
      if (source === 'foreign') {
        other.metadata.provenance = {
          kind: 'agent_injection',
          fromSessionId: 'another-session',
        };
      }
      const current = assistantText('current', '[Open report](report.md)');
      const messages = source === 'future' ? [current, other] : [other, current];
      render(messageView(current, messages, onOpenArtifact));

      fireEvent.click(await screen.findByRole('button', { name: 'Open report' }));
      expect(onOpenArtifact).toHaveBeenCalledWith({
        kind: 'file',
        title: 'report.md',
        path: '/work/report.md',
      });
    }
  );

  it('file-link reliability: does not reuse artifact provenance after switching sessions', async () => {
    const onOpenArtifact = vi.fn();
    const earlier = assistantText('a-created', 'Created `/work/a/results/report.md`.');
    const current = assistantText('a-follow-up', '[Open report](report.md)');
    const { rerender } = render(
      messageView(current, [earlier, current], onOpenArtifact, '/work/a', 'session-a')
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Open report' }));
    expect(onOpenArtifact).toHaveBeenLastCalledWith({
      kind: 'file',
      title: 'report.md',
      path: '/work/a/results/report.md',
    });

    const next = assistantText('b-follow-up', '[Open report](report.md)');
    rerender(messageView(next, [next], onOpenArtifact, '/work/b', 'session-b'));
    fireEvent.click(await screen.findByRole('button', { name: 'Open report' }));
    expect(onOpenArtifact).toHaveBeenLastCalledWith({
      kind: 'file',
      title: 'report.md',
      path: '/work/b/report.md',
    });
  });

  it('file-link reliability: waits for the actual session working directory', async () => {
    const onOpenArtifact = vi.fn();
    const current = assistantText('loading', '[Open report](results/report.md)');
    const { rerender } = render(
      messageView(current, [current], onOpenArtifact, null, 'loading-session')
    );

    expect(await screen.findByText('Open report')).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Open report' })).not.toBeInTheDocument();
    expect(onOpenArtifact).not.toHaveBeenCalled();

    rerender(messageView(current, [current], onOpenArtifact, '/actual/session', 'loading-session'));
    fireEvent.click(await screen.findByRole('button', { name: 'Open report' }));
    expect(onOpenArtifact).toHaveBeenCalledWith({
      kind: 'file',
      title: 'report.md',
      path: '/actual/session/results/report.md',
    });
  });
});
