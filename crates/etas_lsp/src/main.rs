use tower_lsp::{LspService, Server};

use std::{env, fs::OpenOptions, path::PathBuf, process, sync::Mutex};

mod conversions;
mod server;

fn init_tracing() {
    let log_path = env::var_os("ETAS_LSP_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join(format!("etas-lsp-{}.log", process::id())));
    let Ok(file) = OpenOptions::new().create(true).append(true).open(log_path) else {
        return;
    };

    let _ = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_target(false)
        .try_init();
}

#[tokio::main]
async fn main() {
    init_tracing();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::build(server::Backend::new)
        .custom_method("etas/analysisStatus", server::Backend::analysis_status)
        .finish();

    Server::new(stdin, stdout, socket).serve(service).await;
}
