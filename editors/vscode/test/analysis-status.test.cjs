const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { test } = require('node:test');
const { transformSync } = require('esbuild');

const file = path.resolve(__dirname, '../src/analysis-status.ts');
const compiled = new Module(file, module);
compiled._compile(transformSync(fs.readFileSync(file, 'utf8'), { loader: 'ts', format: 'cjs' }).code, file);
const { idleStatus, statusView, duration } = compiled.exports;
const build = {
    server: '0.3.4', frontend: { version: '0.1.0', revision: '21c1cd88abcdef' },
    core: { version: '0.1.0', revision: '4946c9f9abcdef' }, driver: { version: '0.1.0', revision: 'b7e20625abcdef' },
};
const view = (status, connection = 'running', info = build) => statusView({ ...idleStatus, ...status }, connection, info, '0.3.4', '/server/etas-lsp');

test('analysis status distinguishes actual worker stages from startup and disconnection', () => {
    for (const [phase, label] of [['queued', 'Waiting'], ['preparing', 'Loading'], ['checking', 'Analyzing'], ['failed', 'Unavailable']]) {
        assert.ok(view({ phase }).text.includes(label));
    }
    assert.ok(view({}, 'starting').text.includes('Starting'));
    assert.equal(view({}, 'stopped').failed, true);
    assert.ok(!view({}).text.includes('check)'));
    assert.ok(view({ phase: 'ready', elapsedMs: 0 }).text.includes('0 ms'));
});

test('diagnostic counts and completed durations are never confused with server failures', () => {
    assert.ok(view({ phase: 'ready', errors: 2 }).text.includes('$(error) Etas: 2'));
    assert.equal(view({ phase: 'ready', errors: 2 }).failed, false);
    assert.ok(view({ phase: 'ready', warnings: 3 }).text.includes('$(warning) Etas: 3'));
    assert.equal(duration(42), '42 ms');
    assert.equal(duration(1234), '1.2 s');
});

test('tooltip identifies the project and Cargo-locked producers, not an assumed CLI version', () => {
    const state = view({ phase: 'failed', projectRoot: '/project/nomo', files: 95, errors: 1, message: 'Invalid manifest' });
    for (const expected of ['/project/nomo', '95', '21c1cd88', '4946c9f9', 'b7e20625', '0.3.4', 'Invalid manifest', '/server/etas-lsp']) {
        assert.ok(state.tooltip.includes(expected), expected);
    }
    const unknown = statusView(idleStatus, 'running', undefined, '0.3.4', '/custom/server');
    assert.ok(unknown.tooltip.includes('Frontend: Not reported'));
    assert.ok(!unknown.tooltip.includes('21c1cd88'));
});
