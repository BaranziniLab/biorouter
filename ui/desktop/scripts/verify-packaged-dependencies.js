const fs = require('fs');
const path = require('path');
const asar = require('@electron/asar');

function validateArchiveLinks(entries) {
  const nodes = new Map(entries.map(([name, metadata]) => [name.replace(/\\/g, '/').replace(/^\//, ''), metadata]));
  for (const [name, metadata] of nodes) {
    if (!metadata.link) continue;
    let current = metadata;
    const visited = new Set([name]);
    for (let depth = 0; current.link; depth++) {
      const link = current.link.replace(/\\/g, '/');
      const target = path.posix.normalize(link);
      if (path.posix.isAbsolute(link) || path.win32.isAbsolute(link) || /^[a-z]:/i.test(link) ||
          target === '..' || target.startsWith('../')) {
        throw new Error(`Packaged dependency link escapes the application: ${name}`);
      }
      if (depth >= 64 || visited.has(target)) throw new Error(`Packaged dependency link cycle: ${name}`);
      visited.add(target);
      current = nodes.get(target);
      if (!current) throw new Error(`Packaged dependency link has no bundled target: ${name} -> ${target}`);
    }
  }
}

function verifyPackagedDependencies(resources, platform, arch) {
  const archive = path.join(resources, 'app.asar');
  validateArchiveLinks(asar.listPackage(archive).map((entry) => {
    const name = path.normalize(entry.replace(/\\/g, '/').replace(/^\//, ''));
    return [entry, asar.statFile(archive, name, false)];
  }));
  asar.statFile(archive, path.normalize('node_modules/node-pty/lib/index.js'));
  const unpacked = path.join(resources, 'app.asar.unpacked/node_modules/node-pty');
  const nativeDirectories = [
    path.join(unpacked, 'prebuilds', `${platform}-${arch}`),
    path.join(unpacked, 'build/Release'),
  ];
  const native = nativeDirectories.find((directory) => fs.existsSync(path.join(directory, 'pty.node')));
  if (!native) throw new Error(`Packaged node-pty native module missing for ${platform}-${arch}`);
  if (platform === 'darwin') fs.accessSync(path.join(native, 'spawn-helper'), fs.constants.X_OK);
}

module.exports = { verifyPackagedDependencies, validateArchiveLinks };
