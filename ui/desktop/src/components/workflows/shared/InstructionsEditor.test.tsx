import { useState } from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { Dialog, DialogContent, DialogTitle } from '../../ui/dialog';
import InstructionsEditor from './InstructionsEditor';

afterEach(cleanup);

describe('InstructionsEditor', () => {
  it('closes only the nested editor when Escape is pressed', async () => {
    const user = userEvent.setup();

    function Harness() {
      const [parentOpen, setParentOpen] = useState(true);
      const [editorOpen, setEditorOpen] = useState(true);

      return (
        <Dialog open={parentOpen} onOpenChange={setParentOpen}>
          <DialogContent aria-describedby={undefined}>
            <DialogTitle>Workflow editor</DialogTitle>
            <InstructionsEditor
              isOpen={editorOpen}
              onClose={() => setEditorOpen(false)}
              value="Original instructions"
              onChange={() => {}}
            />
          </DialogContent>
        </Dialog>
      );
    }

    render(<Harness />);
    expect(screen.getByText('Instructions editor')).toBeInTheDocument();

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByText('Instructions editor')).not.toBeInTheDocument());
    expect(screen.getByText('Workflow editor')).toBeInTheDocument();
  });
});
