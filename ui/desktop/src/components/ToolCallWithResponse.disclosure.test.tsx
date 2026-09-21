import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ToolCallWithResponse from './ToolCallWithResponse';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../types/message';

function show(
  args: Record<string, unknown>,
  text?: string,
  name = 'developer__exec_command',
  running = false
) {
  const toolRequest = {
    type: 'toolRequest',
    id: 'one',
    toolCall: { status: 'success', value: { name, arguments: args } },
  } as ToolRequestMessageContent;
  const toolResponse =
    text === undefined
      ? undefined
      : ({
          type: 'toolResponse',
          id: 'one',
          toolResult: { status: 'success', value: { content: [{ type: 'text', text }] } },
        } as ToolResponseMessageContent);
  const onOpenArtifact = vi.fn();
  const view = render(
    <ToolCallWithResponse
      toolRequest={toolRequest}
      toolResponse={toolResponse}
      turnActive={running}
      isCancelledMessage={false}
      onOpenArtifact={onOpenArtifact}
    />
  );
  const toggle = view.container.querySelector('button.br-tool-disclosure')!;
  fireEvent.click(toggle);
  return { ...view, toggle, onOpenArtifact };
}

describe('one tool disclosure', () => {
  it('reveals complete short input and output without nested toggles or colored status dots', () => {
    const { container, toggle } = show({ cmd: 'printf hello' }, 'hello');
    expect(screen.getByText('printf hello')).toBeVisible();
    expect(screen.getByText('hello')).toBeVisible();
    expect(screen.queryByText('Show more')).not.toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(document.getElementById(toggle.getAttribute('aria-controls')!)).toBeVisible();
    expect(toggle).not.toHaveClass('tint-interactive', 'biorouter-focus-surface');
    expect(
      container.querySelector(
        '[class*="bg-background-success"], [class*="bg-background-danger"], [class*="bg-background-warning"]'
      )
    ).toBeNull();
    expect(screen.getByLabelText('Tool status: success')).toHaveClass('sr-only');
  });

  it('shows six lines of long literal input then reveals the exact full value', () => {
    const cmd = Array.from({ length: 9 }, (_, i) => `command line ${i + 1}`).join('\n');
    const { container } = show({ cmd });
    expect(container.querySelector('pre')!.textContent).toBe(
      cmd.split('\n').slice(0, 6).join('\n')
    );
    fireEvent.click(screen.getByRole('button', { name: 'Show more' }));
    expect(container.querySelector('pre')!.textContent).toBe(cmd);
    fireEvent.click(screen.getByRole('button', { name: 'Show less' }));
    expect(container.querySelector('pre')!.textContent).not.toContain('command line 9');
  });

  it('keeps a truncated markdown URL inert until its full destination is available', () => {
    const url = `https://example.com/${'a'.repeat(700)}`;
    const { container, onOpenArtifact } = show({}, url);
    expect(container.querySelector('a')).toBeNull();
    expect(screen.queryByRole('button', { name: url })).not.toBeInTheDocument();
    expect(container.querySelector('pre')!.textContent!.length).toBe(600);
    fireEvent.click(screen.getByRole('button', { name: 'Show more' }));
    fireEvent.click(screen.getByRole('button', { name: url }));
    expect(onOpenArtifact).toHaveBeenCalledWith(expect.objectContaining({ url }));
  });

  it('reveals short generated code and additional execute-code arguments together', () => {
    const { container } = show(
      {
        tool_graph: [{ tool: 'developer/shell', description: 'Inspect files' }],
        code: 'const result = 42;',
        timeout: 17,
      },
      'done',
      'multi_tool_use__execute_code'
    );
    expect(container.textContent).toContain('const result = 42;');
    expect(screen.getByText('timeout')).toBeVisible();
    expect(screen.getByText('17')).toBeVisible();
    expect(screen.getByText('done')).toBeVisible();
    expect(screen.queryByText('Show more')).not.toBeInTheDocument();
  });

  it('pulses running text without a status-dot overlay', () => {
    const { container } = show({}, undefined, 'developer__exec_command', true);
    expect(container.querySelector('.br-tool-running')).not.toBeNull();
    expect(screen.getByLabelText('Tool status: loading')).toHaveClass('sr-only');
    expect(container.querySelector('.animate-pulse')).toBeNull();
  });
});
