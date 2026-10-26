const { spawn } = require('node:child_process');
const path = require('node:path');

class LspClient {
    constructor() {
        const binary = process.env.ETAS_LSP_BIN || path.resolve(__dirname, '../../../target/debug/etas-lsp');
        this.process = spawn(binary, [], { stdio: ['pipe', 'pipe', 'pipe'] });
        this.pending = new Map();
        this.notifications = [];
        this.nextId = 1;
        this.buffer = Buffer.alloc(0);
        this.stderr = '';
        this.process.stderr.on('data', data => { this.stderr += data; });
        this.process.stdout.on('data', chunk => this.receive(chunk));
        this.process.on('error', error => this.fail(error));
        this.process.on('exit', code => this.fail(new Error('LSP exited ' + code + ': ' + this.stderr)));
    }
    fail(error) {
        this.failure = error;
        for (const request of this.pending.values()) { clearTimeout(request.timer); request.reject(error); }
        this.pending.clear();
    }
    send(message) {
        const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }));
        this.process.stdin.write('Content-Length: ' + body.length + '\r\n\r\n');
        this.process.stdin.write(body);
    }
    notify(method, params) { this.send({ method, params }); }
    request(method, params) {
        if (this.failure) return Promise.reject(this.failure);
        const id = this.nextId++;
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                this.pending.delete(id);
                reject(new Error('LSP timeout: ' + method + '; stderr: ' + this.stderr));
            }, 120000);
            this.pending.set(id, { resolve, reject, timer });
            this.send({ id, method, params });
        });
    }
    receive(chunk) {
        this.buffer = Buffer.concat([this.buffer, chunk]);
        while (true) {
            const headerEnd = this.buffer.indexOf('\r\n\r\n');
            if (headerEnd < 0) return;
            const match = /Content-Length: (\d+)/i.exec(this.buffer.subarray(0, headerEnd).toString());
            if (!match) throw new Error('invalid LSP frame');
            const end = headerEnd + 4 + Number(match[1]);
            if (this.buffer.length < end) return;
            const message = JSON.parse(this.buffer.subarray(headerEnd + 4, end));
            this.buffer = this.buffer.subarray(end);
            if (message.method && message.id !== undefined) {
                this.send({ id: message.id, result: null });
            } else if (message.id !== undefined) {
                const request = this.pending.get(message.id);
                if (!request) continue;
                clearTimeout(request.timer);
                this.pending.delete(message.id);
                if (message.error) request.reject(new Error(JSON.stringify(message.error)));
                else request.resolve(message.result);
            } else {
                this.notifications.push(message);
            }
        }
    }
    async initialize(rootUri = null) {
        const result = await this.request('initialize', {
            processId: process.pid, rootUri,
            capabilities: { workspace: { semanticTokens: { refreshSupport: true } } },
        });
        this.notify('initialized', {});
        return result;
    }
    async diagnostics(uri, version) {
        const end = Date.now() + 120000;
        while (Date.now() < end) {
            if (this.failure) throw this.failure;
            const result = this.notifications.findLast(message => message.method === 'textDocument/publishDiagnostics'
                && message.params.uri === uri && message.params.version === version);
            if (result) return result.params.diagnostics;
            await new Promise(resolve => setTimeout(resolve, 25));
        }
        throw new Error('diagnostics timeout: ' + uri + ', v' + version);
    }
    async stop() {
        if (this.failure) return;
        try {
            await this.request('shutdown');
            this.notify('exit');
        } finally {
            this.process.stdin.end();
            this.process.kill();
        }
    }
}
module.exports = { LspClient };
