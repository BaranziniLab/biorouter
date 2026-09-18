const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const test = require('node:test');
const { npmCommand } = require('./npm-command');

test('bare Windows packaging runs npm JS through Node with literal arguments', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'npm CLI ü '));
  try {
    const cli = path.join(root, 'node_modules/npm/bin/npm-cli.js');
    fs.mkdirSync(path.dirname(cli), { recursive: true });
    fs.writeFileSync(cli, 'process.stdout.write(JSON.stringify(process.argv.slice(2)))');
    const args = ['run', 'build:web', 'literal space & value'];
    const selectedNode = path.join(root, 'selected-node');
    const [command, commandArgs] = npmCommand(args, {
      platform: 'win32', execPath: selectedNode, env: { PATH: root },
    });
    assert.equal(command, selectedNode);
    const result = spawnSync(process.execPath, commandArgs, { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), args);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('npm-invoked packaging preserves the invoking JS CLI', () => {
  assert.deepEqual(npmCommand(['run', 'build:web'], {
    platform: 'win32', execPath: '/selected/node', env: { npm_execpath: '/selected npm/npm-cli.js' },
  }), ['/selected/node', ['/selected npm/npm-cli.js', 'run', 'build:web']]);
});

test('missing Windows npm fails without falling back to a cmd launcher', () => {
  assert.throws(() => npmCommand([], {
    platform: 'win32', execPath: '/missing-node/npm-test/node', env: {},
  }), /Cannot find npm-cli.js/);
});
