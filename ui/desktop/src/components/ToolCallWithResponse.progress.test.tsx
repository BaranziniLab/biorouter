/**
 * What a tool row says while the work is under way (spec 3.6): the verb carries
 * the state and there is no suffix by default, but a real message from the tool
 * (a progress note, the shell's heartbeat, the latest log line) may follow the
 * label, and the opened well shows the shared thin `Progress` with the message
 * in `text-supporting`.
 */
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import ToolCallWithResponse from './ToolCallWithResponse';
import type { NotificationEvent, ToolRequestMessageContent } from '../types/message';

const request: ToolRequestMessageContent = {
  type: 'toolRequest',
  id: 'tool-progress',
  toolCall: {
    status: 'success',
    value: { name: 'developer__exec_command', arguments: { cmd: 'build-index' } },
  },
};

const notification = (method: string, params: unknown): NotificationEvent =>
  ({
    type: 'Notification',
    request_id: 'tool-progress',
    message: { method, params },
  }) as unknown as NotificationEvent;

const progress = (value: number, total: number | undefined, message: string) =>
  notification('notifications/progress', { progress: value, total, message, progressToken: 'p1' });

const log = (output: string) =>
  notification('notifications/message', { data: { stream: 'stdout', output } });

function renderRunning(notifications?: NotificationEvent[]) {
  return render(
    <ToolCallWithResponse
      isCancelledMessage={false}
      toolRequest={request}
      notifications={notifications}
      turnActive
      onOpenArtifact={vi.fn()}
    />
  );
}

const trigger = () => screen.getByRole('button', { name: /Running build-index/ });

describe('a running tool row', () => {
  it('has no suffix by default: the verb says it is running', () => {
    const { container } = renderRunning();
    expect(screen.getByText('Running build-index')).toBeInTheDocument();
    expect(container.querySelector('.br-transcript-row-meta')).toBeNull();
    expect(container.querySelector('.br-transcript-row')).toHaveAttribute('data-running');
  });

  it('follows the label with the tool’s own progress message, and no other suffix', () => {
    const { container } = renderRunning([progress(3, 10, 'Indexing shard 3')]);
    expect(container.querySelector('.br-transcript-row-meta')).toHaveTextContent(
      '· Indexing shard 3'
    );
    expect(container).not.toHaveTextContent(/Working through/i);
    expect(container).not.toHaveTextContent(/result/i);
  });

  it('opens onto the shared thin Progress with the message above it in supporting type', () => {
    renderRunning([progress(3, 10, 'Indexing shard 3')]);
    fireEvent.click(trigger());

    const bar = screen.getByRole('progressbar', { name: 'Indexing shard 3' });
    expect(bar).toHaveAttribute('data-size', 'thin');
    expect(bar).toHaveAttribute('aria-valuenow', '3');
    expect(bar).toHaveAttribute('aria-valuemax', '10');
    // The label of the section, then the message in `text-supporting` muted.
    expect(screen.getByText('Progress')).toHaveClass('br-transcript-row-section-label');
    const message = screen.getAllByText('Indexing shard 3').find((node) => node.tagName === 'DIV');
    expect(message).toHaveClass('text-supporting');
  });

  it('sweeps an indeterminate bar when the tool reports no total', () => {
    renderRunning([progress(3, undefined, 'Working')]);
    fireEvent.click(trigger());
    const bar = screen.getByRole('progressbar');
    expect(bar).toHaveAttribute('data-size', 'thin');
    expect(bar).not.toHaveAttribute('aria-valuenow');
  });

  it('shows the latest log line after the label and live logs in the well', () => {
    const { container } = renderRunning([log('first'), log('compiling crate 4 of 9')]);
    expect(container.querySelector('.br-transcript-row-meta')).toHaveTextContent(
      '· [stdout] compiling crate 4 of 9'
    );
    fireEvent.click(trigger());
    expect(screen.getByText('Live logs')).toBeInTheDocument();
    expect(screen.getByTestId('tool-logs-box')).toHaveTextContent('[stdout] first');
  });
});

describe('a finished tool row', () => {
  it('drops the live detail and calls its logs Logs', () => {
    const { container } = render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={request}
        toolResponse={{
          type: 'toolResponse',
          id: 'tool-progress',
          toolResult: {
            status: 'success',
            value: { is_error: false, content: [{ type: 'text', text: 'ok' }] },
          },
        }}
        notifications={[progress(10, 10, 'Indexing shard 10'), log('done')]}
        turnActive
        onOpenArtifact={vi.fn()}
      />
    );
    expect(screen.getByText('Ran build-index')).toBeInTheDocument();
    expect(container.querySelector('.br-transcript-row-meta')).toBeNull();
    expect(container.querySelector('.br-transcript-row')).not.toHaveAttribute('data-running');
    fireEvent.click(screen.getByRole('button', { name: /Ran build-index/ }));
    expect(screen.getByText('Logs')).toBeInTheDocument();
    expect(screen.queryByText('Live logs')).toBeNull();
  });
});
