const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
function run(command, args) {
    const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`${command} failed: ${result.signal ?? result.status}`);
}

run('cargo', ['build', '-p', 'etas_lsp', '--locked', '--release', '--manifest-path', '../../Cargo.toml']);
const directory = path.join(root, 'server');
fs.mkdirSync(directory, { recursive: true });
const temporary = path.join(directory, `.etas-lsp-${process.pid}`);
try {
    fs.copyFileSync(path.join(root, '../../target/release/etas-lsp'), temporary);
    run('strip', [temporary]);
    // Existing development servers keep their executable inode while the
    // completed, signed replacement becomes available to new sessions.
    fs.renameSync(temporary, path.join(directory, 'etas-lsp'));
} finally {
    fs.rmSync(temporary, { force: true });
}
