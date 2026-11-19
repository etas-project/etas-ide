const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
function run(command, args, options = {}) {
    const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', ...options });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`${command} failed: ${result.signal ?? result.status}`);
    return result;
}

const sysroot = process.env.ETAS_STD_SYSROOT;
if (!sysroot) throw new Error('Set ETAS_STD_SYSROOT to the SDK containing std/manifest.json.');
const std = path.resolve(sysroot, 'std');
const manifest = JSON.parse(fs.readFileSync(path.join(std, 'manifest.json'), 'utf8'));
const toolchain = manifest.toolchain_id;
if (typeof toolchain !== 'string' || !toolchain || toolchain === '.' || toolchain === '..' || /[/\\]/.test(toolchain)) {
    throw new Error('The SDK manifest toolchain_id must be a directory name.');
}
const build = run('cargo', [
    'build', '-p', 'etas_lsp', '--locked', '--release', '--manifest-path', '../../Cargo.toml',
    '--message-format=json-render-diagnostics',
], { stdio: ['inherit', 'pipe', 'inherit'], encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
const artifact = build.stdout.trim().split('\n').map(line => JSON.parse(line)).find(message =>
    message.reason === 'compiler-artifact' && message.target.name === 'etas-lsp' && message.executable);
if (!artifact) throw new Error('Cargo did not report an etas-lsp executable.');
const directory = path.join(root, 'server');
fs.mkdirSync(directory, { recursive: true });
const temporary = path.join(directory, `.etas-lsp-${process.pid}`);
try {
    fs.copyFileSync(artifact.executable, temporary);
    run('strip', [temporary]);
    // Existing development servers keep their executable inode during replacement.
    fs.renameSync(temporary, path.join(directory, 'etas-lsp'));
} finally {
    fs.rmSync(temporary, { force: true });
}
const lib = path.join(root, 'lib');
const installed = path.join(lib, 'etas');
fs.mkdirSync(lib, { recursive: true });
const staging = fs.mkdtempSync(path.join(directory, '.sdk-'));
try {
    fs.cpSync(std, path.join(staging, toolchain, 'std'), { recursive: true });
    fs.rmSync(installed, { recursive: true, force: true });
    fs.renameSync(staging, installed);
} finally {
    fs.rmSync(staging, { recursive: true, force: true });
}
