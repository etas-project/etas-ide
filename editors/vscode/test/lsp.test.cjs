const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { test } = require('node:test');
const { LspClient } = require('./lsp-client.cjs');

function decodeTokens(data, legend, text) {
    const lines = text.split('\n');
    let line = 0, col = 0, previousEnd = 0;
    const result = [];
    assert.equal(data.length % 5, 0);
    for (let i = 0; i < data.length; i += 5) {
        const delta = data[i];
        line += delta;
        col = delta ? data[i + 1] : col + data[i + 1];
        if (delta) previousEnd = 0;
        assert.ok(line < lines.length && col >= previousEnd && data[i + 2] > 0);
        previousEnd = col + data[i + 2];
        assert.ok(previousEnd <= lines[line].replace(/\r$/, '').length);
        assert.ok(data[i + 3] < legend.tokenTypes.length);
        result.push({ text: lines[line].slice(col, previousEnd), kind: legend.tokenTypes[data[i + 3]] });
    }
    return result;
}

test('stdio: parse diagnostics, outline, UTF-16 tokens, edits, and recovery', { timeout: 120000 }, async t => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-lsp-e2e-'));
    const client = new LspClient();
    t.after(async () => { await client.stop(); fs.rmSync(root, { recursive: true, force: true }); });
    const initialized = await client.initialize(pathToFileURL(root).href);
    assert.equal(initialized.capabilities.positionEncoding, 'utf-16');
    assert.equal(initialized.capabilities.documentSymbolProvider, true);
    const legend = initialized.capabilities.semanticTokensProvider.legend;
    const uri = pathToFileURL(path.join(root, 'main.es')).href;
    const source = 'module probe;\nflow main() -> i32 { let text = "中文😀"; return 0; }\n';
    client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text: source } });
    assert.deepEqual((await client.diagnostics(uri, 1)).filter(diag => diag.severity === 1), []);
    const symbols = await client.request('textDocument/documentSymbol', { textDocument: { uri } });
    assert.equal(symbols[0].name, 'probe');
    const main = symbols[0].children.find(symbol => symbol.name === 'main');
    assert.equal(main.selectionRange.start.line, 1);
    assert.equal(main.selectionRange.start.character, 5);
    const tokens = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
    const decoded = decodeTokens(tokens.data, legend, source);
    assert.ok(decoded.some(token => token.text === 'return' && token.kind === 'keyword'));
    assert.ok(decoded.some(token => token.text === 'main' && token.kind === 'function'));
    assert.ok(decoded.some(token => token.text === '"中文😀"' && token.kind === 'string'));

    const broken = source.replace('return 0;', 'let broken = ; return 0;');
    client.notify('textDocument/didChange', { textDocument: { uri, version: 2 }, contentChanges: [{ text: broken }] });
    const diagnostics = await client.diagnostics(uri, 2);
    assert.ok(diagnostics.some(diag => /Syntax|syntax/.test(diag.code)), JSON.stringify(diagnostics));
    for (const diagnostic of diagnostics) {
        assert.ok(diagnostic.range.start.line < broken.split('\n').length);
        assert.ok(diagnostic.range.end.line >= diagnostic.range.start.line);
    }
    const brokenTokens = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
    assert.ok(decodeTokens(brokenTokens.data, legend, broken).some(token => token.text === 'return'));
    client.notify('textDocument/didChange', { textDocument: { uri, version: 3 }, contentChanges: [{ text: source }] });
    assert.deepEqual((await client.diagnostics(uri, 3)).filter(diag => diag.severity === 1), []);
    const start = source.split('\n')[1].indexOf('0;');
    client.notify('textDocument/didChange', { textDocument: { uri, version: 4 }, contentChanges: [{
        range: { start: { line: 1, character: start }, end: { line: 1, character: start + 1 } }, text: '42',
    }] });
    assert.deepEqual((await client.diagnostics(uri, 4)).filter(diag => diag.severity === 1), []);
    client.notify('textDocument/didClose', { textDocument: { uri } });
});

test('package preparation failures are published and recover after manifest changes', { timeout: 120000 }, async t => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-lsp-package-'));
    fs.mkdirSync(path.join(root, 'src'));
    const file = path.join(root, 'src/main.es');
    const text = 'module main;\nflow main() -> i32 { return 0; }\n';
    fs.writeFileSync(file, text);
    const manifest = path.join(root, 'etas.toml');
    fs.writeFileSync(manifest, '[package]\ninvalid = true\n');
    const client = new LspClient();
    t.after(async () => { await client.stop(); fs.rmSync(root, { recursive: true, force: true }); });
    await client.initialize(pathToFileURL(root).href);
    const uri = pathToFileURL(file).href;
    client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
    const diagnostics = await client.diagnostics(uri, 1);
    assert.ok(diagnostics.some(diag => diag.code === 'package::PrepareFailed'));
    const symbols = await client.request('textDocument/documentSymbol', { textDocument: { uri } });
    assert.equal(symbols[0].children[0].name, 'main');
    const tokens = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
    assert.ok(tokens.data.length > 0);
    client.notifications = [];
    fs.unlinkSync(manifest);
    client.notify('workspace/didChangeWatchedFiles', { changes: [{ uri: pathToFileURL(manifest).href, type: 3 }] });
    const recovered = await client.diagnostics(uri, 1);
    assert.ok(!recovered.some(diag => diag.code === 'package::PrepareFailed'), JSON.stringify(recovered));
});

const project = process.env.ETAS_E2E_PROJECT;
test('stdio: real project parser results, diagnostics, and highlighting', { skip: !project, timeout: 240000 }, async t => {
    const client = new LspClient();
    t.after(() => client.stop());
    const initialized = await client.initialize(pathToFileURL(project).href);
    let count = 0;
    let diagnostics = 0;
    for (const name of fs.readdirSync(path.join(project, 'src'), { recursive: true })) {
        if (!name.endsWith('.es')) continue;
        const file = path.join(project, 'src', name);
        const uri = pathToFileURL(file).href;
        const text = fs.readFileSync(file, 'utf8');
        client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
        const symbols = await client.request('textDocument/documentSymbol', { textDocument: { uri } });
        assert.ok(symbols.length > 0, name + ': missing parser outline');
        const result = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
        const tokens = decodeTokens(result.data, initialized.capabilities.semanticTokensProvider.legend, text);
        assert.ok(tokens.some(token => token.text === 'module' && token.kind === 'keyword'), name);
        const published = await client.diagnostics(uri, 1);
        for (const diagnostic of published) {
            assert.ok(diagnostic.range.start.line < text.split('\n').length, name);
            if (diagnostic.severity === 1) diagnostics++;
        }
        count++;
    }
    assert.ok(count > 0);
    console.log('LSP: ' + count + ' real project files verified; error diagnostic count: ' + diagnostics);
    assert.equal(diagnostics, 0, 'real project should have no compiler errors');
});
