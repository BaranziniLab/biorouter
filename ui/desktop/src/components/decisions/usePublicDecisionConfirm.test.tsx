import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { PublicDecisionConfirmation } from '../../utils/sessionDecisions';
import { usePublicDecisionConfirm, type PublicDecisionAsk } from './usePublicDecisionConfirm';

/** DR-33: the question a start asks when the daemon says the chat would be public. */

const QUESTION: PublicDecisionConfirmation = {
  message: 'This chat will be public: joey-1 runs on Baseten.',
  info: 'A chat is as private as its least private model.',
  models: [{ provider: 'baseten_decision', model: 'joey-1', adapter: null }],
};

function Asker({ ask, onAnswer }: { ask: PublicDecisionAsk; onAnswer: (yes: boolean) => void }) {
  const { confirm, dialog } = usePublicDecisionConfirm(ask);
  return (
    <>
      <button onClick={() => void confirm(QUESTION).then(onAnswer)}>start</button>
      {dialog}
    </>
  );
}

describe('the "this chat will be public" question', () => {
  it('a Send asks with the daemon’s line, and Send answers yes', async () => {
    const answers: boolean[] = [];
    render(<Asker ask="send" onAnswer={(yes) => answers.push(yes)} />);
    fireEvent.click(screen.getByText('start'));
    expect(await screen.findByRole('dialog', { name: 'Make this chat public?' })).toHaveTextContent(
      'This chat will be public: joey-1 runs on Baseten.'
    );
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Send' })));
    expect(answers).toEqual([true]);
  });

  it('a workflow start names the models it uses, and Cancel answers no', async () => {
    const answers: boolean[] = [];
    render(<Asker ask="workflow" onAnswer={(yes) => answers.push(yes)} />);
    fireEvent.click(screen.getByText('start'));
    expect(await screen.findByRole('dialog', { name: 'Start this workflow?' })).toHaveTextContent(
      'This workflow uses joey-1 · Baseten, so this chat will be public.'
    );
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Cancel' })));
    expect(answers).toEqual([false]);
  });

  it('an asker that goes away mid-question answers no', async () => {
    const answers: boolean[] = [];
    const view = render(<Asker ask="send" onAnswer={(yes) => answers.push(yes)} />);
    fireEvent.click(screen.getByText('start'));
    await screen.findByRole('dialog');
    await act(async () => view.unmount());
    expect(answers).toEqual([false]);
  });
});
