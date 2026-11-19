const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { LspClient } = require('./lsp-client.cjs');

async function checkInstalledExtension(extension) {
    const lib = path.join(extension, 'lib', 'etas');
    const toolchains = fs.readdirSync(lib);
    assert.equal(toolchains.length, 1, 'the extension must package exactly one SDK');
    const definition = path.join(lib, toolchains[0], 'std/library/std/text/assembly.es');
    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-installed-extension-'));
    const client = new LspClient(path.join(extension, 'server', 'etas-lsp'));
    try {
        const lines = [
            'module app;',
            'import std.text.{join as assemble};',
            'flow main() -> string ![] { return assemble(["a", "b"], "|"); }',
        ];
        const text = lines.join('\n') + '\n';
        const app = path.join(workspace, 'app.es');
        fs.writeFileSync(app, text);
        const uri = pathToFileURL(app).href;
        await client.initialize(pathToFileURL(workspace).href, { stdProvider: 'artifact' });
        client.notify('textDocument/didOpen', { textDocument: { uri, languageId: 'etas', version: 1, text } });
        const diagnostics = await client.diagnostics(uri, 1);
        assert.deepEqual(diagnostics.filter(diagnostic => diagnostic.severity === 1), []);
        const locations = await client.request('textDocument/definition', {
            textDocument: { uri }, position: { line: 2, character: lines[2].indexOf('assemble') },
        });
        const target = locations?.find(location => location.uri === pathToFileURL(definition).href);
        assert.ok(target, 'expected the packaged std.text.join source, received ' + JSON.stringify(locations));
        const { start, end } = target.range;
        const source = fs.readFileSync(definition, 'utf8').split('\n');
        assert.equal(start.line, end.line);
        assert.equal(source[start.line].slice(start.character, end.character), 'join');
        console.log('Packaged std.text.join definition resolved in ' + toolchains[0]);
    } finally {
        await client.stop();
        fs.rmSync(workspace, { recursive: true, force: true });
    }
}

checkInstalledExtension(path.resolve(process.argv[2])).catch(error => {
    console.error(error);
    process.exitCode = 1;
});
