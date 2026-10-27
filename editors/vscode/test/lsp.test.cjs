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
        result.push({ text: lines[line].slice(col, previousEnd), kind: legend.tokenTypes[data[i + 3]], line, character: col });
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

test('semantic tokens: std aliases, imported types/modules, wildcard calls, and shadowing', { timeout: 120000 }, async t => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-import-token-e2e-'));
    const client = new LspClient();
    t.after(async () => { await client.stop(); fs.rmSync(root, { recursive: true, force: true }); });
    const initialized = await client.initialize(pathToFileURL(root).href);
    const legend = initialized.capabilities.semanticTokensProvider.legend;
    const uri = pathToFileURL(path.join(root, 'main.es')).href;
    const text = 'module tokens;\nimport std.collections.len as count;\nimport std.collections.Array as Seq;\nimport std.text as text_ops;\nimport std.text.*;\nalias Row<T> = { value: T, count: i32 };\nflow parse(args: Array<string>, seq: Seq<string>) -> usize {\n    let text = "中文😀"; let n = count(args);\n    if true { let count = n; let value = count; }\n    let length = len(text);\n    return count(args);\n}\nflow main() -> i32 { return 0; }\n';
    client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
    assert.deepEqual((await client.diagnostics(uri, 1)).filter(diag => diag.severity === 1), []);
    const response = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
    const tokens = decodeTokens(response.data, legend, text);
    assert.deepEqual(tokens.filter(token => token.text === 'count').map(token => token.kind),
        ['function', 'property', 'function', 'variable', 'variable', 'function']);
    assert.deepEqual(tokens.filter(token => token.text === 'Seq').map(token => token.kind), ['type', 'type']);
    assert.deepEqual(tokens.filter(token => token.text === 'text_ops').map(token => token.kind), ['namespace']);
    assert.ok(tokens.some(token => token.text === 'len' && token.kind === 'function'));
    assert.ok(tokens.some(token => token.text === 'T' && token.kind === 'typeParameter'));
    assert.ok(tokens.some(token => token.text === '"中文😀"' && token.kind === 'string'));
});

test('member calls: arrays, chains, name collisions, UTF-16, invalid edits, and recovery', { timeout: 120000 }, async t => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-method-token-e2e-'));
    const client = new LspClient();
    t.after(async () => { await client.stop(); fs.rmSync(root, { recursive: true, force: true }); });
    const initialized = await client.initialize(pathToFileURL(root).href);
    const legend = initialized.capabilities.semanticTokensProvider.legend;
    const uri = pathToFileURL(path.join(root, 'main.es')).href;
    const text = 'module methods;\nalias Holder = { push: i32 };\nflow append(values: Array<string>, flag: string) -> Array<string> {\n    let push = values; let unicode = "中文😀 push"; let next = push.push(flag).push(flag);\n    let literal = ["one"].push(flag); let grouped = (values).push(flag);\n    let size = values.len(); let trimmed = flag.trim(); return next;\n}\nflow field(holder: Holder) -> i32 { return holder.push; }\nflow main() -> i32 { return 0; }\n';
    client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
    assert.deepEqual((await client.diagnostics(uri, 1)).filter(diag => diag.severity === 1), []);
    async function decoded(source) {
        const result = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
        return decodeTokens(result.data, legend, source);
    }
    const tokens = await decoded(text);
    for (const match of text.matchAll(/\.(push|len|trim)\(/g)) {
        const lines = text.slice(0, match.index + 1).split('\n');
        const token = tokens.find(token => token.line === lines.length - 1 && token.character === lines.at(-1).length);
        assert.equal(token?.text, match[1]);
        assert.equal(token.kind, 'function');
    }
    assert.deepEqual(tokens.filter(token => token.text === 'push').map(token => token.kind),
        ['property', 'variable', 'variable', 'function', 'function', 'function', 'function']);
    const methodLines = text.slice(0, text.indexOf('.push(') + 1).split('\n');
    const methodHover = await client.request('textDocument/hover', { textDocument: { uri }, position: {
        line: methodLines.length - 1, character: methodLines.at(-1).length,
    } });
    assert.equal(methodHover, null, 'missing selected method facts must not show the receiver signature');
    const fieldLines = text.slice(0, text.indexOf('holder.push;') + 'holder.'.length).split('\n');
    const fieldHover = await client.request('textDocument/hover', { textDocument: { uri }, position: {
        line: fieldLines.length - 1, character: fieldLines.at(-1).length,
    } });
    assert.ok(fieldHover?.contents.value.includes('push: i32'), JSON.stringify(fieldHover));
    for (const [index, changed] of [text.replace('.push(flag)', '.missing(flag)'), text.replace('.push(flag)', '.push(42)')].entries()) {
        client.notify('textDocument/didChange', { textDocument: { uri, version: index + 2 }, contentChanges: [{ text: changed }] });
        assert.ok((await client.diagnostics(uri, index + 2)).some(diag => diag.severity === 1));
        const badTokens = await decoded(changed);
        assert.ok(!badTokens.some(token => token.kind === 'function' && token.text === 'missing'));
        assert.ok(!badTokens.some(token => token.kind === 'function' && token.text === 'push'),
            'a mismatched last-good snapshot must not paint method names');
    }
    client.notify('textDocument/didChange', { textDocument: { uri, version: 4 }, contentChanges: [{ text }] });
    assert.deepEqual((await client.diagnostics(uri, 4)).filter(diag => diag.severity === 1), []);
    assert.equal((await decoded(text)).filter(token => token.text === 'push' && token.kind === 'function').length, 4);
});

test('record constructors: type names, labels, values, hover, and UTF-16 ranges', { timeout: 120000 }, async t => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-record-token-e2e-'));
    const client = new LspClient();
    t.after(async () => { await client.stop(); fs.rmSync(root, { recursive: true, force: true }); });
    const initialized = await client.initialize(pathToFileURL(root).href);
    const legend = initialized.capabilities.semanticTokensProvider.legend;
    const uri = pathToFileURL(path.join(root, 'main.es')).href;
    const text = 'module records;\nimport std.text.split;\ntype Identity = { id: string, workspace: string, allowed: Array<string> };\ntype Envelope = { identity: Identity };\nflow make(id: string, workspace: string, allow: string) -> Envelope {\n    let unicode = "中文😀"; let identity = Identity { id = id, workspace = workspace, allowed = split(allow, ",") };\n    return Envelope { identity = identity };\n}\nflow main() -> i32 { return 0; }\n';
    client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
    assert.deepEqual((await client.diagnostics(uri, 1)).filter(diag => diag.severity === 1), []);
    const response = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
    const tokens = decodeTokens(response.data, legend, text);
    function at(needle, kind) {
        const offset = text.indexOf(needle);
        assert.ok(offset >= 0);
        const lines = text.slice(0, offset).split('\n');
        const position = { line: lines.length - 1, character: lines.at(-1).length };
        const token = tokens.find(token => token.line === position.line && token.character === position.character);
        assert.equal(token?.kind, kind, needle + ': ' + JSON.stringify(token));
        return position;
    }
    const constructor = at('Identity {', 'type');
    at('Envelope {', 'type');
    at('id = id', 'property');
    at('id, workspace', 'parameter');
    at('identity = identity', 'property');
    at('identity };', 'variable');
    at('split(allow', 'function');
    const hover = await client.request('textDocument/hover', { textDocument: { uri }, position: constructor });
    assert.ok(hover?.contents.value.includes('type Identity'), JSON.stringify(hover));
    assert.deepEqual(hover.range.start, constructor);
    assert.equal(hover.range.end.character - hover.range.start.character, 'Identity'.length);
});

test('hover: readable checked signatures, types, effects, imports, and UTF-16 ranges', { timeout: 120000 }, async t => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-hover-e2e-'));
    const src = path.join(root, 'src');
    fs.mkdirSync(src);
    const client = new LspClient();
    t.after(async () => { await client.stop(); fs.rmSync(root, { recursive: true, force: true }); });
    const helper = 'module helper;\npublic flow square(x: i32) -> i32 ![] { return x * x; }\n';
    fs.writeFileSync(path.join(src, 'helper.es'), helper);
    const text = 'module main;\nimport helper.*;\nimport std.text.{trim};\nspec Region;\ntype Root;\nimpl Root ~ Region;\neffect Store extends FileIO { action read<R ~ Region>(key: string) -> string; }\nflow fetch<R ~ Region>(key: string) -> string ![Store.read<R>] { return perform Store.read<R>(key); }\nflow main() -> i32 { let text = "中文😀"; let label = trim(text); let answer = square(4); return answer; }\n';
    const uri = pathToFileURL(path.join(src, 'main.es')).href;
    fs.writeFileSync(path.join(src, 'main.es'), text);
    await client.initialize(pathToFileURL(root).href);
    client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
    assert.deepEqual((await client.diagnostics(uri, 1)).filter(diag => diag.severity === 1), []);
    async function hover(needle, expected) {
        const offset = text.indexOf(needle);
        assert.ok(offset >= 0);
        const lines = text.slice(0, offset).split('\n');
        const position = { line: lines.length - 1, character: lines.at(-1).length };
        const result = await client.request('textDocument/hover', { textDocument: { uri }, position });
        assert.equal(result.contents.kind, 'markdown');
        assert.ok(result.contents.value.includes(expected), result.contents.value);
        assert.doesNotMatch(result.contents.value, /TypeId\(|SymbolId\(|FlowType|Function\(/);
        assert.deepEqual(result.range.start, position);
        return result;
    }
    const square = await hover('square(4)', 'flow square(x: i32) -> i32 ![]');
    assert.equal(square.range.end.character - square.range.start.character, 'square'.length);
    await hover('answer;', 'let answer: i32');
    await hover('trim(text)', 'flow trim(string) -> string');
    const fetch = await hover('fetch<R', 'flow fetch<R ~ Region>(key: string) -> string ![Store.read<R>]');
    assert.ok(fetch.contents.value.includes('Requested actions:'), fetch.contents.value);
    assert.equal(fetch.contents.value.match(/Store\.read<R>/g).length, 2);
    const broken = text.replace('return answer;', 'return missing;');
    client.notify('textDocument/didChange', { textDocument: { uri, version: 2 }, contentChanges: [{ text: broken }] });
    assert.ok((await client.diagnostics(uri, 2)).some(diag => diag.severity === 1));
    const lines = broken.slice(0, broken.indexOf('missing;')).split('\n');
    assert.equal(await client.request('textDocument/hover', { textDocument: { uri }, position: {
        line: lines.length - 1, character: lines.at(-1).length,
    } }), null, 'invalid current identifiers must not receive last-good hover');
});

const project = process.env.ETAS_E2E_PROJECT;
test('stdio: real project parser results, diagnostics, and highlighting', { skip: !project, timeout: 240000 }, async t => {
    const client = new LspClient();
    t.after(() => client.stop());
    const initialized = await client.initialize(pathToFileURL(project).href);
    let count = 0;
    let diagnostics = 0;
    let hovers = 0;
    for (const name of fs.readdirSync(path.join(project, 'src'), { recursive: true })) {
        if (!name.endsWith('.es')) continue;
        const file = path.join(project, 'src', name);
        const uri = pathToFileURL(file).href;
        const text = fs.readFileSync(file, 'utf8');
        client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
        const symbols = await client.request('textDocument/documentSymbol', { textDocument: { uri } });
        assert.ok(symbols.length > 0, name + ': missing parser outline');
        const published = await client.diagnostics(uri, 1);
        const result = await client.request('textDocument/semanticTokens/full', { textDocument: { uri } });
        const tokens = decodeTokens(result.data, initialized.capabilities.semanticTokensProvider.legend, text);
        assert.ok(tokens.some(token => token.text === 'module' && token.kind === 'keyword'), name);
        for (const match of text.matchAll(/\.push\(/g)) {
            const lines = text.slice(0, match.index + 1).split('\n');
            const token = tokens.find(token => token.line === lines.length - 1 && token.character === lines.at(-1).length);
            assert.equal(token?.text, 'push', name + ': missing array method token');
            assert.equal(token.kind, 'function', name + ': push must be callable');
        }
        if (name.replaceAll(path.sep, '/') === 'nomo/cli/edit_args.es') {
            const aliases = tokens.filter(token => token.text === 'count');
            assert.ok(aliases.length >= 3, 'edit_args.es must include the alias and its calls');
            assert.ok(aliases.every(token => token.kind === 'function'), JSON.stringify(aliases));
        }
        if (name.replaceAll(path.sep, '/') === 'nomo/main.es') {
            assert.ok(tokens.some(token => token.text === 'resume' && token.kind === 'function'),
                'the real resume flow name must not remain a control keyword');
        }
        if (name.replaceAll(path.sep, '/') === 'nomo/cli/harness_checkpoint_args.es') {
            const offset = text.indexOf('Identity { id = id, workspace = workspace');
            assert.ok(offset >= 0, 'the reported constructor must be present');
            const lines = text.slice(0, offset).split('\n');
            const token = tokens.find(token => token.line === lines.length - 1 && token.character === lines.at(-1).length);
            assert.equal(token?.text, 'Identity');
            assert.equal(token.kind, 'type', 'record constructor Identity must be classified as a type');
        }
        for (const diagnostic of published) {
            assert.ok(diagnostic.range.start.line < text.split('\n').length, name);
            if (diagnostic.severity === 1) diagnostics++;
        }
        const flow = /\bflow\s+([A-Za-z_]\w*)/.exec(text);
        if (flow) {
            const offset = flow.index + flow[0].lastIndexOf(flow[1]);
            const lines = text.slice(0, offset).split('\n');
            const hover = await client.request('textDocument/hover', { textDocument: { uri }, position: {
                line: lines.length - 1, character: lines.at(-1).length,
            } });
            assert.ok(hover?.contents.value.includes('flow ' + flow[1]), name + ': missing checked flow signature');
            assert.doesNotMatch(hover.contents.value, /TypeId\(|SymbolId\(|FlowType|Function\(/, name);
            hovers++;
        }
        count++;
    }
    assert.ok(count > 0);
    console.log('LSP: ' + count + ' real project files, ' + hovers + ' checked hovers; error diagnostic count: ' + diagnostics);
    assert.equal(diagnostics, 0, 'real project should have no compiler errors');
});
