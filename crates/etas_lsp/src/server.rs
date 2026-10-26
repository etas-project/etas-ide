use std::collections::HashMap;

use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use tokio::sync::{mpsc, oneshot};

use etas_core::SourceId;
use etas_intel::{Command, Event, LspPosition, LspRange, LspTextEdit};

use crate::conversions;

pub struct Backend {
    client: Client,
    service_tx: mpsc::UnboundedSender<Command>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let (service_tx, mut event_rx) = etas_intel::spawn_service_with_events();

        let pump_client = client.clone();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                match event {
                    Event::DiagnosticsUpdated {
                        diagnostics,
                        source_texts,
                        versions,
                    } => {
                        publish_diagnostics(&pump_client, &diagnostics, &source_texts, &versions)
                            .await;
                        let _ = pump_client.semantic_tokens_refresh().await;
                    }
                }
            }
        });

        Self { client, service_tx }
    }
}

async fn publish_diagnostics(
    client: &Client,
    diagnostics_map: &HashMap<String, Vec<etas_intel::IntelDiagnostic>>,
    source_texts: &HashMap<SourceId, String>,
    versions: &HashMap<String, i32>,
) {
    // Publish for all known URIs (including those with no diagnostics)
    for (uri_str, intel_diags) in diagnostics_map {
        let version = versions.get(uri_str).copied();
        let uri = Url::parse(uri_str).unwrap_or_else(|_| Url::parse("file:///unknown.es").unwrap());

        if intel_diags.is_empty() {
            // Clear diagnostics for this URI
            client.publish_diagnostics(uri, vec![], version).await;
            continue;
        }

        let lsp_diags: Vec<Diagnostic> = intel_diags
            .iter()
            .map(|d| {
                let text = source_texts
                    .get(&d.source_id)
                    .map(String::as_str)
                    .unwrap_or("");
                conversions::intel_diagnostic_to_lsp(d, text)
            })
            .collect();

        client.publish_diagnostics(uri, lsp_diags, version).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        let root = params
            .workspace_folders
            .and_then(|folders| folders.into_iter().next())
            .map(|folder| folder.uri)
            .or(params.root_uri);
        if let Some(root_uri) = root {
            if let Ok(path) = root_uri.to_file_path() {
                let _ = self.service_tx.send(Command::SetWorkspaceRoot(path));
            }
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(PositionEncodingKind::UTF16),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(false),
                        })),
                        ..Default::default()
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: None,
                document_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: etas_intel::SEMANTIC_TOKEN_TYPES
                                    .iter()
                                    .map(|name| SemanticTokenType::new(name))
                                    .collect(),
                                token_modifiers: vec![SemanticTokenModifier::DECLARATION],
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..Default::default()
                        },
                    ),
                ),
                code_action_provider: None,
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "etas-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        tracing::info!("etas-lsp initialized");
    }

    async fn shutdown(&self) -> LspResult<()> {
        tracing::info!("etas-lsp shutting down");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let _ = self.service_tx.send(Command::OpenDocument {
            uri: params.text_document.uri.to_string(),
            text: params.text_document.text,
            version: params.text_document.version,
        });
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let content_changes: Vec<LspTextEdit> = params
            .content_changes
            .into_iter()
            .map(|c| LspTextEdit {
                range: c.range.map(|r| LspRange {
                    start: LspPosition {
                        line: r.start.line,
                        character: r.start.character,
                    },
                    end: LspPosition {
                        line: r.end.line,
                        character: r.end.character,
                    },
                }),
                text: c.text,
            })
            .collect();

        let _ = self.service_tx.send(Command::ChangeDocument {
            uri: params.text_document.uri.to_string(),
            content_changes,
            version: params.text_document.version,
        });
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let _ = self.service_tx.send(Command::CloseDocument {
            uri: params.text_document.uri.to_string(),
        });
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let pos = params.text_document_position_params.position;

        let (tx, rx) = oneshot::channel();
        let _ = self.service_tx.send(Command::Hover {
            uri,
            position: LspPosition {
                line: pos.line,
                character: pos.character,
            },
            respond: tx,
        });

        let result = rx.await.unwrap_or(None);
        Ok(result.map(conversions::intel_hover_to_lsp))
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let pos = params.text_document_position.position;

        let (tx, rx) = oneshot::channel();
        let _ = self.service_tx.send(Command::Completion {
            params: etas_intel::CompletionParams {
                uri,
                position: LspPosition {
                    line: pos.line,
                    character: pos.character,
                },
            },
            respond: tx,
        });

        let items = rx
            .await
            .unwrap_or_default()
            .into_iter()
            .map(conversions::intel_completion_to_lsp)
            .collect();

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let pos = params.text_document_position_params.position;
        tracing::info!(
            "goto_definition uri={uri} line={} col={}",
            pos.line,
            pos.character
        );

        let (tx, rx) = oneshot::channel();
        let _ = self.service_tx.send(Command::GoToDefinition {
            uri,
            position: LspPosition {
                line: pos.line,
                character: pos.character,
            },
            respond: tx,
        });

        let locations: Vec<Location> = rx
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|loc| conversions::intel_location_to_lsp(&loc))
            .collect();

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(GotoDefinitionResponse::Array(locations)))
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for change in params.changes {
            if let Ok(path) = change.uri.to_file_path() {
                if matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("etas.toml" | "etas.lock" | "package-index.json")
                ) {
                    let _ = self.service_tx.send(Command::ReloadPackage);
                    continue;
                }
                let path_str = path.to_string_lossy().to_string();
                match change.typ {
                    FileChangeType::CREATED => {
                        let (tx, rx) = oneshot::channel();
                        let _ = self.service_tx.send(Command::AddSourceFile {
                            path: path_str,
                            respond: tx,
                        });
                        let _ = rx.await;
                    }
                    FileChangeType::DELETED => {
                        let (tx, rx) = oneshot::channel();
                        let _ = self.service_tx.send(Command::RemoveSourceFile {
                            path: path_str,
                            respond: tx,
                        });
                        let _ = rx.await;
                    }
                    _ => {}
                }
            }
        }
    }

    async fn did_save(&self, _params: DidSaveTextDocumentParams) {
        let _ = self.service_tx.send(Command::DebouncedCheck);
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> LspResult<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri.to_string();
        let (tx, rx) = oneshot::channel();
        let _ = self
            .service_tx
            .send(Command::SemanticTokens { uri, respond: tx });
        let (tokens, text) = rx.await.unwrap_or_default();
        let data = conversions::intel_tokens_to_lsp(&tokens, &text);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> LspResult<Option<DocumentSymbolResponse>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.service_tx.send(Command::DocumentSymbols {
            uri: params.text_document.uri.to_string(),
            respond: tx,
        });
        let (symbols, text) = rx.await.unwrap_or_default();
        Ok(Some(DocumentSymbolResponse::Nested(
            symbols
                .iter()
                .map(|symbol| conversions::intel_symbol_to_lsp(symbol, &text))
                .collect(),
        )))
    }
}
