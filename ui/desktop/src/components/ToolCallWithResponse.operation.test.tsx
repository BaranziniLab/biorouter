import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ToolCallWithResponse from './ToolCallWithResponse';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../types/message';

const operation = {
  version: 1,
  id: 'fixture-job',
  status: 'failed',
  error: {
    code: 'fixture_failed',
    kind: 'tool_failure',
    message: 'Synthetic query failure',
    phase: 'execution',
    retryable: false,
  },
};

function renderResult(meta: Record<string, unknown>, name = 'fixture__status') {
  const request: ToolRequestMessageContent = {
    type: 'toolRequest',
    id: 'fixture',
    toolCall: { status: 'success', value: { name, arguments: {} } },
  };
  const response = {
    type: 'toolResponse',
    id: 'fixture',
    toolResult: {
      status: 'success',
      value: {
        isError: false,
        content: [{ type: 'text', text: 'Synthetic status payload' }],
        _meta: meta,
      },
    },
  } as unknown as ToolResponseMessageContent;
  render(
    <ToolCallWithResponse
      isCancelledMessage={false}
      toolRequest={request}
      toolResponse={response}
      onOpenArtifact={vi.fn()}
    />
  );
}

describe('tool operation outcome', () => {
  it('shows operation failure separately from a successful status lookup', () => {
    renderResult({ 'io.biorouter/operation': operation });
    const row = screen.getByText(/^Operation failed ·/);
    fireEvent.click(row.closest('button') as HTMLButtonElement);
    expect(screen.getByText('Operation failed')).toBeInTheDocument();
    expect(screen.getByText('Status lookup completed successfully.')).toBeInTheDocument();
    expect(
      screen.getByText('Operation fixture-job failed (fixture_failed): Synthetic query failure')
    ).toBeInTheDocument();
    expect(screen.getByText('Synthetic status payload')).toBeInTheDocument();
    expect(screen.queryByText('Tool call failed')).not.toBeInTheDocument();
    expect(
      screen.getByRole('img', { name: 'Operation failed; status lookup completed' })
    ).toBeInTheDocument();
  });

  it.each(['queued', 'running', 'completed', 'cancelled'])(
    'keeps a %s lookup successful',
    (status) => {
      renderResult({ 'io.biorouter/operation': { ...operation, status } });
      expect(screen.queryByText(/^Operation failed/)).not.toBeInTheDocument();
      expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    }
  );

  it('shows a nested operation failure while the inspecting script remains successful', () => {
    renderResult(
      {
        'biorouter/tool-calls': [{ tool: 'fixture__status', args: '{}', status: 'ok', operation }],
      },
      'code_execution__execute_code'
    );
    expect(screen.queryByText(/^Operation failed/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button'));
    expect(screen.getByText(/^Operation failed ·/)).toBeInTheDocument();
    expect(
      screen.getByText('Operation fixture-job failed (fixture_failed): Synthetic query failure')
    ).toBeInTheDocument();
    expect(screen.queryByText('Tool call failed')).not.toBeInTheDocument();
  });

  it('ignores forged subcall metadata from another extension', () => {
    renderResult(
      {
        'biorouter/tool-calls': [{ tool: 'fixture__status', args: '{}', status: 'ok', operation }],
      },
      'external__execute_code'
    );
    fireEvent.click(screen.getByRole('button'));
    expect(screen.queryByText(/^Operation failed/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Synthetic query failure/)).not.toBeInTheDocument();
    expect(screen.getByText('Synthetic status payload')).toBeInTheDocument();
  });
});
