import { describe, expect, it } from 'vitest';
import { summarizeToolCall } from './ToolCallWithResponse';
import { toolRowLabel } from './toolCallCopy';

/**
 * Subagents run in the background by default and the chat summary lists them,
 * so no tool row may promise a tab. These are the labels the delegation and the
 * phase plan get, running and done.
 */
describe('subagent and phase plan rows', () => {
  const label = (name: string, args: Record<string, unknown>) =>
    summarizeToolCall({ name, arguments: args });

  it('names a delegate by the role the parent gave it', () => {
    expect(
      label('workspace__subagent', { instructions: 'Read every trial report.', role: 'Reader' })
    ).toBe('Delegating to Reader: Read every trial report.');
    expect(label('workspace__subagent', { role: 'Reader' })).toBe('Delegating to Reader');
    // A subworkflow still names itself first; a plain call is unchanged.
    expect(label('subagent', { subworkflow: 'lint', role: 'Reader' })).toBe('Delegating lint');
    expect(label('subagent', {})).toBe('Delegating to a subagent');
  });

  it('labels each workspace_orchestrate action, and conjugates it when done', () => {
    const cases: [Record<string, unknown>, string, string][] = [
      [{ action: 'plan', title: 'Survey' }, 'Setting up phases: Survey', 'Set up phases: Survey'],
      [{ action: 'plan' }, 'Setting up phases', 'Set up phases'],
      [{ action: 'start', phase: 'research' }, 'Starting phase research', 'Started phase research'],
      [
        { action: 'handoff', phase: 'research', summary: 'x' },
        'Writing the hand-off for research',
        'Wrote the hand-off for research',
      ],
      [
        { action: 'complete', phase: 'research' },
        'Completing phase research',
        'Completed phase research',
      ],
      [{ action: 'read' }, 'Reading the phase plan', 'Read the phase plan'],
      [{ action: 'shuffle' }, 'Updating the phase plan', 'Updated the phase plan'],
    ];
    for (const [args, running, done] of cases) {
      const summary = label('workspace__workspace_orchestrate', args);
      expect(summary).toBe(running);
      expect(toolRowLabel(summary, 'done')).toBe(done);
    }
  });

  it('never says a tab was opened', () => {
    const labels = [
      label('workspace__subagent', { instructions: 'Check the numbers.', visible: false }),
      label('workspace__workspace_orchestrate', { action: 'plan', title: 'Plan' }),
    ];
    for (const text of labels) expect(text).not.toMatch(/\btab\b/i);
  });
});
