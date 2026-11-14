import { createRoot, type Root } from 'react-dom/client';
import { useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { ConfigProvider } from '../../ConfigContext';
import type { CrewDraft } from '../state/types';
import { CrewTestProvider, crewTestController } from '../files/crewTestController';
import { AgentTaskPane } from '../pane/AgentTaskPane';
import { Composer } from './Composer';

interface Counts {
  sent: number;
  asked: number;
  started: number;
}

interface CrewDictationLiveHandle {
  counts(): Counts;
  show(mode: 'channel' | 'task'): void;
  unmount(): void;
}

/** Test-only entry mounted by the owned PR-27 Electron live driver. */
export function mountCrewDictationLiveFixture(): CrewDictationLiveHandle {
  const counts: Counts = { sent: 0, asked: 0, started: 0 };
  const host = document.createElement('div');
  host.id = 'pr27-crew-live';
  host.style.cssText =
    'position:fixed;inset:0;z-index:10000;background:var(--background-default,#fff);padding:24px;overflow:auto';
  document.body.append(host);
  const appRoot = document.getElementById('root');
  if (appRoot) appRoot.style.display = 'none';
  let showMode: (mode: 'channel' | 'task') => void = () => undefined;
  let clearDraft: () => void = () => undefined;

  function Fixture() {
    const [mode, setMode] = useState<'channel' | 'task'>('channel');
    const [draft, setDraft] = useState<CrewDraft>({ body: '', attachments: [], references: [] });
    showMode = setMode;
    clearDraft = () => setDraft((current) => ({ ...current, body: '' }));
    const controller = crewTestController({
      draft,
      setBody: (body) => setDraft((current) => ({ ...current, body })),
      send: async () => {
        counts.sent++;
      },
      askAgent: async () => {
        counts.asked++;
        return true;
      },
      startOwnedRun: async () => {
        counts.started++;
        return true;
      },
    });
    return (
      <MemoryRouter>
        <ConfigProvider>
          <CrewTestProvider controller={controller}>
            {mode === 'channel' ? (
              <div data-pr27-field="channel" style={{ maxWidth: 760 }}>
                <Composer />
              </div>
            ) : (
              <div data-pr27-field="task" style={{ maxWidth: 760 }}>
                <AgentTaskPane />
              </div>
            )}
          </CrewTestProvider>
        </ConfigProvider>
      </MemoryRouter>
    );
  }

  const root: Root = createRoot(host);
  root.render(<Fixture />);
  return {
    counts: () => ({ ...counts }),
    show: (mode) => {
      clearDraft();
      showMode(mode);
    },
    unmount: () => {
      root.unmount();
      host.remove();
      if (appRoot) appRoot.style.display = '';
    },
  };
}
