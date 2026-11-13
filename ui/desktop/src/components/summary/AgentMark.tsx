import type { ReactNode } from 'react';
import type { AgentMarkKind, AgentMarkShape } from './agentGrid';
import './summaryAgentGrid.css';

/**
 * One mark of the agent grid's notation (`agentGrid.ts`): a square for a
 * language-model agent, a circle for decision-model work, filled by status.
 *
 * Drawn in a 12-unit box and scaled to `size`, in `currentColor`, so the status
 * ink is one `color` (summaryAgentGrid.css) and forced colours repaint the whole
 * mark in the system ink while its shape still says where it stands. A failure
 * is the outline with a small cross inside, in the muted ink at full strength
 * (owner direction, 2026-11-12: calm, never red, never faded), and its word
 * says "Failed".
 *
 * Always `aria-hidden`: the control that carries a mark names the status in words.
 */

/** The small cross inside a failed mark's outline, drawn with the ring's stroke. */
const CROSS = 'M4.25 4.25 L7.75 7.75 M7.75 4.25 L4.25 7.75';

const SOLID: Record<AgentMarkShape, string> = {
  square:
    'M2.5 0 H9.5 A2.5 2.5 0 0 1 12 2.5 V9.5 A2.5 2.5 0 0 1 9.5 12 H2.5 A2.5 2.5 0 0 1 0 9.5 V2.5 A2.5 2.5 0 0 1 2.5 0 Z',
  circle: 'M0 6 A6 6 0 1 1 12 6 A6 6 0 1 1 0 6 Z',
};

const LOWER_HALF: Record<AgentMarkShape, string> = {
  square: 'M0.75 6 H11.25 V9.25 A2 2 0 0 1 9.25 11.25 H2.75 A2 2 0 0 1 0.75 9.25 Z',
  circle: 'M0.75 6 A5.25 5.25 0 0 0 11.25 6 Z',
};

function Ring({ shape }: { shape: AgentMarkShape }) {
  return shape === 'circle' ? (
    <circle className="br-agent-mark__ring" cx="6" cy="6" r="5.25" />
  ) : (
    <rect className="br-agent-mark__ring" x="0.75" y="0.75" width="10.5" height="10.5" rx="2" />
  );
}

function Core({ shape }: { shape: AgentMarkShape }) {
  return shape === 'circle' ? (
    <circle className="br-agent-mark__core" cx="6" cy="6" r="2.5" />
  ) : (
    <rect className="br-agent-mark__core" x="3.5" y="3.5" width="5" height="5" rx="1" />
  );
}

function body(kind: AgentMarkKind, shape: AgentMarkShape): ReactNode {
  switch (kind) {
    case 'queued':
      return <Ring shape={shape} />;
    case 'running':
      return (
        <>
          <Ring shape={shape} />
          <Core shape={shape} />
        </>
      );
    case 'waiting':
      return (
        <>
          <Ring shape={shape} />
          <path className="br-agent-mark__fill" d={LOWER_HALF[shape]} />
        </>
      );
    case 'done':
      return <path className="br-agent-mark__fill" d={SOLID[shape]} />;
    case 'failed':
      return (
        <>
          <Ring shape={shape} />
          <path className="br-agent-mark__cross" d={CROSS} />
        </>
      );
    case 'idle':
      return (
        <>
          <Ring shape={shape} />
          <rect
            className="br-agent-mark__fill"
            x="3.25"
            y="5.25"
            width="5.5"
            height="1.5"
            rx="0.75"
          />
        </>
      );
  }
}

export function AgentMark({
  kind,
  shape = 'square',
  size,
  pulse = false,
  testId,
}: {
  kind: AgentMarkKind;
  shape?: AgentMarkShape;
  /** The side in CSS pixels (`AGENT_MARK_PX`). */
  size: number;
  /** A running child's core breathes; never under reduced motion. */
  pulse?: boolean;
  testId?: string;
}) {
  return (
    <svg
      className="br-agent-mark"
      viewBox="0 0 12 12"
      width={size}
      height={size}
      // Inline, so a surrounding glyph slot's own svg size (`.br-summary__glyph >
      // svg`) cannot stretch the mark.
      style={{ width: size, height: size }}
      data-kind={kind}
      data-shape={shape}
      data-pulse={pulse ? '' : undefined}
      data-small={size < 9 ? '' : undefined}
      data-testid={testId}
      aria-hidden="true"
      focusable="false"
    >
      {body(kind, shape)}
    </svg>
  );
}
