/**
 * Mirrored tool calls — the GUI parity gate for the coding-agent providers.
 *
 * A `claude_code` / `codex` turn replays the tool calls its child made as
 * ordinary `toolRequest` / `toolResponse` message content, stamped with
 * `biorouterProviderExecuted` in the per-tool provider metadata that
 * `ToolRequest` / `ToolResponse` already carry
 * (`crates/biorouter/src/providers/coding_agent/mirror.rs`). The claim this file
 * makes executable:
 *
 *   - a mirrored pair renders through the SAME card as an API provider's pair —
 *     same name, same status, same expandable arguments, same expandable result;
 *   - a `bridged` pair renders **byte-identically** to a pair with no metadata
 *     at all (that is the parity assertion, and the point of the phase);
 *   - a `child` pair — one the child agent ran in its own sandbox, which passed
 *     none of Biorouter's gates — is the ONE case that says so on the card.
 */
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import ToolCallWithResponse, { providerExecutionOf } from './ToolCallWithResponse';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../types/message';

// The transcript is no longer a display surface for artifacts — it can only hand
// one to the panel — so `onOpenArtifact` is required all the way down the chain.
const noopOpenArtifact = vi.fn();

const CHILD_LABEL = /not gated by Biorouter/;

/** The exact wire shape a mirrored request arrives in. */
function mirroredRequest(
  executed?: 'bridged' | 'child',
  overrides: Partial<ToolRequestMessageContent> = {}
): ToolRequestMessageContent {
  return {
    type: 'toolRequest',
    id: 'toolu_1',
    toolCall: {
      status: 'success',
      value: {
        name: 'developer__shell',
        arguments: { command: 'npm run typecheck' },
      },
    },
    ...(executed ? { metadata: { biorouterProviderExecuted: executed } } : {}),
    ...overrides,
  };
}

function mirroredResponse(
  executed?: 'bridged' | 'child',
  { isError = false, text = 'ok, 0 errors' }: { isError?: boolean; text?: string } = {}
): ToolResponseMessageContent {
  return {
    type: 'toolResponse',
    id: 'toolu_1',
    toolResult: {
      status: 'success',
      value: {
        content: [{ type: 'text', text }],
        isError,
      },
    },
    ...(executed ? { metadata: { biorouterProviderExecuted: executed } } : {}),
  };
}

describe('providerExecutionOf', () => {
  it('reads the marker off either half of the pair', () => {
    expect(providerExecutionOf(mirroredRequest('bridged'), mirroredResponse('bridged'))).toBe(
      'bridged'
    );
    expect(providerExecutionOf(mirroredRequest(), mirroredResponse('child'))).toBe('child');
    expect(providerExecutionOf(mirroredRequest('child'), mirroredResponse())).toBe('child');
  });

  it('reports nothing for an ordinary provider pair, and for a marker it does not know', () => {
    expect(providerExecutionOf(mirroredRequest(), mirroredResponse())).toBeNull();
    expect(providerExecutionOf(undefined, undefined)).toBeNull();
    // Fail-safe direction, matching the backend: only a positively recognised
    // value earns a provenance claim. A future marker kind must not be
    // mistaken for one Biorouter gated.
    expect(
      providerExecutionOf(
        mirroredRequest(undefined, { metadata: { biorouterProviderExecuted: 'teleported' } }),
        undefined
      )
    ).toBeNull();
  });
});

describe('a mirrored tool call renders as an ordinary tool call', () => {
  it('shows the tool name, a success status, and expandable arguments and result', () => {
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest('bridged')}
        toolResponse={mirroredResponse('bridged')}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    // Name: summarised exactly as the same call from an API provider is.
    const row = screen.getByText(/Running npm run typecheck/);
    expect(row).toBeInTheDocument();
    // Status: a result arrived and it is not an error, so "Ran …", never
    // "Problem with" and never the pending "No result".
    expect(screen.getByText(/^Ran/)).toBeInTheDocument();
    expect(screen.queryByText(/Problem with/)).not.toBeInTheDocument();
    expect(screen.queryByText(/No result/)).not.toBeInTheDocument();
    expect(screen.getByText(/1 result ready/)).toBeInTheDocument();

    // Arguments: collapsed at rest, reachable in two clicks like any other card.
    expect(screen.queryByText('command')).not.toBeInTheDocument();
    fireEvent.click(row.closest('button') as HTMLElement);
    expect(screen.getByText('command')).toBeInTheDocument();
    expect(screen.getByText('npm run typecheck')).toBeInTheDocument();

    // Result: its own disclosure, holding the child's output.
    expect(screen.getByText('ok, 0 errors')).toBeInTheDocument();
  });

  it('paints a failed mirrored call as a failure, from isError on the result', () => {
    const { container } = render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest('bridged')}
        toolResponse={mirroredResponse('bridged', {
          isError: true,
          text: 'command not found: npm',
        })}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    // A mirrored failure travels as a SUCCESSFUL transport carrying
    // `isError: true` — the spelling `getToolResultError` reads — so the card
    // preserves the failure text without a colored background.
    expect(screen.getByText(/Problem with/)).toBeInTheDocument();
    expect(screen.getByText(/Tool call failed/)).toBeInTheDocument();
    expect(container.querySelector('.bg-background-danger\\/5')).toBeNull();

    fireEvent.click(screen.getByText(/Problem with/).closest('button') as HTMLElement);
    expect(screen.getByText('command not found: npm')).toBeInTheDocument();
  });

  it('is still pending-shaped while the turn runs and no response has arrived', () => {
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest('bridged')}
        turnActive
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.getByText(/Working on/)).toBeInTheDocument();
    expect(screen.queryByText(/^Ran/)).not.toBeInTheDocument();
  });
});

describe('the child-executed affordance', () => {
  it('says so on a call the child agent ran in its own sandbox', () => {
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest('child')}
        toolResponse={mirroredResponse('child')}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    const label = screen.getByText(CHILD_LABEL);
    expect(label).toBeInTheDocument();
    // Honest, and explained on hover rather than shouted in the row.
    expect(label).toHaveAttribute(
      'title',
      expect.stringContaining("ran inside the coding agent's own sandbox")
    );
    // Quiet: the row's own muted type, no badge, no status colour of its own.
    expect(label.closest('button')).toHaveClass('br-tool-disclosure');
    expect(label.className).not.toContain('text-text-default');
    expect(label.className).not.toContain('bg-');
    // Everything else about the card is unchanged.
    expect(screen.getByText(/Running npm run typecheck/)).toBeInTheDocument();
    expect(screen.getByText(/^Ran/)).toBeInTheDocument();
  });

  it('shows it when only the response carries the marker', () => {
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest()}
        toolResponse={mirroredResponse('child')}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.getByText(CHILD_LABEL)).toBeInTheDocument();
  });

  it('is ABSENT on a bridged call, which Biorouter did gate', () => {
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest('bridged')}
        toolResponse={mirroredResponse('bridged')}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.queryByText(CHILD_LABEL)).not.toBeInTheDocument();
  });

  it('is ABSENT on an ordinary API-provider call', () => {
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest()}
        toolResponse={mirroredResponse()}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.queryByText(CHILD_LABEL)).not.toBeInTheDocument();
  });
});

/**
 * THE parity assertion. A bridged mirrored pair passed every gate an API
 * provider's call passes, so it must be indistinguishable from one — not
 * "similar", identical markup. Anything that ever puts a badge on a mirrored
 * card by default fails here.
 */
describe('parity with an API-provider tool call', () => {
  const renderPair = (executed?: 'bridged' | 'child', isError = false) =>
    render(
      <ToolCallWithResponse
        isCancelledMessage={false}
        toolRequest={mirroredRequest(executed)}
        toolResponse={mirroredResponse(executed, { isError })}
        onOpenArtifact={noopOpenArtifact}
      />
    ).container.innerHTML.replace(/aria-controls="[^"]+"/g, 'aria-controls="content"');

  it('renders a bridged pair identically to a pair with no metadata at all', () => {
    expect(renderPair('bridged')).toBe(renderPair(undefined));
  });

  it('holds for the failure cell too', () => {
    expect(renderPair('bridged', true)).toBe(renderPair(undefined, true));
  });

  it('differs from an unmarked pair only by the child label', () => {
    const child = renderPair('child');
    const plain = renderPair(undefined);
    expect(child).not.toBe(plain);
    expect(child).toContain('not gated by Biorouter');
    expect(plain).not.toContain('not gated by Biorouter');
  });
});
