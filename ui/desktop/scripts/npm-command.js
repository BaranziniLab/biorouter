const fs = require('node:fs');
const path = require('node:path');

function npmCommand(args, { platform = process.platform, execPath = process.execPath, env = process.env } = {}) {
  if (env.npm_execpath && /\.[cm]?js$/.test(env.npm_execpath)) {
    return [execPath, [env.npm_execpath, ...args]];
  }
  if (platform !== 'win32') return ['npm', args];

  const directories = [path.dirname(execPath), ...(env.PATH || env.Path || '').split(';')];
  const npmCli = directories.filter(Boolean)
    .map((directory) => path.join(directory, 'node_modules/npm/bin/npm-cli.js'))
    .find((candidate) => fs.existsSync(candidate));
  if (!npmCli) throw new Error('Cannot find npm-cli.js; install npm alongside Node or invoke packaging through npm.');
  return [execPath, [npmCli, ...args]];
}

module.exports = { npmCommand };
