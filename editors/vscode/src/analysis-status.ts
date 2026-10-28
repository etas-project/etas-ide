export interface AnalysisStatus {
    sequence: number;
    phase: 'idle' | 'queued' | 'preparing' | 'checking' | 'ready' | 'failed';
    projectRoot: string | null;
    elapsedMs: number | null;
    files: number;
    errors: number;
    warnings: number;
    message: string | null;
}

export interface BuildInfo {
    server: string;
    frontend: { version: string; revision: string };
    core: { version: string; revision: string };
    driver: { version: string; revision: string };
}

export const idleStatus: AnalysisStatus = {
    sequence: 0, phase: 'idle', projectRoot: null, elapsedMs: null, files: 0,
    errors: 0, warnings: 0, message: null,
};

export function duration(ms: number): string {
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

export function statusView(status: AnalysisStatus, connection: 'starting' | 'running' | 'stopped', build: BuildInfo | undefined, extensionVersion: string, serverPath: string) {
    const labels = {
        idle: 'Waiting for analysis', queued: 'Waiting for edits', preparing: 'Loading package',
        checking: 'Analyzing', ready: 'Analysis complete', failed: 'Analysis unavailable',
    };
    let text: string;
    if (connection === 'starting') text = '$(sync~spin) Etas: Starting';
    else if (connection === 'stopped') text = '$(error) Etas: Stopped';
    else if (status.phase === 'queued') text = '$(clock) Etas: Waiting';
    else if (status.phase === 'preparing') text = '$(sync~spin) Etas: Loading';
    else if (status.phase === 'checking') text = '$(sync~spin) Etas: Analyzing';
    else if (status.phase === 'failed') text = '$(error) Etas: Unavailable';
    else if (status.errors) text = `$(error) Etas: ${status.errors}`;
    else if (status.warnings) text = `$(warning) Etas: ${status.warnings}`;
    else if (status.phase === 'ready') text = `$(check) Etas${status.elapsedMs === null ? '' : ': ' + duration(status.elapsedMs)}`;
    else text = '$(code) Etas';
    const details = [
        connection === 'running' ? labels[status.phase] : `Language server ${connection}`,
        `Project: ${status.projectRoot ?? 'No active project'}`,
        `Source files: ${status.files}`,
        `Errors: ${status.errors} | Warnings: ${status.warnings}`,
        `Last analysis: ${status.elapsedMs === null ? 'Not available' : duration(status.elapsedMs)}`,
        `Extension: ${extensionVersion}`,
        `Language server: ${build?.server ?? 'Not reported'}`,
        `Frontend: ${build ? build.frontend.version + ' @ ' + build.frontend.revision.slice(0, 8) : 'Not reported'}`,
        `Core/std: ${build ? build.core.version + ' @ ' + build.core.revision.slice(0, 8) : 'Not reported'}`,
        `Driver: ${build ? build.driver.version + ' @ ' + build.driver.revision.slice(0, 8) : 'Not reported'}`,
        `Server: ${serverPath}`,
    ];
    if (status.message) details.push(status.message);
    return { text, tooltip: details.join('\n'), failed: connection === 'stopped' || status.phase === 'failed' };
}
