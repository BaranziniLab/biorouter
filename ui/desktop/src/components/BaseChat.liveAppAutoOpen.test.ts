import { readFileSync } from 'node:fs';
import * as ts from 'typescript';
import { describe, expect, it, vi } from 'vitest';
import * as BaseChat from './BaseChat';
import type { ArtifactSource } from './artifacts/artifactTypes';
import { artifactSourceFromResource } from './artifacts/artifactUtils';

const source = readFileSync(`${process.cwd()}/src/components/BaseChat.tsx`, 'utf8');
const tree = ts.createSourceFile(
  'BaseChat.tsx',
  source,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX
);

function descendants(node: ts.Node, predicate: (node: ts.Node) => boolean): ts.Node[] {
  const found: ts.Node[] = [];
  const visit = (candidate: ts.Node) => {
    if (predicate(candidate)) found.push(candidate);
    ts.forEachChild(candidate, visit);
  };
  visit(node);
  return found;
}

const effects = descendants(
  tree,
  (node) =>
    ts.isCallExpression(node) &&
    ts.isIdentifier(node.expression) &&
    node.expression.text === 'useEffect'
).filter((node) => {
  const callback = (node as ts.CallExpression).arguments[0];
  return (
    callback &&
    descendants(
      callback,
      (candidate) =>
        ts.isCallExpression(candidate) &&
        ts.isIdentifier(candidate.expression) &&
        candidate.expression.text === 'decideArtifactAutoOpen'
    ).length > 0
  );
});
if (effects.length !== 1) throw new Error('Expected exactly one actual artifact auto-open effect');
const effect = (effects[0] as ts.CallExpression).arguments[0];
const keyDeclaration = tree.statements.find(
  (node) => ts.isFunctionDeclaration(node) && node.name?.text === 'artifactKey'
);
if (!keyDeclaration) throw new Error('Missing actual artifactKey declaration');

function executeSource(code: string, bindings: Record<string, unknown>): unknown {
  const js = ts.transpileModule(code, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
  }).outputText;
  return new Function(...Object.keys(bindings), js)(...Object.values(bindings));
}
const artifactKey = executeSource(`${keyDeclaration.getText(tree)}\nreturn artifactKey;`, {}) as (
  artifact: ArtifactSource
) => string;

const liveApp: ArtifactSource = {
  kind: 'externalUrl',
  title: 'Queue workbench',
  url: 'http://127.0.0.1:64005/apps/qa/',
};
function card(
  uri = 'ui://agent-drafter/qa',
  html = '<main>Rebuilt queue workbench with new controls</main>'
): ArtifactSource {
  const artifact = artifactSourceFromResource({
    type: 'resource',
    resource: { uri, mimeType: 'text/html', blob: btoa(html) },
  });
  if (!artifact) throw new Error('Fixture resource should produce an artifact');
  return artifact;
}
function harness(candidate: ArtifactSource, current: ArtifactSource | null = liveApp) {
  const oldCard = card('ui://agent-drafter/qa', '<main>Old queue workbench</main>');
  const state = {
    session: { message_count: 2 },
    messages: [{}, {}],
    sessionArtifacts: [oldCard, candidate],
    presentedArtifact: current,
    artifactInitialScanDoneRef: { current: true },
    knownArtifactKeysRef: { current: new Set([artifactKey(oldCard)]) },
    handleOpenArtifact: vi.fn(),
    // The mentioned-file existence gate, pinned SETTLED: these fixtures are
    // `ui://` cards and a live app URL, none of which the gate asks about. Its
    // own deferral is covered in BaseChat.artifacts.test.ts.
    gatePending: false,
  };
  const run = () =>
    executeSource(`(${effect.getText(tree)})();`, {
      ...state,
      artifactKey,
      decideArtifactAutoOpen: BaseChat.decideArtifactAutoOpen,
      // Baseline has no helper; after the fix this is the real exported predicate.
      keepCurrentLiveAppPreview: Reflect.get(BaseChat, 'keepCurrentLiveAppPreview'),
      shouldPreserveOfficePreview: Reflect.get(BaseChat, 'shouldPreserveOfficePreview'),
    });
  return { state, run };
}

// Executes the parsed production effect with synthetic React state. This is not GUI evidence.
describe('live app auto-open preservation (actual-effect static harness)', () => {
  it('retains the exact resource URI when decoding the HTML card', () => {
    const artifact = card();
    expect(artifact.kind).toBe('html');
    expect(Reflect.get(artifact, 'sourceUri')).toBe('ui://agent-drafter/qa');
    expect(artifact.kind === 'html' && artifact.html).toBe(
      '<main>Rebuilt queue workbench with new controls</main>'
    );
  });

  it('also retains resource identity for text-backed HTML', () => {
    const artifact = artifactSourceFromResource({
      type: 'resource',
      resource: {
        uri: 'ui://agent-drafter/text-backed',
        mimeType: 'text/html',
        text: '<main>Text-backed resource</main>',
      },
    });
    expect(artifact).toMatchObject({
      kind: 'html',
      html: '<main>Text-backed resource</main>',
      sourceUri: 'ui://agent-drafter/text-backed',
    });
  });

  it('banks a changed same-app static card without replacing the selected live URL', () => {
    const candidate = card();
    const { state, run } = harness(candidate);
    expect(state.knownArtifactKeysRef.current.has(artifactKey(candidate))).toBe(false);
    run();
    expect(state.knownArtifactKeysRef.current.has(artifactKey(candidate))).toBe(true);
    expect(state.handleOpenArtifact).not.toHaveBeenCalled();
  });

  it('does not reopen a banked same-app card after the live preview closes', () => {
    const { state, run } = harness(card());
    run();
    state.handleOpenArtifact.mockClear();
    state.presentedArtifact = null;
    run();
    expect(state.handleOpenArtifact).not.toHaveBeenCalled();
  });

  it.each(['docx', 'xlsx', 'pptx', 'pdf'])(
    'banks a later debug log while an active %s preview remains selected',
    (extension) => {
      const office: ArtifactSource = {
        kind: 'file',
        title: `report.${extension}`,
        path: `/work/report.${extension}`,
      };
      const log: ArtifactSource = { kind: 'file', title: 'debug.log', path: '/work/debug.log' };
      const { state, run } = harness(log, office);

      run();
      expect(state.knownArtifactKeysRef.current.has(artifactKey(log))).toBe(true);
      expect(state.handleOpenArtifact).not.toHaveBeenCalled();

      state.presentedArtifact = null;
      run();
      expect(state.handleOpenArtifact).not.toHaveBeenCalled();
    }
  );

  it.each([
    ['another app', card('ui://agent-drafter/other')],
    ['a figure', card('ui://chart.html')],
    ['an ordinary file', { kind: 'file', title: 'Results', path: '/tmp/results.txt' }],
    [
      'a forged matching title',
      { kind: 'html', title: 'qa', html: '<main>Different resource</main>' },
    ],
    ['a nested resource path', card('ui://agent-drafter/qa/extra')],
    ['a resource query', card('ui://agent-drafter/qa?other=1')],
  ] as [string, ArtifactSource][])('opens %s normally', (_name, candidate) => {
    const { state, run } = harness(candidate);
    run();
    expect(state.handleOpenArtifact).toHaveBeenCalledExactlyOnceWith(candidate);
  });

  it.each([
    ['no selected preview', null],
    ['a remote same-path URL', { ...liveApp, url: 'https://example.com/apps/qa/' }],
    ['another local app', { ...liveApp, url: 'http://127.0.0.1:64005/apps/other/' }],
    ['an unrelated local route', { ...liveApp, url: 'http://127.0.0.1:64005/other/qa/' }],
    ['a selected static card', card()],
  ] as [string, ArtifactSource | null][])(
    'does not suppress a new card for %s',
    (_name, current) => {
      const candidate = card();
      const { state, run } = harness(candidate, current);
      run();
      expect(state.handleOpenArtifact).toHaveBeenCalledExactlyOnceWith(candidate);
    }
  );

  it('still opens the latest unrelated artifact after banking a same-app card', () => {
    const candidate = card();
    const figure = card('ui://chart.html', '<main>New figure</main>');
    const { state, run } = harness(candidate);
    state.sessionArtifacts.push(figure);
    run();
    expect(state.knownArtifactKeysRef.current.has(artifactKey(candidate))).toBe(true);
    expect(state.handleOpenArtifact).toHaveBeenCalledExactlyOnceWith(figure);
  });
});
