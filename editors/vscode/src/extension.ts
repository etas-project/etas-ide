import * as fs from 'fs';
import * as path from 'path';
import { ExtensionContext, workspace, window, commands } from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions, TransportKind } from 'vscode-languageclient/lib/node/main';

console.log('etas extension module loaded');

let client: LanguageClient;

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

export function activate(context: ExtensionContext) {
    console.log('etas extension activate() called');
    try {
        const config = workspace.getConfiguration('etas.lsp');
        const configPath: string = config.get('path', 'etas-lsp');
        const serverPath = resolveServerPath(context, configPath);
        console.log('etas: using server path:', serverPath);

        const serverOptions: ServerOptions = {
            command: serverPath,
            args: [],
            transport: TransportKind.stdio,
        };

        const clientOptions: LanguageClientOptions = {
            documentSelector: [{ scheme: 'file', language: 'etas' }],
            synchronize: {
                fileEvents: workspace.createFileSystemWatcher('**/*.es'),
            },
        };

        client = new LanguageClient(
            'etas-lsp',
            'Etas Language Server',
            serverOptions,
            clientOptions,
        );

        context.subscriptions.push({
            dispose: () => { client.stop(); },
        });

        client.start().then(() => {
            console.log('etas: language server started');
        }).catch((err: Error) => {
            console.error('etas: failed to start language server:', err.message);
        });

        context.subscriptions.push(
            commands.registerCommand('etas.restartServer', async () => {
                await client.restart();
                window.showInformationMessage('Etas language server restarted');
            }),
        );

        console.log('etas: extension activated successfully');
    } catch (err) {
        console.error('etas: activation error:', err);
    }
}

export function deactivate(): Thenable<void> | undefined {
    console.log('etas: deactivate() called');
    return client?.stop();
}
