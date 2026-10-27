const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vscode = require('vscode');

async function eventually(query, accept, label) {
    const deadline = Date.now() + 120000;
    while (Date.now() < deadline) {
        const result = await query();
        if (accept(result)) return result;
        await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error('VS Code timed out: ' + label);
}

async function run() {
    if (process.env.ETAS_LSP_BIN) {
        await vscode.workspace.getConfiguration('etas.lsp').update('path', process.env.ETAS_LSP_BIN, vscode.ConfigurationTarget.Global);
    }
    const extension = vscode.extensions.getExtension('etas-project.etas');
    assert.ok(extension, 'Etas extension must be loaded by VS Code');
    await extension.activate();
    const project = process.env.ETAS_E2E_PROJECT;
    assert.ok(project, 'ETAS_E2E_PROJECT must select a real project');
    let checked = 0;
    let hoversChecked = 0;
    for (const name of fs.readdirSync(path.join(project, 'src'), { recursive: true })) {
        if (!name.endsWith('.es')) continue;
        const uri = vscode.Uri.file(path.join(project, 'src', name));
        const doc = await vscode.workspace.openTextDocument(uri);
        assert.equal(doc.languageId, 'etas', name);
        const symbols = await eventually(
            () => vscode.commands.executeCommand('vscode.executeDocumentSymbolProvider', uri),
            result => Array.isArray(result) && result.length > 0, 'outline: ' + name);
        assert.ok(symbols.some(symbol => symbol.name.startsWith('nomo')), name);
        const legend = await vscode.commands.executeCommand('vscode.provideDocumentSemanticTokensLegend', uri);
        const tokens = await eventually(
            () => vscode.commands.executeCommand('vscode.provideDocumentSemanticTokens', uri),
            result => result && result.data.length > 0, 'tokens: ' + name);
        assert.ok(legend.tokenTypes.includes('keyword'));
        assert.equal(tokens.data.length % 5, 0);
        if (name.replaceAll(path.sep, '/') === 'nomo/main.es') {
            await eventually(
                async () => {
                    const ready = await vscode.commands.executeCommand('vscode.provideDocumentSemanticTokens', uri);
                    if (!ready) return false;
                    let line = 0, col = 0;
                    for (let i = 0; i < ready.data.length; i += 5) {
                        line += ready.data[i];
                        col = ready.data[i] ? ready.data[i + 1] : col + ready.data[i + 1];
                        if (doc.getText(new vscode.Range(line, col, line, col + ready.data[i + 2])) === 'resume'
                            && legend.tokenTypes[ready.data[i + 3]] === 'function') return true;
                    }
                    return false;
                }, result => result, 'checked resume name after initial semantic refresh');
        }
        if (name.replaceAll(path.sep, '/') === 'nomo/cli/edit_args.es') {
            let line = 0, col = 0, count = 0;
            for (let i = 0; i < tokens.data.length; i += 5) {
                line += tokens.data[i];
                col = tokens.data[i] ? tokens.data[i + 1] : col + tokens.data[i + 1];
                const word = doc.getText(new vscode.Range(line, col, line, col + tokens.data[i + 2]));
                if (word !== 'count') continue;
                assert.equal(legend.tokenTypes[tokens.data[i + 3]], 'function', 'count must keep its imported callable classification');
                count++;
            }
            assert.ok(count >= 3, 'the real edit_args.es alias and calls must reach the provider');
        }
        if (name.replaceAll(path.sep, '/') === 'nomo/cli/harness_checkpoint_args.es') {
            const offset = doc.getText().indexOf('Identity { id = id, workspace = workspace');
            assert.ok(offset >= 0, 'the reported record constructor must be present');
            const position = doc.positionAt(offset);
            let line = 0, col = 0, matched = false;
            for (let i = 0; i < tokens.data.length; i += 5) {
                line += tokens.data[i];
                col = tokens.data[i] ? tokens.data[i + 1] : col + tokens.data[i + 1];
                if (line !== position.line || col !== position.character) continue;
                assert.equal(legend.tokenTypes[tokens.data[i + 3]], 'type');
                assert.equal(doc.getText(new vscode.Range(line, col, line, col + tokens.data[i + 2])), 'Identity');
                matched = true;
            }
            assert.ok(matched, 'the real Identity constructor must reach the semantic provider');
            const hovers = await vscode.commands.executeCommand('vscode.executeHoverProvider', uri, position);
            assert.ok(hovers.some(hover => hover.contents.some(content => content.value?.includes('type Identity'))));
        }
        const flow = /\bflow\s+([A-Za-z_]\w*)/.exec(doc.getText());
        if (flow) {
            const position = doc.positionAt(flow.index + flow[0].lastIndexOf(flow[1]));
            const hovers = await eventually(
                () => vscode.commands.executeCommand('vscode.executeHoverProvider', uri, position),
                result => Array.isArray(result) && result.some(hover => hover.contents.some(content =>
                    content.value?.includes('flow ' + flow[1]))), 'hover: ' + name);
            for (const hover of hovers) {
                for (const content of hover.contents) assert.doesNotMatch(content.value, /TypeId\(|SymbolId\(|FlowType|Function\(/, name);
            }
            hoversChecked++;
        }
        checked++;
    }
    assert.ok(checked > 0);
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'etas-vscode-e2e-'));
    try {
        const uri = vscode.Uri.file(path.join(root, 'unicode.es'));
        const valid = 'module unicode;\nflow square(x: i32) -> i32 ![] { return x * x; }\nflow main() -> i32 { let text = "中文😀"; let answer = square(4); return 0; }\n';
        fs.writeFileSync(uri.fsPath, valid);
        const doc = await vscode.workspace.openTextDocument(uri);
        await vscode.window.showTextDocument(doc);
        const edit = new vscode.WorkspaceEdit();
        edit.replace(uri, new vscode.Range(0, 0, doc.lineCount, 0), valid.replace('return 0;', 'let broken = ; return 0;'));
        assert.equal(await vscode.workspace.applyEdit(edit), true);
        const diagnostics = await eventually(
            () => vscode.languages.getDiagnostics(uri),
            result => result.some(diag => String(diag.code).includes('Syntax')), 'parse diagnostics');
        assert.ok(diagnostics.some(diag => diag.source === 'etas'));
        const repaired = new vscode.WorkspaceEdit();
        repaired.replace(uri, new vscode.Range(0, 0, doc.lineCount, 0), valid);
        assert.equal(await vscode.workspace.applyEdit(repaired), true);
        await eventually(
            () => vscode.languages.getDiagnostics(uri),
            result => !result.some(diag => diag.severity === vscode.DiagnosticSeverity.Error), 'diagnostic recovery');
        const tokens = await vscode.commands.executeCommand('vscode.provideDocumentSemanticTokens', uri);
        const legend = await vscode.commands.executeCommand('vscode.provideDocumentSemanticTokensLegend', uri);
        let line = 0, col = 0, sawReturn = false;
        for (let i = 0; i < tokens.data.length; i += 5) {
            line += tokens.data[i];
            col = tokens.data[i] ? tokens.data[i + 1] : col + tokens.data[i + 1];
            if (legend.tokenTypes[tokens.data[i + 3]] === 'keyword'
                && doc.getText(new vscode.Range(line, col, line, col + tokens.data[i + 2])) === 'return') {
                assert.equal(col, valid.split('\n')[line].indexOf('return'));
                sawReturn = true;
            }
        }
        assert.ok(sawReturn, 'UTF-16 return token must reach the VS Code provider');
        const hovers = await eventually(
            () => vscode.commands.executeCommand('vscode.executeHoverProvider', uri, doc.positionAt(valid.indexOf('square(4)'))),
            result => Array.isArray(result) && result.some(hover => hover.contents.some(content =>
                typeof content.value === 'string' && content.value.includes('flow square(x: i32) -> i32 ![]'))), 'readable hover');
        const hover = hovers.find(hover => hover.contents.some(content => content.value?.includes('flow square')));
        assert.equal(doc.getText(hover.range), 'square');
        for (const content of hover.contents) assert.doesNotMatch(content.value, /TypeId\(|SymbolId\(|FlowType|Function\(/);
        await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
    console.log('VS Code E2E passed: ' + checked + ' nomo files, ' + hoversChecked + ' checked hovers, outline, semantic providers, parse errors and recovery');
}

exports.run = async function () {
    try {
        await run();
        if (process.env.ETAS_E2E_RESULT) fs.writeFileSync(process.env.ETAS_E2E_RESULT, JSON.stringify({ passed: true }));
    } catch (error) {
        if (process.env.ETAS_E2E_RESULT) fs.writeFileSync(process.env.ETAS_E2E_RESULT, JSON.stringify({ passed: false, error: error.stack || String(error) }));
        throw error;
    }
};
