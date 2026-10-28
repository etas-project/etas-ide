import * as fs from 'fs';
import * as path from 'path';
import { ExtensionContext, workspace, window, commands } from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions, TransportKind } from 'vscode-languageclient/lib/node/main';
import { AnalysisStatusBar } from './status-bar';

let client: LanguageClient | undefined;

function resolveServerPath(context: ExtensionContext, configPath: string): string {
    // 1. Explicit user config
    if (configPath && configPath !== 'etas-lsp') {
        return configPath;
    }
    // 2. Bundled binary
    const bundled = context.asAbsolutePath(path.join('server', 'etas-lsp'));
    if (fs.existsSync(bundled)) {
        console.log('etas: using bundled server:', bundled);
        return bundled;
    }
    // 3. Fallback to PATH
    console.log('etas: bundled server not found, using PATH');
    return 'etas-lsp';
}

export async function activate(context: ExtensionContext) {
    const output = window.createOutputChannel('Etas Language Server');
    context.subscriptions.push(output);
    let status: AnalysisStatusBar | undefined;
    try {
        const config = workspace.getConfiguration('etas.lsp');
        const configPath: string = config.get('path', 'etas-lsp');
        const serverPath = resolveServerPath(context, configPath);
        status = new AnalysisStatusBar(context, output, serverPath);
        context.subscriptions.push(status);
        console.log('etas: using server path:', serverPath);

        const serverOptions: ServerOptions = {
            command: serverPath,
            args: [],
            transport: TransportKind.stdio,
        };

        const clientOptions: LanguageClientOptions = {
            documentSelector: [{ scheme: 'file', language: 'etas' }, { scheme: 'untitled', language: 'etas' }],
            outputChannel: output,
            synchronize: {
                fileEvents: [
                    workspace.createFileSystemWatcher('**/*.es'),
                    workspace.createFileSystemWatcher('**/{etas.toml,etas.lock,.etas/package-index.json}'),
                ],
            },
        };

        client = new LanguageClient(
            'etas-lsp',
            'Etas Language Server',
            serverOptions,
            clientOptions,
        );
        status.attach(client);

        context.subscriptions.push(
            commands.registerCommand('etas.restartServer', async () => {
                await client?.restart();
                window.showInformationMessage('Etas language server restarted');
            }),
        );

        await client.start();
        output.appendLine(`Etas language server started: ${serverPath}`);
        return { analysisStatus: () => status?.snapshot() };
    } catch (err) {
        status?.fail(err);
        output.appendLine(`Etas language server failed: ${String(err)}`);
        void window.showErrorMessage(`Etas language server failed to start: ${String(err)}`);
        throw err;
    }
}

export function deactivate(): Thenable<void> | undefined {
    console.log('etas: deactivate() called');
    return client?.stop();
}
