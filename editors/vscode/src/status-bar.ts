import { commands, Disposable, ExtensionContext, StatusBarAlignment, ThemeColor, window, OutputChannel } from 'vscode';
import { LanguageClient, State } from 'vscode-languageclient/lib/node/main';
import { AnalysisStatus, BuildInfo, idleStatus, statusView } from './analysis-status';

export class AnalysisStatusBar implements Disposable {
    private readonly item = window.createStatusBarItem('etas.analysis', StatusBarAlignment.Right, 95);
    private readonly subscriptions: Disposable[] = [];
    private status = { ...idleStatus };
    private connection: 'starting' | 'running' | 'stopped' = 'starting';
    private build: BuildInfo | undefined;
    private visible = false;
    private generation = 0;
    private initialSyncComplete = false;

    constructor(private readonly context: ExtensionContext, private readonly output: OutputChannel, private readonly serverPath: string) {
        this.item.name = 'Etas Analysis';
        this.item.command = 'etas.showStatus';
        this.subscriptions.push(window.onDidChangeActiveTextEditor(() => this.render()));
        this.subscriptions.push(commands.registerCommand('etas.showStatus', async () => {
            const selected = await window.showQuickPick([
                { label: '$(output) Open Language Server Log', action: 'log', detail: this.item.tooltip as string },
                { label: '$(sync) Restart Language Server', action: 'restart' },
            ], { title: 'Etas Analysis' });
            if (selected?.action === 'log') {
                this.output.appendLine(this.item.tooltip as string);
                this.output.show(true);
            } else if (selected?.action === 'restart') {
                await commands.executeCommand('etas.restartServer');
            }
        }));
        this.render();
    }

    attach(client: LanguageClient) {
        this.subscriptions.push(client.onNotification('etas/analysisStatus', (status: AnalysisStatus) => {
            this.update(status);
        }));
        this.subscriptions.push(client.onDidChangeState(event => {
            if (event.newState === State.Starting) {
                this.generation++;
                this.connection = 'starting';
                this.status = { ...idleStatus };
                this.build = undefined;
                this.initialSyncComplete = false;
            } else if (event.newState === State.Stopped) {
                this.generation++;
                this.connection = 'stopped';
            } else {
                this.connection = 'running';
                this.build = client.initializeResult?.capabilities.experimental?.etasBuild;
                this.output.appendLine(`Etas build: ${JSON.stringify(this.build ?? 'not reported')}`);
                if (client.initializeResult?.capabilities.experimental?.etasAnalysisStatus) {
                    const generation = this.generation;
                    // This request is served from the protocol-side status cache,
                    // so it does not queue behind an in-flight compiler query.
                    void client.sendRequest<AnalysisStatus>('etas/analysisStatus').then(status => {
                        if (this.generation === generation) {
                            this.initialSyncComplete = true;
                            this.update(status);
                        }
                    }, error => this.output.appendLine(`Analysis status unavailable: ${String(error)}`));
                }
            }
            this.render();
        }));
    }

    fail(error: unknown) {
        this.connection = 'stopped';
        this.status = { ...idleStatus, phase: 'failed', message: String(error) };
        this.render();
    }

    snapshot() {
        return { ...this.status, build: this.build, visible: this.visible, text: this.item.text, tooltip: this.item.tooltip, connection: this.connection, initialSyncComplete: this.initialSyncComplete };
    }

    private update(status: AnalysisStatus) {
        if (status.sequence < this.status.sequence) return;
        this.status = status;
        this.render();
    }

    private render() {
        const view = statusView(this.status, this.connection, this.build, this.context.extension.packageJSON.version, this.serverPath);
        this.item.text = view.text;
        this.item.tooltip = view.tooltip;
        this.item.accessibilityInformation = { label: view.tooltip };
        this.item.backgroundColor = view.failed ? new ThemeColor('statusBarItem.errorBackground') : undefined;
        this.visible = window.activeTextEditor?.document.languageId === 'etas';
        if (this.visible) this.item.show(); else this.item.hide();
    }

    dispose() {
        this.item.dispose();
        for (const disposable of this.subscriptions) disposable.dispose();
    }
}
