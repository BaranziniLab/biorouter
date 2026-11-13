#!/usr/bin/env node
// Stages the pinned esbuild for the test runner and exports its path to the
// later steps of the same CI job as BIOROUTER_ESBUILD_BIN.
//
// The agent_drafter bundle tests and the offline integration binaries compile
// app examples with it; without a staged copy the bundler falls back to
// `npx --yes esbuild`, a download from the npm registry, which the offline
// integration step cannot reach. Every Rust test job in rust.yml runs this, so
// it lives in one file rather than in a heredoc copied into each job.
//
// The binary goes to RUNNER_TEMP, never to ui/desktop/src/bin/, which belongs
// to the release pipeline.
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { fetchEsbuild, ESBUILD_VERSION } = require(
  path.join(__dirname, '..', '..', 'ui', 'desktop', 'scripts', 'fetch-esbuild.js'),
);

const runnerTemp = process.env.RUNNER_TEMP;
const githubEnv = process.env.GITHUB_ENV;
if (!runnerTemp || !githubEnv) {
  throw new Error('stage-esbuild.cjs runs inside a GitHub Actions job (RUNNER_TEMP and GITHUB_ENV)');
}

const compiler = path.resolve(
  fetchEsbuild(process.platform, process.arch, {
    binDir: path.join(runnerTemp, 'biorouter-test-tools'),
    cacheDir: path.join(runnerTemp, 'biorouter-esbuild-cache'),
  }),
);
const version = execFileSync(compiler, ['--version'], {
  encoding: 'utf8',
  timeout: 5000,
  maxBuffer: 64,
}).trim();
if (version !== ESBUILD_VERSION) {
  throw new Error('Native esbuild does not match the pinned release');
}
fs.appendFileSync(githubEnv, `BIOROUTER_ESBUILD_BIN=${compiler}\n`);
