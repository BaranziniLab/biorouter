// Run with Node's built-in test runner; no installed packages are needed.
// Only the copied release dispatcher and its provenance helpers execute. Git
// stays inside the disposable project; gh and native attestation are fixtures.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync,
  readdirSync, realpathSync, rmSync, statSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, sep } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const SOURCE_RELEASE = fileURLToPath(new URL('../release.sh', import.meta.url));
const SOURCE_CROSS_ENV = fileURLToPath(new URL('../cross-env.sh', import.meta.url));
const VERSION = '9.9.9';
const FOREIGN_SHA = 'a'.repeat(40);
const WORKFLOWS = ['linux-gui-packages.yml', 'windows-gui-packages.yml'];
const NODE = realpathSync(process.execPath);
const GIT = '/usr/bin/git';
const BASH = '/bin/bash';
const TIMEOUT = 30_000;
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const fileSha256 = (path) => sha256(readFileSync(path));
const shQuote = (text) => `'${text.replaceAll("'", "'\\''")}'`;

const adoptedPaths = [
  `ui/desktop/out/make/deb/x64/biorouter_${VERSION}_amd64.deb`,
  `ui/desktop/out/make/rpm/x64/Biorouter-${VERSION}-1.x86_64.rpm`,
  `dist/cli/biorouter-cli_${VERSION}_amd64.deb`,
  `dist/cli/biorouter-cli-${VERSION}-1.x86_64.rpm`,
  `ui/desktop/out/make/zip/win32/x64/Biorouter-win32-x64-${VERSION}.zip`,
  `ui/desktop/out/make/squirrel.windows/x64/Biorouter-Setup-${VERSION}.exe`,
];
const localPaths = [
  `ui/desktop/out/make/Biorouter-${VERSION}-arm64.dmg`,
  `ui/desktop/out/make/Biorouter-${VERSION}-x64.dmg`,
  `ui/desktop/out/make/zip/darwin/arm64/Biorouter-darwin-arm64-${VERSION}.zip`,
  `ui/desktop/out/make/zip/darwin/x64/Biorouter-darwin-x64-${VERSION}.zip`,
  'ui/desktop/out/make/latest-mac.yml',
];

// A stored ZIP with one ordinary text member. Native packages/executables below
// remain synthetic byte payloads; none of them represents an installable app.
function tinyZip(name, text) {
  const filename = Buffer.from(name);
  const data = Buffer.from(text);
  let crc = 0xffffffff;
  for (const byte of data) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  crc = (crc ^ 0xffffffff) >>> 0;
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4);
  local.writeUInt32LE(crc, 14);
  local.writeUInt32LE(data.length, 18);
  local.writeUInt32LE(data.length, 22);
  local.writeUInt16LE(filename.length, 26);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4);
  central.writeUInt16LE(20, 6);
  central.writeUInt32LE(crc, 16);
  central.writeUInt32LE(data.length, 20);
  central.writeUInt32LE(data.length, 24);
  central.writeUInt16LE(filename.length, 28);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(central.length + filename.length, 12);
  end.writeUInt32LE(local.length + filename.length + data.length, 16);
  return Buffer.concat([local, filename, data, central, filename, end]);
}

const STUB = String.raw`
import { appendFileSync, copyFileSync, mkdirSync, readFileSync, readdirSync, realpathSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { basename, dirname, join, resolve } from 'node:path';
const [tool, ...args] = process.argv.slice(2);
const repo = realpathSync(process.env.FIXTURE_REPO);
const control = realpathSync(process.env.FIXTURE_CONTROL);
const config = JSON.parse(readFileSync(join(control, 'config.json'), 'utf8'));
function stop(text) { console.error('fixture boundary: ' + text); process.exit(97); }
function option(name) { const index = args.indexOf(name); if (index < 0 || index + 1 >= args.length) stop('missing ' + name); return args[index + 1]; }
function log(kind) { appendFileSync(join(control, 'calls.jsonl'), JSON.stringify({ kind, args }) + '\n'); }
if (tool === 'mktemp') {
  const temporary = realpathSync(process.env.TMPDIR);
  if (temporary !== join(dirname(repo), 'temporary')) stop('unexpected fixture temporary root');
  let command;
  if (JSON.stringify(args) === JSON.stringify(['-d'])) {
    // macOS's bare -d prefers _CS_DARWIN_USER_TEMP_DIR over TMPDIR.
    command = ['-d', join(temporary, 'adopt-ci.XXXXXX')];
  } else if (args.length === 1) {
    const template = resolve(args[0]);
    if (basename(template) !== 'release-build-' + config.version + '.tsv.XXXXXX' || realpathSync(dirname(template)) !== join(repo, 'dist')) stop('temporary file escaped fixture provenance');
    command = [join(repo, 'dist', basename(template))];
  } else stop('unexpected mktemp arguments');
  const result = spawnSync('/usr/bin/mktemp', command, { env: process.env, encoding: 'utf8', timeout: 10000 });
  if (result.error) stop(result.error.message);
  process.stdout.write(result.stdout || ''); process.stderr.write(result.stderr || ''); process.exit(result.status ?? 98);
}
if (tool === 'git') {
  if (args[0] !== '-C' || realpathSync(args[1]) !== repo) stop('git escaped disposable project');
  const command = args.slice(2);
  if (!(JSON.stringify(command) === JSON.stringify(['rev-parse', 'HEAD']) || JSON.stringify(command) === JSON.stringify(['status', '--porcelain', '--untracked-files=normal']))) stop('unexpected git command');
  const result = spawnSync('/usr/bin/git', args, { env: process.env, encoding: 'utf8', timeout: 10000 });
  if (result.error) stop(result.error.message);
  process.stdout.write(result.stdout || ''); process.stderr.write(result.stderr || ''); process.exit(result.status ?? 98);
}
if (tool !== 'gh') { log('blocked'); stop('unexpected external tool ' + tool); }
log('gh');
if (args[0] === 'run' && args[1] === 'list') {
  const workflow = option('--workflow');
  if (!config.runs[workflow]) stop('unknown workflow ' + workflow);
  if (config.queryError === workflow) { console.error('synthetic gh query failure'); process.exit(42); }
  const query = option('--jq');
  if (option('--json') !== 'databaseId,headSha,status,conclusion' || option('--limit') !== '50') stop('unexpected run projection');
  const source = query.match(/\.headSha == "([a-f0-9]{40})"/);
  let runs = config.runs[workflow];
  if (source) runs = runs.filter((run) => run.headSha === source[1]);
  if (query.includes('.status == "completed"')) runs = runs.filter((run) => run.status === 'completed');
  if (query.includes('.conclusion == "success"')) runs = runs.filter((run) => run.conclusion === 'success');
  const first = runs[0];
  if (query.endsWith('| .databaseId')) console.log(first?.databaseId ?? 'null');
  else if (query.includes('\\(.databaseId) \\(.status) \\(.conclusion)')) console.log(first ? first.databaseId + ' ' + first.status + ' ' + (first.conclusion ?? 'null') : 'null');
  else stop('unexpected jq query');
  process.exit(0);
}
if (args[0] === 'run' && args[1] === 'download') {
  const id = args[2];
  const entry = Object.entries(config.runs).find(([, runs]) => runs.some((run) => String(run.databaseId) === id));
  if (!entry) stop('unknown selected run ' + id);
  const platform = entry[0] === 'linux-gui-packages.yml' ? 'linux' : 'windows';
  if (option('-n') !== platform + '-packages-' + config.version) stop('artifact version/name mismatch');
  const requested = resolve(option('-D'));
  const stage = realpathSync(dirname(requested));
  if (basename(requested) !== platform || dirname(stage) !== realpathSync(process.env.TMPDIR)) stop('download escaped fixture TMPDIR');
  const destination = join(stage, platform);
  mkdirSync(destination);
  if (config.downloadFailure === platform) { console.error('synthetic download failure'); process.exit(43); }
  function copyTree(source, target) {
    mkdirSync(target, { recursive: true });
    for (const entry of readdirSync(source, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) stop('payload link is forbidden');
      const from = join(source, entry.name); const to = join(target, entry.name);
      if (entry.isDirectory()) copyTree(from, to); else copyFileSync(from, to);
    }
  }
  copyTree(join(control, 'payload', platform), destination);
  process.exit(0);
}
if (args[0] === 'api') {
  const id = args[1]?.match(/^repos\/\{owner\}\/\{repo\}\/actions\/runs\/(\d+)$/)?.[1];
  if (!id || option('--jq') !== '[.path, .head_sha, .status, .conclusion] | @tsv') stop('unexpected API request');
  const run = config.api[id];
  if (!run) stop('unknown run API identity');
  console.log([run.path, run.head_sha, run.status, run.conclusion].join('\t')); process.exit(0);
}
stop('no workflow dispatch, release, upload, publish, credentials or network command is permitted');
`;

const ATTESTATION = String.raw`import hashlib,json,os,pathlib,sys,zipfile
p=pathlib.Path(sys.argv[1]).resolve()
root=pathlib.Path(os.environ['FIXTURE_REPO']).resolve()
if root not in p.parents:
    raise SystemExit('fixture attestation escaped disposable project')
if len(sys.argv)!=3 or sys.argv[2] not in ('linux-x64','win32-x64'):
    raise SystemExit('unexpected native/signing attestation request')
with open(pathlib.Path(os.environ['FIXTURE_CONTROL'])/'attestations.jsonl','a') as out:
    out.write(json.dumps({'path':str(p),'target':sys.argv[2]})+'\n')
if p.suffix=='.zip':
    try:
        with zipfile.ZipFile(p) as archive:
            if not archive.namelist() or archive.testzip() is not None:
                raise ValueError('invalid ZIP member')
    except (zipfile.BadZipFile,ValueError) as error:
        print('synthetic archive verifier: invalid ZIP',file=sys.stderr)
        raise SystemExit(44) from error
print(json.dumps({'fixture_only':True,'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'target':sys.argv[2]}))
`;

function put(path, bytes) { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, bytes); }
function lines(path) { return existsSync(path) ? readFileSync(path, 'utf8').trim().split('\n').filter(Boolean) : []; }
function jsonLines(path) { return lines(path).map((line) => JSON.parse(line)); }

function fixture(t) {
  const base = realpathSync(mkdtempSync(join(tmpdir(), 'biorouter-adopt-ci-')));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const repo = join(base, 'project');
  const control = join(base, 'control');
  const bin = join(base, 'bin');
  const temp = join(base, 'temporary');
  const home = join(base, 'home');
  for (const dir of [repo, control, bin, temp, home, join(base, 'hooks'), join(base, 'git-template')]) mkdirSync(dir, { recursive: true });
  const env = {
    PATH: `${bin}:/usr/bin:/bin:/usr/sbin:/sbin`, HOME: home,
    TMPDIR: `${temp}${sep}`, LANG: 'C', LC_ALL: 'C', TZ: 'UTC',
    BIOROUTER_DISABLE_KEYRING: 'true', BIOROUTER_DEV_PROFILE_ROOT: join(base, 'crew'),
    BIOROUTER_PATH_ROOT: join(base, 'application'), XDG_CONFIG_HOME: join(base, 'xdg'),
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: join(home, 'gitconfig'),
    GIT_CONFIG_COUNT: '2', GIT_CONFIG_KEY_0: 'core.hooksPath', GIT_CONFIG_VALUE_0: join(base, 'hooks'),
    GIT_CONFIG_KEY_1: 'commit.gpgsign', GIT_CONFIG_VALUE_1: 'false',
    GIT_AUTHOR_NAME: 'Release Fixture', GIT_AUTHOR_EMAIL: 'release-fixture@example.invalid',
    GIT_COMMITTER_NAME: 'Release Fixture', GIT_COMMITTER_EMAIL: 'release-fixture@example.invalid',
    FIXTURE_REPO: repo, FIXTURE_CONTROL: control,
  };
  put(env.GIT_CONFIG_GLOBAL, '');
  copyFileSync(SOURCE_RELEASE, putPath('scripts/release.sh'));
  copyFileSync(SOURCE_CROSS_ENV, putPath('scripts/cross-env.sh'));
  put(putPath('scripts/verify-computer-use-artifact.py'), ATTESTATION);
  put(putPath('Cargo.toml'), `[workspace.package]\nversion = "${VERSION}"\n`);
  put(putPath('.gitignore'), '/dist/\n/ui/desktop/out/\n');
  put(join(control, 'stub.mjs'), STUB);
  for (const tool of ['git', 'gh', 'mktemp', 'curl', 'wget', 'security', 'codesign', 'xcrun', 'sudo', 'docker', 'open', 'osascript', 'ssh', 'scp', 'cargo', 'rustc', 'npm', 'npx', 'just', 'pbcopy']) {
    put(join(bin, tool), `#!/bin/sh\nexec ${shQuote(NODE)} ${shQuote(join(control, 'stub.mjs'))} ${shQuote(tool)} "$@"\n`);
  }
  for (const tool of readdirSync(bin)) chmodSync(join(bin, tool), 0o755);
  const release = join(repo, 'scripts/release.sh');
  const manifest = join(repo, 'dist', `release-build-${VERSION}.tsv`);
  const f = { base, repo, control, bin, temp, home, env, release, manifest, version: VERSION };
  function putPath(path) { const target = join(repo, path); mkdirSync(dirname(target), { recursive: true }); return target; }
  f.run = (binary, args) => {
    assert.ok(isAbsolute(binary));
    const result = spawnSync(binary, args, { cwd: repo, env, encoding: 'utf8', timeout: TIMEOUT, maxBuffer: 1024 * 1024 });
    assert.ifError(result.error);
    assert.equal(result.signal, null, `fixture command exceeded its bound: ${result.signal}`);
    return result;
  };
  f.git = (...args) => {
    assert.ok(['init', 'add', 'commit', 'rev-parse', 'status'].includes(args[0]), 'fixture setup never uses remotes or external refs');
    const result = f.run(GIT, args);
    assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
  };
  f.helper = (name) => {
    assert.ok(['start_release_provenance', 'verify_release_provenance', 'verify_release_ci_runs'].includes(name));
    return f.run(BASH, ['-c', 'set -euo pipefail; source "$1"; "$2" "$3"', 'adopt-ci-fixture', release, name, VERSION]);
  };
  f.adopt = (version = VERSION) => f.run(BASH, [release, 'adopt-ci', version]);
  f.calls = () => jsonLines(join(control, 'calls.jsonl'));
  f.downloads = () => f.calls().filter((call) => call.kind === 'gh' && call.args[0] === 'run' && call.args[1] === 'download');
  f.rows = (kind) => lines(manifest).map((line) => line.split('\t')).filter((row) => row[0] === kind);
  f.saveConfig = () => put(join(control, 'config.json'), `${JSON.stringify(f.config)}\n`);
  f.git('init', '-q', '-b', 'main', `--template=${join(base, 'git-template')}`);
  f.git('add', 'Cargo.toml', '.gitignore', 'scripts');
  f.git('commit', '-q', '-m', 'isolated adopt-ci fixture');
  f.sourceSha = f.git('rev-parse', 'HEAD');
  f.config = { version: VERSION, runs: {}, api: {} };
  for (const [index, workflow] of WORKFLOWS.entries()) {
    const goodId = index === 0 ? 101 : 202;
    f.config.runs[workflow] = [
      { databaseId: 900 + index * 10, headSha: FOREIGN_SHA, status: 'completed', conclusion: 'success' },
      { databaseId: 901 + index * 10, headSha: f.sourceSha, status: 'completed', conclusion: 'failure' },
      { databaseId: 902 + index * 10, headSha: f.sourceSha, status: 'in_progress', conclusion: null },
      { databaseId: goodId, headSha: f.sourceSha, status: 'completed', conclusion: 'success' },
    ];
    f.config.api[goodId] = { path: `.github/workflows/${workflow}`, head_sha: f.sourceSha, status: 'completed', conclusion: 'success' };
  }
  f.saveConfig();
  f.payloads = new Map();
  for (const [index, path] of adoptedPaths.entries()) {
    const platform = index < 4 ? 'linux' : 'windows';
    const filename = path.split('/').at(-1);
    const payloadPath = join(control, 'payload', platform, 'uploaded', 'nested', filename);
    const bytes = filename.endsWith('.zip') ? tinyZip('fixture-version.txt', `${VERSION}\n`) : Buffer.from(`synthetic ${filename}\n`);
    put(payloadPath, bytes); f.payloads.set(path, { path: payloadPath, bytes });
  }
  assert.equal(fileSha256(release), fileSha256(SOURCE_RELEASE), 'fixture must execute the exact real release script');
  assert.equal(fileSha256(join(repo, 'scripts/cross-env.sh')), fileSha256(SOURCE_CROSS_ENV));
  const started = f.helper('start_release_provenance');
  assert.equal(started.status, 0, started.stderr);
  return f;
}

function expectSuccess(result) { assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`); }
function expectRefusal(result, pattern) {
  assert.notEqual(result.status, 0, 'actual release dispatcher unexpectedly succeeded');
  assert.match(`${result.stdout}\n${result.stderr}`, pattern);
  assert.doesNotMatch(result.stdout, /adopted 6 CI-built assets/);
}
function assertNoExternalMutation(f) {
  assert.deepEqual(f.calls().filter((call) => call.kind === 'blocked'), []);
  assert.deepEqual(readdirSync(f.temp), [], 'download staging must be removed on every exit');
}
function assertNoDownloads(f) {
  assert.deepEqual(f.downloads(), []);
  for (const path of adoptedPaths) assert.equal(existsSync(join(f.repo, path)), false);
  assert.deepEqual(f.rows('asset'), []);
  assert.deepEqual(f.rows('ci_run'), []);
  assertNoExternalMutation(f);
}
function assertAdoptedBytes(f) {
  assert.deepEqual(f.rows('source_sha'), [['source_sha', f.sourceSha]]);
  assert.deepEqual(f.rows('version'), [['version', VERSION]]);
  const rows = f.rows('asset');
  assert.equal(rows.length, 6);
  for (const path of adoptedPaths) {
    const destination = join(f.repo, path);
    assert.deepEqual(readFileSync(destination), f.payloads.get(path).bytes);
    const row = rows.filter((entry) => entry[1] === path);
    assert.deepEqual(row, [['asset', path, fileSha256(destination), String(statSync(destination).size)]]);
  }
  assert.deepEqual(f.rows('ci_run'), [['ci_run', WORKFLOWS[0], '101'], ['ci_run', WORKFLOWS[1], '202']]);
  assert.equal(f.rows('computer_use').length, 5, 'only four Linux archives and the Windows ZIP are directly attested');
  assert.deepEqual(f.downloads().slice(-2).map((call) => [call.args[2], call.args[4]]), [['101', `linux-packages-${VERSION}`], ['202', `windows-packages-${VERSION}`]]);
  const firstFour = f.calls().filter((call) => call.kind === 'gh').slice(0, 4);
  assert.deepEqual(firstFour.map((call) => call.args[1]), ['list', 'list', 'download', 'download']);
  assert.equal(f.git('rev-parse', 'HEAD'), f.sourceSha);
  assert.equal(f.git('status', '--porcelain', '--untracked-files=normal'), '');
  assertNoExternalMutation(f);
}
function seedLocalAssets(f) {
  for (const path of localPaths) {
    const bytes = path.endsWith('.zip') ? tinyZip('local-fixture.txt', 'synthetic local ZIP\n') : Buffer.from(`synthetic local ${path}\n`);
    put(join(f.repo, path), bytes);
    let row = `asset\t${path}\t${sha256(bytes)}\t${bytes.length}\n`;
    if (!path.endsWith('.yml')) row += `computer_use\t${path}\t{"fixture_only":true}\n`;
    writeFileSync(f.manifest, readFileSync(f.manifest, 'utf8') + row);
  }
}

const options = { timeout: 120_000, concurrency: false };

test('adopt-ci selects matching successful runs and records exact copied bytes once', options, (t) => {
  const f = fixture(t);
  expectSuccess(f.adopt()); assertAdoptedBytes(f);
  const attestations = jsonLines(join(f.control, 'attestations.jsonl'));
  assert.deepEqual(attestations.map((entry) => entry.target), ['linux-x64', 'linux-x64', 'linux-x64', 'linux-x64', 'win32-x64']);
  assert.ok(attestations.every((entry) => !entry.path.endsWith('.exe')));
  expectSuccess(f.adopt()); assertAdoptedBytes(f);
  seedLocalAssets(f);
  expectSuccess(f.helper('verify_release_provenance'));
  assert.equal(f.rows('asset').length, 11);
  assert.equal(f.rows('computer_use').length, 9);
  assertNoExternalMutation(f);
});

for (const [name, change, refusal] of [
  ['missing provenance', (f) => rmSync(f.manifest), /release provenance missing/],
  ['corrupt schema', (f) => put(f.manifest, readFileSync(f.manifest, 'utf8').replace('schema\t1\n', 'schema\t2\n')), /unsupported or corrupt release provenance/],
  ['manifest version mismatch', (f) => put(f.manifest, readFileSync(f.manifest, 'utf8').replace(`version\t${VERSION}\n`, 'version\t9.9.8\n')), /release provenance version does not match 9\.9\.9/],
  ['Cargo version mismatch', (f) => put(join(f.repo, 'Cargo.toml'), '[workspace.package]\nversion = "9.9.10"\n'), /Cargo\.toml version 9\.9\.10 does not match release 9\.9\.9/],
  ['changed source HEAD', (f) => { put(join(f.repo, 'later-source.txt'), 'synthetic source change\n'); f.git('add', 'later-source.txt'); f.git('commit', '-q', '-m', 'local fixture changed'); }, /HEAD changed after release builds started/],
  ['dirty source tree', (f) => put(join(f.repo, 'uncommitted-source.txt'), 'synthetic dirty source\n'), /release source tree is not clean/],
]) {
  test(`adopt-ci refuses ${name} before querying GitHub`, options, (t) => {
    const f = fixture(t); change(f); expectRefusal(f.adopt(), refusal);
    assert.deepEqual(f.calls(), []);
    assertNoExternalMutation(f);
  });
}

test('adopt-ci requires an explicit version without resolving a bump keyword', options, (t) => {
  const f = fixture(t); expectRefusal(f.adopt('patch'), /only valid for 'bump' and 'all'/);
  assert.deepEqual(f.calls(), []); assertNoDownloads(f);
});

for (const [name, run, refusal] of [
  ['missing Windows run', null, /no windows-gui-packages\.yml run.*Dispatch it and wait/s],
  ['running Windows run', { databaseId: 203, status: 'in_progress', conclusion: null }, /still in_progress.*do NOT dispatch another/s],
  ['failed Windows run', { databaseId: 204, status: 'completed', conclusion: 'failure' }, /conclusion 'failure'.*Fix the cause.*gh run view 204 --log-failed/s],
]) {
  test(`adopt-ci distinguishes a ${name} and downloads nothing`, options, (t) => {
    const f = fixture(t); const before = readFileSync(f.manifest);
    f.config.runs[WORKFLOWS[1]] = run ? [{ ...run, headSha: f.sourceSha }] : [];
    f.saveConfig(); expectRefusal(f.adopt(), refusal);
    assert.deepEqual(readFileSync(f.manifest), before); assertNoDownloads(f);
  });
}

test('adopt-ci preserves a GitHub query error instead of calling it a missing run', options, (t) => {
  const f = fixture(t); f.config.queryError = WORKFLOWS[1]; f.saveConfig();
  const result = f.adopt(); expectRefusal(result, /could not query GitHub.*not a missing run/s);
  assert.match(result.stderr, /synthetic gh query failure/); assertNoDownloads(f);
});

test('adopt-ci removes its owned staging directory when a download fails', options, (t) => {
  const f = fixture(t); f.config.downloadFailure = 'windows'; f.saveConfig();
  expectRefusal(f.adopt(), /could not download windows-packages-9\.9\.9 from run 202/);
  assert.equal(f.downloads().length, 2); assert.deepEqual(f.rows('asset'), []);
  assert.deepEqual(f.rows('ci_run'), []); assertNoExternalMutation(f);
});

for (const mode of ['missing', 'another version']) {
  test(`adopt-ci refuses a Windows ZIP ${mode} from the exact artifact basename`, options, (t) => {
    const f = fixture(t); const entry = f.payloads.get(adoptedPaths[4]);
    rmSync(entry.path);
    if (mode === 'another version') put(join(dirname(entry.path), 'Biorouter-win32-x64-9.9.8.zip'), entry.bytes);
    expectRefusal(f.adopt(), /run artifact did not contain Biorouter-win32-x64-9\.9\.9\.zip/);
    assert.deepEqual(f.rows('ci_run'), []); assertNoExternalMutation(f);
  });
}

test('adopt-ci refuses a malformed ZIP at the stubbed attestation boundary and cleans staging', options, (t) => {
  const f = fixture(t); put(f.payloads.get(adoptedPaths[4]).path, 'synthetic non-ZIP bytes\n');
  expectRefusal(f.adopt(), /synthetic archive verifier: invalid ZIP/);
  assert.deepEqual(f.rows('ci_run'), []); assertNoExternalMutation(f);
});

test('adopt-ci provenance verification rejects copied ZIP bytes changed after adoption', options, (t) => {
  const f = fixture(t); expectSuccess(f.adopt()); seedLocalAssets(f);
  expectSuccess(f.helper('verify_release_provenance'));
  const zip = join(f.repo, adoptedPaths[4]);
  writeFileSync(zip, Buffer.concat([readFileSync(zip), Buffer.from('synthetic later bytes\n')]));
  expectRefusal(f.helper('verify_release_provenance'), /release artifact changed after build: .*Biorouter-win32-x64-9\.9\.9\.zip/);
  assertNoExternalMutation(f);
});

for (const [name, change, refusal] of [
  ['workflow identity', (f) => { f.config.api[202].path = `.github/workflows/${WORKFLOWS[0]}`; }, /run 202 for windows-gui-packages\.yml, but that run is of/],
  ['source SHA', (f) => { f.config.api[202].head_sha = FOREIGN_SHA; }, /run 202 built a{40}, not this release's source/],
  ['successful completion', (f) => { f.config.api[202].conclusion = 'failure'; }, /run 202 is completed\/failure, not completed\/success/],
]) {
  test(`adopt-ci recorded run verification checks ${name}`, options, (t) => {
    const f = fixture(t); expectSuccess(f.adopt()); change(f); f.saveConfig();
    expectRefusal(f.helper('verify_release_ci_runs'), refusal); assertNoExternalMutation(f);
  });
}

for (const [name, change, count] of [
  ['missing', (rows) => rows.filter((row) => !row.startsWith(`ci_run\t${WORKFLOWS[1]}\t`)), 0],
  ['duplicate', (rows) => [...rows, `ci_run\t${WORKFLOWS[1]}\t202`], 2],
]) {
  test(`adopt-ci recorded run verification refuses a ${name} workflow row`, options, (t) => {
    const f = fixture(t); expectSuccess(f.adopt()); put(f.manifest, `${change(lines(f.manifest)).join('\n')}\n`);
    expectRefusal(f.helper('verify_release_ci_runs'), new RegExp(`release provenance names ${count} windows-gui-packages\\.yml runs`));
    assertNoExternalMutation(f);
  });
}
