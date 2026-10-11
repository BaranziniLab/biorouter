import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Vision round 1, V3-06 (spec 3.11: "Cancel uses secondary"). Cancel was a ghost text button in
 * every Views-A dialog and a filled secondary in Skills, Extensions, BAAM and Knowledge, so the two
 * sets of dialogs looked like different apps. One look per control (principle 5): Cancel is the
 * filled `secondary` button everywhere.
 *
 * Asserted at the source because the rendering tests only reach a dialog they open, and there are
 * nine of these; a tenth is caught by the file list below going out of date (it is a glob of the
 * three areas plus the one top-level modal, filtered to files that draw a Cancel button).
 */
const COMPONENTS = join(__dirname, '..');
const FILES = [
  'workflows/ImportWorkflowForm.tsx',
  'workflows/CreateEditWorkflowModal.tsx',
  'workflows/CreateWorkflowFromSessionModal.tsx',
  'workflows/WorkflowsView.tsx',
  'workflows/shared/InstructionsEditor.tsx',
  'workflows/shared/JsonSchemaEditor.tsx',
  'schedule/ScheduleModal.tsx',
  'applications/ExportAppDialog.tsx',
  'ParameterInputModal.tsx',
];

/** The `<Button …>` that opens the button whose label is at `labelAt`. */
function buttonOpeningBefore(source: string, labelAt: number): string {
  const open = source.lastIndexOf('<Button', labelAt);
  return source.slice(open, labelAt);
}

describe('Cancel in the Views-A dialogs', () => {
  it.each(FILES)('%s draws Cancel as the filled secondary button', (file) => {
    const source = readFileSync(join(COMPONENTS, file), 'utf8');
    const labels = [...source.matchAll(/>\s*\{\s*(?:copy|scheduleCopy)\.cancel\s*\}/g)];
    expect(labels.length).toBeGreaterThan(0);
    for (const label of labels) {
      const button = buttonOpeningBefore(source, label.index!);
      expect(button, `${file}: ${button}`).toMatch(/variant="secondary"/);
      expect(button).not.toMatch(/variant="(ghost|outline)"/);
    }
  });
});
