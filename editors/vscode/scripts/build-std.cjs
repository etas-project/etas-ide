const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
function run(args, options = {}) {
    const result = spawnSync('cargo', args, { cwd: root, stdio: 'inherit', ...options });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`cargo failed: ${result.signal ?? result.status}`);
    return result;
}

const sysroot = process.env.ETAS_STD_SYSROOT;
if (!sysroot) throw new Error('Set ETAS_STD_SYSROOT to the output SDK directory.');
const metadata = JSON.parse(run([
    'metadata', '--locked', '--format-version', '1', '--manifest-path', '../../Cargo.toml',
], { stdio: ['inherit', 'pipe', 'inherit'], encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).stdout);
const driver = metadata.packages.find(pkg => pkg.name === 'etas_driver');
const standard = metadata.packages.find(pkg => pkg.name === 'etas_std');
const main = path.resolve(path.dirname(driver.manifest_path), '../..', 'Cargo.toml');
const source = path.join(path.dirname(standard.manifest_path), 'etas.toml');
fs.mkdirSync(path.resolve(sysroot), { recursive: true });
run([
    'run', '--locked', '--release', '--manifest-path', main,
    '--target-dir', metadata.target_directory, '-p', 'etas_cli', '--features', 'forbid-legacy-std',
    '--', '--no-config', '--cache', 'off', 'std', 'build',
    '--manifest-path', source, '--out-dir', path.resolve(sysroot, 'std'),
]);
